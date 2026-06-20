/**
 * Local function tools — `localTool(...)` (Batch V2b) — unit tests (no server, plus a
 * spawned stdio MCP server round-trip against the built `dist/_toolserver.js`).
 */

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { Agent, REACT_AUTO_RECIPE_HANDLE, REACT_RECIPE_HANDLE } from "../src/agent.js";
import { flow } from "../src/flow.js";
import {
  KxToolError,
  type LocalToolDef,
  isLocalTool,
  localTool,
  localToolsOf,
  registerLocalTools,
  resolveLocalTools,
  serverNameFor,
} from "../src/tools.js";

const here = dirname(fileURLToPath(import.meta.url));
const distIndex = resolve(here, "../dist/index.js");

// --- localTool + schema mapping ----------------------------------------------

describe("localTool — schema mapping", () => {
  it("maps explicit params + required", () => {
    const add = localTool({
      name: "add",
      description: "Add two ints.",
      params: { a: "integer", b: "string", c: "boolean", d: { type: "integer", required: false } },
      run: () => 0,
    });
    expect(isLocalTool(add)).toBe(true);
    expect(add.name).toBe("add");
    expect(add.version).toBe("1");
    expect(add.schema).toEqual({
      type: "object",
      properties: {
        a: { type: "integer" },
        b: { type: "string" },
        c: { type: "boolean" },
        d: { type: "integer" },
      },
      required: ["a", "b", "c"],
    });
  });

  it("maps a string array to an enum", () => {
    const t = localTool({ name: "pick", params: { mode: ["fast", "slow"] }, run: () => "" });
    expect(t.schema.properties).toEqual({ mode: { enum: ["fast", "slow"] } });
  });

  it("captures the calling module", () => {
    const t = localTool({ name: "x", run: () => 0 });
    expect(t.module).toContain("tools.test");
  });
});

// --- tool-set splitting + flow wiring -----------------------------------------

describe("tools=[...] splitting", () => {
  const add = localTool({ name: "add", params: { a: "integer" }, run: ({ a }) => a });

  it("a model step splits strings from local defs", () => {
    const lowered = flow()
      .agent("go", { tools: ["web-search", add] })
      .lower();
    // the string grant lowers as before; the local def rides off the lowering
    expect(lowered.steps[0]?.tool_contract).toEqual({ "web-search": "1" });
  });

  it("localToolsOf extracts the local defs", () => {
    expect(localToolsOf(["web-search", add]).map((d) => d.name)).toEqual(["add"]);
    expect(localToolsOf(["web-search"]).length).toBe(0);
  });

  it("flow().tool(def, args) builds a deferred local tool node", () => {
    const chainObj = flow().tool(add, { a: 7 }).toChain();
    const tools = chainObj.collectLocalTools();
    expect(tools.map((t) => t.name)).toEqual(["add"]);
    const lowered = chainObj.lower();
    expect(lowered.steps[0]?.kind).toBe("tool");
    expect(lowered.steps[0]?.tool_contract).toEqual({}); // filled at resolution
    expect(lowered.steps[0]?.params["kx.tool.args"]).toBe('{"a":7}');
  });
});

// --- run-terminal resolution (mock client) ------------------------------------

class FakeClient {
  registered: Array<{ name: string; transport: string; endpoint: string; args: string[] }> = [];
  invoked: Array<{ handle: string }> = [];

  async registerMcpServer(input: {
    name: string;
    transport: string;
    endpoint: string;
    args: string[];
  }): Promise<unknown> {
    this.registered.push(input);
    return { connectionId: "00", discovered: 1, health: "connected" };
  }

  async discoverServerTools(name: string): Promise<{ tools: ReadonlyArray<{ toolName: string }> }> {
    return { tools: [{ toolName: `${name}/add` }] };
  }

  async invoke(handle: string): Promise<unknown> {
    this.invoked.push({ handle });
    return "INVOKED";
  }

  // The frozen-tools Agent lane throws before this is reached (it satisfies AgentClient).
  async runChain(): Promise<unknown> {
    return "RAN";
  }
}

