/**
 * Test fixtures that spin up a real `kx serve` gateway (ports the Python
 * `conftest.py`). The contract tests drive the SDK against an actual embedded
 * single-system runtime (the FFI-free `kx` binary) and compare results to the
 * `kx` CLI, byte-for-byte. The `kx` binary is located via `KX_BIN`, then
 * `target/release/kx`, then `target/debug/kx`; if none exist it is built FFI-free.
 */

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { KxClient } from "../../src/node.js";

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(HERE, "../../../..");
export const ECHO_HANDLE = "kx/recipes/echo";

let cachedBin: string | null = null;

export function findOrBuildKx(): string {
  if (cachedBin) return cachedBin;
  const env = process.env.KX_BIN;
  if (env && existsSync(env)) {
    cachedBin = env;
    return env;
  }
  // NOTE: a pre-existing binary is used as-is. The dataset contract tests need a
  // binary built `--features hnsw`; if you have a stale non-hnsw `target/release/kx`,
  // those tests fail with UNIMPLEMENTED — `rm` it (or set KX_BIN to an hnsw build).
  // CI builds fresh with the feature, so it always exercises the data-plane.
  for (const rel of ["target/release/kx", "target/debug/kx"]) {
    const cand = path.join(REPO_ROOT, rel);
    if (existsSync(cand)) {
      cachedBin = cand;
      return cand;
    }
  }
  // Build it FFI-free (no C++ toolchain needed). `--features hnsw` adds the Datasets
  // data-plane (RAG) — still pure-Rust (kx-dataset-hnsw + rusqlite, no llama.cpp) — so
  // the contract tests can exercise the client-vector ingest/query path.
  execFileSync("cargo", ["build", "--release", "-p", "kx-cli", "--features", "hnsw"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
  cachedBin = path.join(REPO_ROOT, "target/release/kx");
  return cachedBin;
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/** A running `kx serve` under test. */
export class Server {
  constructor(
    readonly endpoint: string,
    readonly wsEndpoint: string,
    readonly proc: ChildProcess,
    public token?: string,
  ) {}

  stop(): void {
    this.proc.kill("SIGTERM");
  }
}

/** Poll the gateway with a unary RPC until it answers (any gRPC status = ready). */
async function waitReady(endpoint: string, proc: ChildProcess, timeoutMs = 40_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  const probe = new KxClient(endpoint);
  try {
    while (Date.now() < deadline) {
      if (proc.exitCode !== null) {
        throw new Error(`kx serve exited early (code ${proc.exitCode})`);
      }
      try {
        await probe.listSignatures();
        return; // resolved → ready
      } catch (e) {
        const code = (e as { code?: string }).code;
        // unavailable / connect = not up yet; anything else (unimplemented,
        // unauthenticated, …) is a real gRPC response, so the server IS ready.
        if (code === "unavailable" || code === "connect") {
          await sleep(100);
          continue;
        }
        return;
      }
    }
    throw new Error("kx serve did not become ready in time");
  } finally {
    probe.close();
  }
}

const SERVERS: Server[] = [];

/** Spawn a gateway with the given extra flags; tracked for {@link stopAllServers}. */
export async function spawnServer(...extra: string[]): Promise<Server> {
  const kxBin = findOrBuildKx();
  const [port, wsPort] = await Promise.all([freePort(), freePort()]);
  const tmp = await mkdtemp(path.join(tmpdir(), "kxsrv-"));
  const endpoint = `http://127.0.0.1:${port}`;
  const wsEndpoint = `ws://127.0.0.1:${wsPort}`;
  const proc = spawn(
    kxBin,
    [
      "serve",
      "--journal",
      path.join(tmp, "kx.db"),
      "--content",
      path.join(tmp, "blobs"),
      "--listen",
      `127.0.0.1:${port}`,
      "--ws-listen",
      `127.0.0.1:${wsPort}`,
      // A console-enabled binary would otherwise bind the DEFAULT :8888 —
      // concurrent test servers then collide (a console-less binary accepts
      // the flag as a no-op). Every test spawn disables the console.
      "--no-console",
      ...extra,
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  const server = new Server(endpoint, wsEndpoint, proc);
  SERVERS.push(server);
  await waitReady(endpoint, proc);
  return server;
}

/** A loopback `--dev-allow-local` gateway (no token needed). */
export function devServer(): Promise<Server> {
  return spawnServer("--dev-allow-local");
}

/** A token-authenticated gateway (`--auth-token s3cr3t=alice`). */
export async function authServer(): Promise<Server> {
  const s = await spawnServer("--auth-token", "s3cr3t=alice");
  s.token = "s3cr3t";
  return s;
}

/** Stop + reap every gateway spawned so far (call in an `afterEach`/`afterAll`). */
export async function stopAllServers(): Promise<void> {
  for (const s of SERVERS.splice(0)) {
    s.stop();
  }
  // Give the OS a beat to release the sockets before the next test binds.
  await sleep(50);
}
