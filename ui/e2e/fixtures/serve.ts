/**
 * Spawn a real `kx serve` for the browser E2E. The gateway is given an explicit
 * `--cors-origin` so the SPA (served at the pinned preview origin) can make gRPC-web
 * calls — proving the real browser CORS + gRPC-web path end to end. Readiness is
 * probed with the Node client (the test browser uses the web client).
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
// ui/e2e/fixtures → up 3 = repo root.
export const REPO_ROOT = path.resolve(HERE, "../../..");

let cachedBin: string | null = null;

function findOrBuildKx(): string {
  if (cachedBin) {
    return cachedBin;
  }
  const env = process.env.KX_BIN;
  if (env && existsSync(env)) {
    cachedBin = env;
    return env;
  }
  // NOTE: a pre-existing binary is used as-is. The datasets e2e needs a binary built
  // `--features hnsw`; a stale non-hnsw `target/release/kx` makes it fail with
  // UNIMPLEMENTED — `rm` it (or set KX_BIN to an hnsw build). CI builds fresh with it.
  for (const rel of ["target/release/kx", "target/debug/kx"]) {
    const cand = path.join(REPO_ROOT, rel);
    if (existsSync(cand)) {
      cachedBin = cand;
      return cand;
    }
  }
  // `--features hnsw` adds the Datasets data-plane (RAG) — still FFI-free (pure-Rust
  // kx-dataset-hnsw + rusqlite, no llama.cpp) — so the e2e can exercise the section.
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
          return;
        }
      }
    }
    throw new Error("kx serve did not become ready in time");
  } finally {
    probe.close();
  }
}

export interface Gateway {
  endpoint: string;
  /** The R5 WS-bridge endpoint (for the Activity live tail). */
  wsEndpoint: string;
  /** The embedded web console origin (only when spawned with `console: true`). */
  consoleOrigin?: string;
  stop(): void;
}

export interface SpawnOpts {
  /** Allowed browser origin (omit to test deny-by-default). */
  corsOrigin?: string;
  /**
   * Serve the embedded web console (D139) on a free loopback port. Needs a
   * `--features console` kx (the CI ui job builds one); everything else passes
   * `--no-console` so a default-on console build can never collide on 8888.
   */
  console?: boolean;
  /**
   * D155: set `KX_SERVE_FS_ROOT` for this spawn — the operator read root that
   * enables `SnapshotInto` (default-OFF). The branch RPCs (CreateBranch / the
   * branches.db store) are always wired; only snapshot's host read is gated.
   */
  fsRoot?: string;
}

export async function spawnGateway(opts: SpawnOpts = {}): Promise<Gateway> {
  const kxBin = findOrBuildKx();
  const [port, wsPort, consolePort] = await Promise.all([freePort(), freePort(), freePort()]);
  const tmp = await mkdtemp(path.join(tmpdir(), "kxe2e-"));
  const endpoint = `http://127.0.0.1:${port}`;
  const args = [
    "serve",
    "--journal",
    path.join(tmp, "kx.db"),
    "--content",
    path.join(tmp, "blobs"),
    "--listen",
    `127.0.0.1:${port}`,
    "--ws-listen",
    `127.0.0.1:${wsPort}`,
    "--dev-allow-local",
  ];
  // `--no-console` parses as a no-op on console-less builds, so it is safe to
  // pass unconditionally — and REQUIRED for console builds (default-on 8888
  // would collide across parallel spawns).
  if (opts.console) {
    args.push("--console-listen", `127.0.0.1:${consolePort}`);
  } else {
    args.push("--no-console");
  }
  if (opts.corsOrigin) {
    args.push("--cors-origin", opts.corsOrigin);
  }
  // Force Ollama OFF so every spawn is model-free + deterministic, MATCHING CI (which
  // has no Ollama daemon). `KX_SERVE_OLLAMA=auto` (the default) would auto-detect a
  // dev's ambient Ollama on :11434 and silently provision a model — flaking any spec
  // that asserts model-free behaviour (e.g. the no-model degrade notice). The
  // model-needing specs opt back in explicitly.
  const env: NodeJS.ProcessEnv = { ...process.env, KX_SERVE_OLLAMA: "off" };
  if (opts.fsRoot) {
    env.KX_SERVE_FS_ROOT = opts.fsRoot;
  }
  const proc = spawn(kxBin, args, { stdio: ["ignore", "pipe", "pipe"], env });
  let stopped = false;
  const stop = () => {
    if (!stopped) {
      stopped = true;
      proc.kill("SIGTERM");
    }
  };
  await waitReady(endpoint, proc);
  return {
    endpoint,
    wsEndpoint: `ws://127.0.0.1:${wsPort}`,
    consoleOrigin: opts.console ? `http://127.0.0.1:${consolePort}` : undefined,
    stop,
  };
}

/** The pinned origin the SPA is served from (must match playwright webServer). */
export const SPA_ORIGIN = "http://localhost:4173";