describe("resolveLocalTools", () => {
  const add = localTool({ name: "add", params: { a: "integer" }, run: ({ a }) => a });

  it("registers a stdio server + resolves the namespaced name", async () => {
    const fc = new FakeClient();
    const chainObj = flow().tool(add, { a: 2 }).toChain();
    const resolved = await resolveLocalTools(fc, chainObj);
    const serverName = serverNameFor(add.module);
    expect(resolved?.get(add)).toBe(`${serverName}/add`);
    expect(fc.registered[0]?.transport).toBe("stdio");
    expect(fc.registered[0]?.name).toBe(serverName);
    expect(fc.registered[0]?.args).toContain("--module");
    expect(fc.registered[0]?.args[0]).toContain("_toolserver");
  });

  it("is a no-op without local tools", async () => {
    const fc = new FakeClient();
    const chainObj = flow()
      .agent("hi", { tools: ["web-search"] })
      .toChain();
    const resolved = await resolveLocalTools(fc, chainObj);
    expect(resolved).toBeUndefined();
    expect(fc.registered.length).toBe(0);
  });
});

// --- Agent routing ------------------------------------------------------------

describe("Agent routing", () => {
  const add = localTool({ name: "add", params: { a: "integer" }, run: ({ a }) => a });

  it("frozen + tools throws a pre-flight hint", async () => {
    await expect(
      new Agent("go", { tools: [add] }).run("2+2", { client: new FakeClient() }),
    ).rejects.toThrow(KxToolError);
  });

  it("dynamic + local tools registers + routes to react-auto", async () => {
    const fc = new FakeClient();
    await new Agent("go", { tools: [add], dynamic: true }).run("2+2", { client: fc });
    expect(fc.registered.length).toBeGreaterThan(0);
    expect(fc.invoked[0]?.handle).toBe(REACT_AUTO_RECIPE_HANDLE);
  });

  it("dynamic without tools uses plain react", async () => {
    const fc = new FakeClient();
    await new Agent("chat", { dynamic: true }).run("hi", { client: fc });
    expect(fc.registered.length).toBe(0);
    expect(fc.invoked[0]?.handle).toBe(REACT_RECIPE_HANDLE);
  });
});

// --- the stdio MCP server round-trip (spawns dist/_toolserver.js) --------------

function roundTrip(moduleUrl: string, reqs: object[]): Promise<Record<number, any>> {
  const entry = resolve(here, "../dist/_toolserver.js");
  return new Promise((resolveP, rejectP) => {
    const proc = spawn(process.execPath, [entry, "--module", moduleUrl, "--tools", "add"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    let out = "";
    let err = "";
    proc.stdout.on("data", (d) => {
      out += d;
    });
    proc.stderr.on("data", (d) => {
      err += d;
    });
    proc.on("error", rejectP);
    proc.on("close", () => {
      try {
        const byId: Record<number, any> = {};
        for (const line of out.split("\n")) {
          if (line.trim()) {
            const r = JSON.parse(line);
            byId[r.id] = r;
          }
        }
        resolveP(byId);
      } catch (e) {
        rejectP(new Error(`bad toolserver output: ${out}\nstderr: ${err}\n${e}`));
      }
    });
    proc.stdin.write(`${reqs.map((r) => JSON.stringify(r)).join("\n")}\n`);
    proc.stdin.end();
  });
}

describe("the stdio toolserver", () => {
  it("serves initialize / tools/list / tools/call and skips a main guard", async () => {
    const dir = mkdtempSync(join(tmpdir(), "kxtool-"));
    const mod = join(dir, "mytools.mjs");
    writeFileSync(
      mod,
      `import { localTool } from ${JSON.stringify(pathToFileURL(distIndex).href)};\n` +
        `localTool({ name: "add", description: "Add.", params: { a: "integer", b: "integer" }, run: ({ a, b }) => a + b });\n` +
        `globalThis.__MAIN_RAN = true; // not a guard; the toolserver must still answer\n`,
    );
    const byId = await roundTrip(pathToFileURL(mod).href, [
      { jsonrpc: "2.0", id: 1, method: "initialize", params: {} },
      { jsonrpc: "2.0", method: "notifications/initialized" }, // no id ⇒ no reply
      { jsonrpc: "2.0", id: 2, method: "tools/list" },
      {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "add", arguments: { a: 2, b: 40 } },
      },
      { jsonrpc: "2.0", id: 4, method: "tools/call", params: { name: "nope", arguments: {} } },
    ]);
    expect(Object.keys(byId).sort()).toEqual(["1", "2", "3", "4"]); // a notification gets no reply
    expect(byId[1].result.protocolVersion).toBeTruthy();
    expect(byId[2].result.tools[0].name).toBe("add");
    expect(byId[2].result.tools[0].inputSchema.required).toEqual(["a", "b"]);
    expect(byId[3].result).toBe(42); // the runtime extracts `result` verbatim
    expect(byId[4].error.code).toBe(-32602);
  });
});
