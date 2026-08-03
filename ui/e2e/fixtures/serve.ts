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

/**
 * The model a `model: true` spawn serves when the operator names none. Gemma-4 via
 * Ollama — the family the runtime renders a real chat template for. Override with
 * `KX_SERVE_OLLAMA_MODELS`.
 */
const DEFAULT_MODEL = "gemma4:12b";

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
  /**
   * Serve a REAL MODEL for this spawn — the opt-in the comment below used to
   * describe and nothing implemented. Turns `KX_SERVE_OLLAMA` back `on` and names
   * the model, so a console-originated request can reach a served model.
   *
   * REQUIRES a `serve-engine` binary via `KX_MODEL_BIN`, and fails loudly without
   * one. `hnsw` does NOT imply `serve-engine`, so the default e2e binary does not
   * merely lack a model — the model-driven RPCs (`DeriveApp`, `ProposeWorkflow`)
   * are not compiled into it at all and answer `unimplemented`. Falling back to it
   * would turn "no model" into a green test against a gateway that could never
   * have answered.
   *
   * Every spec using this must ALSO `test.skip` itself off by default: CI has no
   * Ollama daemon, and this is opt-in exactly like the `#[ignore]` live tests.
   */
  model?: boolean;
}

/**
 * The `serve-engine` binary used for a `model: true` spawn. Kept OUT of
 * {@link findOrBuildKx} deliberately: that function memoises ONE path for the whole
 * process, so a run mixing model-less and model-served specs would hand the second
 * kind whichever binary the first kind resolved.
 *
 * Never built on demand. A `serve-engine` build is minutes long and the feature set
 * is part of the experiment, so the operator states it rather than inheriting a
 * binary someone else's recipe happened to leave in `target/`.
 */
function modelBin(): string {
  const bin = process.env.KX_MODEL_BIN;
  if (!bin || !existsSync(bin)) {
    const got = bin ? `a path that does not exist: ${bin}` : "no KX_MODEL_BIN";
    throw new Error(
      `spawnGateway({ model: true }) needs KX_MODEL_BIN pointing at a kx built with \`--features console,serve-engine,hnsw,hosted-apps,observability\`. Got ${got}. Refusing to fall back to the model-less binary: it has no serve-engine, so DeriveApp/ProposeWorkflow answer \`unimplemented\` and the spec would pass on nothing.`,
    );
  }
  return bin;
}

export async function spawnGateway(opts: SpawnOpts = {}): Promise<Gateway> {
  const kxBin = opts.model ? modelBin() : findOrBuildKx();
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
  // Ollama is forced OFF unless a spec opts IN, so the default spawn is model-free and
  // deterministic, MATCHING CI (which has no Ollama daemon). `KX_SERVE_OLLAMA=auto` (the
  // real default) would auto-detect a dev's ambient Ollama on :11434 and silently
  // provision a model — flaking any spec that asserts model-free behaviour (e.g. the
  // no-model degrade notice).
  //
  // ⚠ THE LITERAL GOES AFTER THE SPREAD IN BOTH ARMS, so an ambient `KX_SERVE_OLLAMA`
  // can never decide which kind of spawn this is: the spec does, and only the spec.
  //
  // What this replaced, because the correction is the useful part: the opt-in described
  // here did not exist, and the comment saying "the model-needing specs opt back in
  // explicitly" described a mechanism nobody had built. It also over-claimed in the
  // other direction — "all console e2e specs are model-less by construction" is false.
  // `rule41-lineage-gemma.spec.ts` is model-served; it simply does not come through this
  // fixture, pointing instead at a hand-started serve on a fixed port. The accurate
  // statement was always narrower: every spec routed through `spawnGateway` was
  // model-less. That is what `model: true` now changes — and specs using it still opt
  // themselves out of the default suite, so a green CI run remains no evidence at all
  // about model behaviour.
  const env: NodeJS.ProcessEnv = { ...process.env };
  if (opts.model) {
    env.KX_SERVE_OLLAMA = "on";
    env.KX_SERVE_OLLAMA_MODELS = process.env.KX_SERVE_OLLAMA_MODELS ?? DEFAULT_MODEL;
    // ONE ENGINE PER RUN. Exporting a GGUF alongside the Ollama vars makes the GGUF the
    // serve PRIMARY while only the label says Ollama — a run that reports the engine it
    // was not using. Cleared here so an ambient shell cannot produce that. `undefined`
    // is a genuine UNSET for `child_process.spawn` (verified: it wins over an ambient
    // value, rather than reaching the child as the string "undefined").
    env.KX_SERVE_MODEL_GGUF = undefined;
    env.KX_GEMMA_MODEL_DEST = undefined;
  } else {
    env.KX_SERVE_OLLAMA = "off";
  }
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
