/**
 * Zero-config client resolution for Node (V2a g1) — the TS mirror of Python's
 * `kortecx.defaults`.
 *
 * ```ts
 * import { run, flow } from "@kortecx/sdk";
 * const out = await run(flow().agent("Summarize the README"));
 * console.log((out as Result).text);
 * ```
 *
 * A lazily-built, process-wide default client so the simplest path needs no
 * constructor. Config order (first wins):
 *
 * 1. explicit args,
 * 2. environment — `KX_ENDPOINT` / `KX_TOKEN` / `KX_DEFAULT_MODEL`,
 * 3. `~/.kortecx/config.toml` (a `[client]` table),
 * 4. the conventional defaults (loopback endpoint, no token, server-bound model).
 *
 * NODE-ONLY: reads `node:fs`/`node:os` and builds the gRPC {@link KxClient}. The
 * `web` and `chains` entrypoints do NOT import this module (explicit-client by
 * design), so it never reaches a browser bundle. On load it installs the lazy
 * factory the {@link Flow}/{@link Agent} terminals use for a zero-config client.
 */

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Chain } from "./chains.js";
import type { InvokeOptions } from "./client.js";
import {
  getDefaultClient,
  setDefaultClient as registrySet,
  setDefaultClientFactory,
} from "./default-client.js";
import { KxUsage } from "./errors.js";
import { type AgentStepOptions, Flow, flow } from "./flow.js";
import { KxClient } from "./node.js";
import type { Result, Run } from "./run.js";
import { type Args, DEFAULT_ENDPOINT } from "./transport.js";

interface ClientConfig {
  endpoint?: string;
  token?: string;
  default_model?: string;
}

/**
 * Best-effort read of the `[client]` table from `~/.kortecx/config.toml`. A missing
 * file / read error ⇒ `{}` (env + defaults still apply). Never throws. A minimal,
 * dependency-free reader: only `key = "value"` lines inside the `[client]` table are
 * read (the three keys the SDK uses) — not a general TOML parser.
 */
function loadConfig(): ClientConfig {
  let text: string;
  try {
    text = readFileSync(join(homedir(), ".kortecx", "config.toml"), "utf-8");
  } catch {
    return {};
  }
  const out: ClientConfig = {};
  let inClient = false;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    if (line.startsWith("[")) {
      inClient = line === "[client]";
      continue;
    }
    if (!inClient) continue;
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim();
    let val = line.slice(eq + 1).trim();
    if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
      val = val.slice(1, -1);
    }
    if (key === "endpoint") out.endpoint = val;
    else if (key === "token") out.token = val;
    else if (key === "default_model") out.default_model = val;
  }
  return out;
}

function resolveEndpoint(explicit?: string): string {
  return explicit || process.env.KX_ENDPOINT || loadConfig().endpoint || DEFAULT_ENDPOINT;
}

function resolveDefaultModel(explicit?: string): string {
  return explicit || process.env.KX_DEFAULT_MODEL || loadConfig().default_model || "";
}

/** Build a {@link KxClient} from explicit args + env + `~/.kortecx/config.toml` + defaults. */
export function makeClient(
  opts: { endpoint?: string; token?: string; defaultModel?: string } = {},
): KxClient {
  const cfg = loadConfig();
  const token = opts.token ?? process.env.KX_TOKEN ?? cfg.token;
  return new KxClient(resolveEndpoint(opts.endpoint), {
    token,
    defaultModel: resolveDefaultModel(opts.defaultModel),
  });
}

// Install the lazy factory the bundle-safe registry uses so `flow().run()` /
// `agent().run()` / `run(...)` work with no explicit client in Node. The guard
// mirrors Python's `default_client()`: a top-level run inside the `_toolserver`
// re-import context would recurse, so refuse it loudly.
setDefaultClientFactory(() => {
  if (process.env.KX_TOOLSERVE) {
    throw new KxUsage(
      "cannot start a run while serving localTool functions (the _toolserver re-import " +
        "context); guard your run()/invoke() calls so they do not re-execute on import",
    );
  }
  return makeClient();
});

/**
 * The lazily-built, process-wide default Node client used by the module-level
 * {@link run} / {@link invoke} and the {@link Flow} / {@link Agent} terminals.
 *
 * NOTE (concurrency): this is a SINGLE shared client (one transport) — fine for
 * scripts and sync use; for concurrent work construct explicit {@link KxClient}
 * instances rather than relying on this singleton.
 */
export function defaultClient(): KxClient {
  return getDefaultClient() as KxClient;
}

/** Override (or clear, with `null`) the process-wide default client — for tests or a
 * custom endpoint/token without threading a client through every call. */
export function setDefaultClient(client: KxClient | null): void {
  registrySet(client ?? undefined);
}

/** Module-level convenience — run a {@link Flow}, a {@link Chain}, or a bare prompt (a
 * one-line agent) via the default client. `opts` agent fields (`tools` / `reasoning` /
 * …) apply only to the prompt form. */
export function run(
  target: Flow | Chain | string,
  opts: { wait?: boolean; timeoutMs?: number } & AgentStepOptions = {},
): Promise<Run | Result> {
  const kx = defaultClient();
  if (target instanceof Flow) {
    return target.run({ wait: opts.wait, timeoutMs: opts.timeoutMs, client: kx });
  }
  if (typeof target === "string") {
    const { wait, timeoutMs, ...agentOpts } = opts;
    return flow().agent(target, agentOpts).run({ wait, timeoutMs, client: kx });
  }
  return kx.runChain(target, { wait: opts.wait ?? true, timeoutMs: opts.timeoutMs });
}

/** Module-level convenience — invoke a published recipe via the default client. */
export function invoke(handle: string, args: Args, opts?: InvokeOptions): Promise<Run | Result> {
  return defaultClient().invoke(handle, args, opts);
}

/** Module-level convenience — a grounded chat via the default client (the TS mirror of
 * Python's `kx.chat`). Pass `dataset` (+ optional `k`) to auto-RAG the answer over a
 * dataset's retrieved text; returns the answer text. */
export function chat(
  prompt: string,
  opts: { dataset?: string; k?: number; timeoutMs?: number } = {},
): Promise<string> {
  return defaultClient().chat(prompt, opts);
}
