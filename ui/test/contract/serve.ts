/**
 * Spin up a real `kx serve` gateway for the contract test (adapted from the SDK's
 * fixtures). The `kx` binary is located via `KX_BIN`, then `target/release/kx`,
 * then `target/debug/kx`; if none exist it is built FFI-free. The contract test
 * uses the Node client (guaranteed under Node); the browser gRPC-web + CORS path is
 * proven separately by the Playwright E2E.
 */

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { KxClient } from "@kortecx/sdk/node";

const HERE = path.dirname(fileURLToPath(import.meta.url));
// ui/test/contract → up 3 = repo root.
export const REPO_ROOT = path.resolve(HERE, "../../..");
export const ECHO_HANDLE = "kx/recipes/echo";
/** The T3.3 deterministic multi-node demo recipe (root → 3 children → gather). */
export const FANOUT_HANDLE = "kx/recipes/passthrough-dag";

let cachedBin: string | null = null;

export function findOrBuildKx(): string {
  if (cachedBin) {
    return cachedBin;
  }
  const env = process.env.KX_BIN;
  if (env && existsSync(env)) {
    cachedBin = env;
    return env;
  }
  for (const rel of ["target/release/kx", "target/debug/kx"]) {
    const cand = path.join(REPO_ROOT, rel);
    if (existsSync(cand)) {
      cachedBin = cand;
      return cand;
    }
  }
  execFileSync("cargo", ["build", "--release", "-p", "kx-cli"], {
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

export class Server {
  constructor(
    readonly endpoint: string,
    readonly proc: ChildProcess,
    public token?: string,
  ) {}

  stop(): void {
    this.proc.kill("SIGTERM");
  }
}

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
        return;
      } catch (e) {
        const code = (e as { code?: string }).code;
        if (code === "unavailable" || code === "connect") {
          await sleep(100);
        } else {
          return; // any real gRPC answer means the server is up
        }
      }
    }
    throw new Error("kx serve did not become ready in time");
  } finally {
    probe.close();
  }
}

const SERVERS: Server[] = [];

export async function spawnServer(...extra: string[]): Promise<Server> {
  const kxBin = findOrBuildKx();
  const [port, wsPort] = await Promise.all([freePort(), freePort()]);
  const tmp = await mkdtemp(path.join(tmpdir(), "kxui-"));
  const endpoint = `http://127.0.0.1:${port}`;
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
      // A console-feature kx defaults its web console onto the well-known
      // 8888 — disable it here so parallel contract servers never collide
      // (a no-op on console-less builds; same as the e2e fixture).
      "--no-console",
      ...extra,
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  const server = new Server(endpoint, proc);
  SERVERS.push(server);
  await waitReady(endpoint, proc);
  return server;
}

export function devServer(): Promise<Server> {
  return spawnServer("--dev-allow-local");
}

export async function authServer(): Promise<Server> {
  const s = await spawnServer("--auth-token", "s3cr3t=alice");
  s.token = "s3cr3t";
  return s;
}

export async function stopAllServers(): Promise<void> {
  for (const s of SERVERS.splice(0)) {
    s.stop();
  }
  await sleep(50);
}
