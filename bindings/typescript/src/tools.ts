/**
 * Local function tools — `localTool(...)` (Batch V2b), the TS mirror of Python's
 * `@kx.tool`.
 *
 * ```ts
 * import { localTool, flow } from "@kortecx/sdk";
 *
 * const add = localTool({
 *   name: "add",
 *   params: { a: "integer", b: "integer" }, // explicit — TS types are erased at runtime
 *   run: ({ a, b }) => a + b,
 * });
 *
 * await flow().tool(add, { a: 2, b: 2 }).run({ client: kx }); // deterministic, today
 * ```
 *
 * `localTool` exposes a JS function as a real, governed tool: the SDK runs it as a
 * local **stdio MCP server** (`_toolserver`) the runtime DIALS through the existing
 * PR-6b MCP gateway — zero new runtime substrate; the runtime fires it under a
 * server-built warrant (SN-8).
 *
 * **Node + dev-scoped.** The runtime spawns the stdio server subprocess, so this is
 * the Node SDK only (a browser cannot spawn a subprocess) and the tool MODULE must
 * be Node-importable (compiled JS / `.mjs`) and co-located with the serve. Unlike
 * Python (type hints → schema), TS types are erased, so the param schema is explicit.
 *
 * **Firing lanes (GR15-honest):** deterministic `flow().tool(fn, args)` (today) ·
 * steered `new Agent({ tools:[fn], dynamic:true })` → `kx/recipes/react-auto` (today,
 * needs `KX_SERVE_AUTOGRANT=1`) · frozen agentic loop → PR-9b-2 (a clear pre-flight hint).
 */

import { type Chain, type Task, task } from "./chains.js";

/** A local-tool authoring/registration error. */
export class KxToolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "KxToolError";
  }
}

/** A param's declared type — `"string"` / `"integer"` / `"boolean"`, or a string
 *  array (an exact-match enum). Floats are intentionally absent (the runtime does
 *  not type-gate numbers; pass an int). */
export type LocalParamType = "string" | "integer" | "boolean" | readonly string[];

/** A param spec: a bare type (required) or `{ type, required? }`. */
export type LocalParamSpec = LocalParamType | { type: LocalParamType; required?: boolean };

/** The spec passed to {@link localTool}. */
export interface LocalToolSpec {
  /** The tool name (the model + `flow().tool` reference it). */
  name: string;
  /** The argument schema (explicit — TS types are erased at runtime). */
  params?: Readonly<Record<string, LocalParamSpec>>;
  /** The implementation — receives the decoded args, returns a JSON-able value. */
  run: (args: Record<string, unknown>) => unknown;
  version?: string;
  description?: string;
  /** The defining module's `import.meta.url` (auto-captured from the call site if
   *  omitted; pass it explicitly if capture fails, e.g. under a bundler). */
  module?: string;
}

/** A function exposed as a local MCP tool (the value put in `tools: [...]`). */
export interface LocalToolDef {
  readonly __kxLocalTool: true;
  readonly name: string;
  readonly version: string;
  readonly description: string;
  /** The derived MCP `inputSchema`. */
  readonly schema: Record<string, unknown>;
  /** The implementation (run IN the toolserver subprocess). */
  readonly run: (args: Record<string, unknown>) => unknown;
  /** The defining module (file URL / path), used to re-import in the toolserver. */
  readonly module: string;
}

/** Process-global registry the toolserver subprocess reads after re-importing a
 *  module (name → def). Backed by `globalThis` so it is ONE Map even when `tools.ts`
 *  is inlined into multiple bundles (tsup `splitting: false`) — the user registers via
 *  the SDK bundle, the toolserver reads via its own; both must see the same registry. */
export const LOCAL_TOOLS: Map<string, LocalToolDef> = (() => {
  const key = "__kx_local_tools__";
  const g = globalThis as Record<string, unknown>;
  const existing = g[key];
  if (existing instanceof Map) {
    return existing as Map<string, LocalToolDef>;
  }
  const m = new Map<string, LocalToolDef>();
  g[key] = m;
  return m;
})();

function jsonProp(t: LocalParamType): Record<string, unknown> {
  if (Array.isArray(t)) {
    return { enum: [...t] };
  }
  if (t === "integer") {
    return { type: "integer" };
  }
  if (t === "boolean") {
    return { type: "boolean" };
  }
  return { type: "string" };
}

