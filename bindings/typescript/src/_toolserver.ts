/**
 * A minimal stdio MCP server for `localTool(...)` functions (Batch V2b, Node-only).
 *
 * The runtime SPAWNS this (`node _toolserver.js --module <url> --tools a,b`) when it
 * dials an SDK-registered local tool server. It re-imports the user's module to
 * recover the `localTool(...)` registrations, then speaks newline-delimited JSON-RPC
 * 2.0 over stdin/stdout — the wire the runtime's `StdioSession` drives (`initialize`
 * → `tools/list` → `tools/call`). Hand-rolled (no `mcp` dependency).
 *
 * Re-import: top-level `localTool(...)` calls register into the process registry;
 * guard any `.run()` calls under an entry check (`import.meta.url === ...` / a main
 * guard) so they do not re-execute (`KX_TOOLSERVE` is set, so a stray run is refused).
 */

import { createInterface } from "node:readline";
import { LOCAL_TOOLS, type LocalToolDef } from "./tools.js";

const PROTOCOL_VERSION = "2026-07-28";

interface JsonRpcReq {
  id?: unknown;
  method?: string;
  params?: { name?: unknown; arguments?: unknown };
}

function parseArgs(argv: string[]): { module?: string; names: string[] } {
  let module: string | undefined;
  let names: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--module" && i + 1 < argv.length) {
      module = argv[++i];
    } else if (argv[i] === "--tools" && i + 1 < argv.length) {
      names = (argv[++i] ?? "").split(",").filter((n) => n.length > 0);
    }
  }
  return { module, names };
}

async function loadTools(
  moduleUrl: string | undefined,
  names: string[],
): Promise<Map<string, LocalToolDef>> {
  process.env.KX_TOOLSERVE = "1"; // a stray top-level run() is refused (see resolveLocalTools)
  if (moduleUrl !== undefined) {
    await import(moduleUrl); // registers via localTool()
  }
  const selected = new Map<string, LocalToolDef>();
  for (const n of names) {
    const d = LOCAL_TOOLS.get(n);
    if (d !== undefined) {
      selected.set(n, d);
    }
  }
  return selected;
}

function ok(id: unknown, result: unknown): string {
  return JSON.stringify({ jsonrpc: "2.0", id, result });
}

function err(id: unknown, code: number, message: string): string {
  return JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } });
}

async function handle(req: JsonRpcReq, tools: Map<string, LocalToolDef>): Promise<string | null> {
  if (!("id" in req)) {
    return null; // a notification (e.g. notifications/initialized) — no reply
  }
  const id = req.id;
  if (req.method === "initialize") {
    return ok(id, {
      protocolVersion: PROTOCOL_VERSION,
      capabilities: {},
      serverInfo: { name: "kortecx-local-tools", version: "1" },
    });
  }
  if (req.method === "tools/list") {
    return ok(id, {
      tools: [...tools.values()].map((t) => ({
        name: t.name,
        description: t.description,
        inputSchema: t.schema,
      })),
    });
  }
  if (req.method === "tools/call") {
    const name = req.params?.name;
    const def = typeof name === "string" ? tools.get(name) : undefined;
    if (def === undefined) {
      return err(id, -32602, `no such tool: ${String(name)}`);
    }
    const args = req.params?.arguments;
    if (typeof args !== "object" || args === null) {
      return err(id, -32602, "arguments must be a JSON object");
    }
    let result: unknown;
    try {
      result = await def.run(args as Record<string, unknown>);
    } catch (e) {
      return err(id, -32000, `${(e as Error)?.name ?? "Error"}: ${(e as Error)?.message ?? e}`);
    }
    try {
      JSON.stringify(result); // the runtime extracts `result` verbatim
    } catch {
      return err(id, -32000, "tool returned a non-JSON-serializable value");
    }
    return ok(id, result);
  }
  return err(id, -32601, `no such method: ${req.method}`);
}

export async function main(argv: string[] = process.argv.slice(2)): Promise<void> {
  const { module, names } = parseArgs(argv);
  const tools = await loadTools(module, names);
  const rl = createInterface({ input: process.stdin });
  for await (const line of rl) {
    const t = line.trim();
    if (t.length === 0) {
      continue;
    }
    let req: unknown;
    try {
      req = JSON.parse(t);
    } catch {
      process.stdout.write(`${err(0, -32700, "parse error")}\n`);
      continue;
    }
    if (typeof req !== "object" || req === null) {
      process.stdout.write(`${err(0, -32600, "invalid request")}\n`);
      continue;
    }
    const reply = await handle(req as JsonRpcReq, tools);
    if (reply !== null) {
      process.stdout.write(`${reply}\n`);
    }
  }
}

// Run when invoked directly (`node _toolserver.js ...`).
main().catch((e) => {
  process.stderr.write(`${String(e)}\n`);
  process.exit(1);
});