function buildSchema(params?: Readonly<Record<string, LocalParamSpec>>): Record<string, unknown> {
  const properties: Record<string, unknown> = {};
  const required: string[] = [];
  for (const [name, spec] of Object.entries(params ?? {})) {
    if (typeof spec === "string" || Array.isArray(spec)) {
      properties[name] = jsonProp(spec); // a bare type or a string-enum array
      required.push(name);
    } else if (typeof spec === "object" && "type" in spec) {
      properties[name] = jsonProp(spec.type);
      if (spec.required !== false) {
        required.push(name);
      }
    }
  }
  const schema: Record<string, unknown> = { type: "object", properties };
  if (required.length > 0) {
    schema.required = required;
  }
  return schema;
}

/** Best-effort caller-module capture from the stack (no Node import — browser-safe).
 *  The first stack frame is THIS file (the SDK bundle — `dist/node.js` etc. once
 *  bundled, not literally `tools.ts`), so we skip every frame sharing that file and
 *  return the first user frame. Robust to bundling (tsup inlines `tools.ts` into each
 *  entry, so a filename-based skip would miss it). */
function callerModule(): string {
  const stack = new Error().stack ?? "";
  let selfFile: string | undefined;
  for (const line of stack.split("\n").slice(1)) {
    const m = line.match(/(file:\/\/\/\S+?|[/\\]\S+?):\d+:\d+/);
    if (m === null || m[1] === undefined) {
      continue;
    }
    const p: string = m[1];
    if (selfFile === undefined) {
      // The frame of `callerModule` itself — the SDK bundle to skip past.
      selfFile = p;
      continue;
    }
    if (p === selfFile || p.includes("/node_modules/")) {
      continue;
    }
    return p;
  }
  throw new KxToolError(
    "could not determine the calling module for localTool(); pass { module: import.meta.url }",
  );
}

/** Expose a JS function as a governed local tool (V2b). Returns a {@link LocalToolDef}
 *  to put in `tools: [...]` or `flow().tool(def, args)`; also registers it (so the
 *  toolserver subprocess recovers it after re-importing the module). */
export function localTool(spec: LocalToolSpec): LocalToolDef {
  if (typeof spec.run !== "function") {
    throw new KxToolError("localTool({ run }) must be a function");
  }
  const def: LocalToolDef = {
    __kxLocalTool: true,
    name: spec.name,
    version: spec.version ?? "1",
    description: spec.description ?? "",
    schema: buildSchema(spec.params),
    run: spec.run,
    module: spec.module ?? callerModule(),
  };
  LOCAL_TOOLS.set(def.name, def);
  return def;
}

/** Is `t` a {@link LocalToolDef}? */
export function isLocalTool(t: unknown): t is LocalToolDef {
  return (
    typeof t === "object" && t !== null && (t as { __kxLocalTool?: boolean }).__kxLocalTool === true
  );
}

/** The local-tool defs in a `tools` value (for the dynamic react-auto lane). */
export function localToolsOf(
  tools?: readonly (string | LocalToolDef)[] | Readonly<Record<string, string>>,
): LocalToolDef[] {
  if (tools === undefined || !Array.isArray(tools)) {
    return [];
  }
  return tools.filter(isLocalTool);
}

/** A deterministic dependency-free server name per defining module (so re-runs
 *  upsert the same connection — `connection_id_of(name)` is deterministic, SN-8). */
export function serverNameFor(module: string): string {
  // FNV-1a 32-bit — stable + dependency-free (no cryptographic strength needed).
  let h = 0x811c9dc5;
  for (let i = 0; i < module.length; i++) {
    h ^= module.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return `kxlocal-${(h >>> 0).toString(16).padStart(8, "0")}`;
}

/** A minimal client surface the resolver needs. */
export interface ToolGatewayClient {
  registerMcpServer(input: {
    name: string;
    transport: string;
    endpoint: string;
    args: string[];
  }): Promise<unknown>;
  discoverServerTools(name: string): Promise<{ tools: ReadonlyArray<{ toolName: string }> }>;
}

/** Resolve a local tool's namespaced `<server>/<name>` from a server's discovered set. */
function resolvedName(
  serverName: string,
  toolName: string,
  registered: ReadonlyArray<{ toolName: string }>,
): string {
  const want = `${serverName}/${toolName}`;
  for (const rt of registered) {
    if (rt.toolName === want) {
      return rt.toolName;
    }
  }
  for (const rt of registered) {
    const full = rt.toolName;
    if (full.includes("/") && full.slice(full.indexOf("/") + 1) === toolName) {
      return full;
    }
  }
  throw new KxToolError(
    `local tool '${toolName}' was not discovered on server '${serverName}' (registration mismatch)`,
  );
}

/** Guard: refuse to (re)register while the toolserver is re-importing a module — a
 *  stray un-guarded top-level `.run()` would recurse. */
function assertNotReentrant(): void {
  const env = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process
    ?.env;
  if (env?.KX_TOOLSERVE !== undefined) {
    throw new KxToolError(
      "cannot start a run while serving localTool() tools (the toolserver re-import context) — " +
        "guard your .run() calls (e.g. behind a main/entry check) so they do not re-execute.",
    );
  }
}

/** The Node binary + the built `_toolserver.js` entry (Node-only; throws in a browser).
 *  The toolserver sits beside the running bundle in `dist/`; we resolve its directory
 *  from `import.meta.url` (the ESM build) or `__dirname` (the CJS build — esbuild emits
 *  one or the other depending on the consumed format). */
async function spawnTarget(): Promise<{ node: string; entry: string }> {
  assertNotReentrant();
  const proc = (globalThis as { process?: { execPath?: string } }).process;
  if (proc?.execPath === undefined) {
    throw new KxToolError(
      "local tools require the Node SDK (@kortecx/sdk/node) — a browser cannot spawn a tool server",
    );
  }
  const { fileURLToPath } = await import("node:url");
  const { dirname, join } = await import("node:path");
  const metaUrl = (import.meta as { url?: string }).url;
  const baseDir =
    typeof metaUrl === "string" && metaUrl.length > 0
      ? dirname(fileURLToPath(metaUrl))
      : ((globalThis as { __dirname?: string }).__dirname ?? __dirname);
  return { node: proc.execPath, entry: join(baseDir, "_toolserver.js") };
}

async function toFileUrl(module: string): Promise<string> {
  if (module.startsWith("file:")) {
    return module;
  }
  const { pathToFileURL } = await import("node:url");
  return pathToFileURL(module).href;
}

/** Register every local tool a chain references (Node), returning the def → server-derived
 *  `<server>/<name>` map (or `undefined` when there are none). */
export async function resolveLocalTools(
  client: ToolGatewayClient,
  chain: Chain,
): Promise<ReadonlyMap<LocalToolDef, string> | undefined> {
  assertNotReentrant();
  const defs = chain.collectLocalTools();
  if (defs.length === 0) {
    return undefined;
  }
  const { node, entry } = await spawnTarget();
  const byModule = new Map<string, LocalToolDef[]>();
  for (const d of defs) {
    const list = byModule.get(d.module);
    if (list === undefined) {
      byModule.set(d.module, [d]);
    } else {
      list.push(d);
    }
  }
  const resolved = new Map<LocalToolDef, string>();
  for (const [module, group] of byModule) {
    const serverName = serverNameFor(module);
    const url = await toFileUrl(module);
    const names = [...new Set(group.map((d) => d.name))].sort();
    await client.registerMcpServer({
      name: serverName,
      transport: "stdio",
      endpoint: node,
      args: [entry, "--module", url, "--tools", names.join(",")],
    });
    const page = await client.discoverServerTools(serverName);
    for (const d of group) {
      resolved.set(d, resolvedName(serverName, d.name, page.tools));
    }
  }
  return resolved;
}

/** Register the local tools in a `tools=` set WITHOUT rewriting a contract — used by
 *  the dynamic react-auto lane (which auto-grants the live registry). */
export async function registerLocalTools(
  client: ToolGatewayClient,
  tools?: readonly (string | LocalToolDef)[] | Readonly<Record<string, string>>,
): Promise<void> {
  const defs = localToolsOf(tools);
  if (defs.length === 0) {
    return;
  }
  const { node, entry } = await spawnTarget();
  const byModule = new Map<string, LocalToolDef[]>();
  for (const d of defs) {
    const list = byModule.get(d.module);
    if (list === undefined) {
      byModule.set(d.module, [d]);
    } else {
      list.push(d);
    }
  }
  for (const [module, group] of byModule) {
    const url = await toFileUrl(module);
    const names = [...new Set(group.map((d) => d.name))].sort();
    await client.registerMcpServer({
      name: serverNameFor(module),
      transport: "stdio",
      endpoint: node,
      args: [entry, "--module", url, "--tools", names.join(",")],
    });
  }
}

/** A standalone TOOL node firing a local function deterministically (V2b). */
export function localToolNode(
  def: LocalToolDef,
  args: Readonly<Record<string, string | number | boolean>> = {},
): Task {
  return task.localTool(def, args);
}
