/**
 * The fluent Flow builder + first-class Agent — pure unit tests (no server).
 *
 * A Flow is sugar over the combinator API, so the core assertion is PARITY: a fluent
 * chain lowers byte-identically to the equivalent combinator/DSL chain.
 */

import { describe, expect, it } from "vitest";
import { Agent } from "../src/agent.js";
import { ChainParseError, chain, chainFrom, par, seq, task } from "../src/chains.js";
import type { KxClientBase } from "../src/client.js";
import { getDefaultClient, setDefaultClient } from "../src/default-client.js";
import { Flow, flow } from "../src/flow.js";
import { Result, Run } from "../src/run.js";

/** A minimal stub for the Flow/Agent terminals — records which wait path runs. */
class FakeClient {
  anyCalls = 0;
  termCalls = 0;
  async runChain(_chain: unknown, opts: { wait?: boolean } = {}): Promise<unknown> {
    // empty terminal ⇒ Run.wait() takes the await-any path.
    const run = new Run(
      this as unknown as KxClientBase,
      new Uint8Array(16).fill(1),
      new Uint8Array(0),
      new Uint8Array(0),
    );
    return opts.wait ? this._awaitAny() : run;
  }
  async _awaitAny(): Promise<Result> {
    this.anyCalls++;
    return "ANY" as unknown as Result;
  }
  async _awaitTerminal(): Promise<Result> {
    this.termCalls++;
    return "TERM" as unknown as Result;
  }
  // AgentClient surface (the frozen no-tools lane never reaches these).
  async invoke(): Promise<unknown> {
    return "INVOKED";
  }
  async registerMcpServer(): Promise<unknown> {
    return {};
  }
  async discoverServerTools(
    _name: string,
  ): Promise<{ tools: ReadonlyArray<{ toolName: string }> }> {
    return { tools: [] };
  }
  async bindReactVision(
    args: Record<string, unknown>,
    _image: unknown,
  ): Promise<{ handle: string; args: Record<string, unknown> }> {
    return { handle: "kx/recipes/react-vision", args: { ...args, image_ref: "ab".repeat(32) } };
  }
}

describe("Flow — fluent builder", () => {
  it("a sequence matches the combinator form", () => {
    const fluent = flow()
      .agent("research", { tools: ["web-search"] })
      .then("review")
      .lower();
    const combinator = chainFrom(
      seq(task.model("", "research", {}, { tools: ["web-search"] }), task.model("", "review")),
    ).lower();
    expect(fluent).toEqual(combinator);
    expect(fluent.steps.map((s) => s.kind)).toEqual(["model", "model"]);
    expect(fluent.steps[0]?.model_id).toBe("");
    expect(fluent.steps[0]?.tool_contract).toEqual({ "web-search": "1" });
    expect(fluent.edges).toEqual([{ parent: 0, child: 1, edge: "data" }]);
  });

  it("parallel fans out and in", () => {
    const fanOut = flow().agent("a").parallel("b", "c").lower();
    expect(fanOut.steps).toHaveLength(3);
    expect(fanOut.edges.map((e) => [e.parent, e.child]).sort()).toEqual([
      [0, 1],
      [0, 2],
    ]);
    const fanIn = flow().parallel("a", "b").then("c").lower();
    expect(fanIn.edges.map((e) => [e.parent, e.child]).sort()).toEqual([
      [0, 2],
      [1, 2],
    ]);
  });

  it("step and tool", () => {
    const lowered = flow().step({ topic: "hi" }).tool("echo", "1", { n: 3 }).lower();
    expect(lowered.steps[0]?.kind).toBe("pure");
    expect(lowered.steps[0]?.params).toEqual({ topic: "hi" });
    expect(lowered.steps[1]?.kind).toBe("tool");
    expect(lowered.steps[1]?.tool_contract).toEqual({ echo: "1" });
    expect(lowered.steps[1]?.params).toEqual({ "kx.tool.args": '{"n":3}' });
  });

  it("image() grounds the next agent step per-step (AGENTIC-VISION)", () => {
    // .image(ref) binds image_ref into ONLY the immediately-following agent step's config;
    // a step without a preceding .image() carries none (mirrors the Python test).
    const refA = "a".repeat(64);
    const refB = "b".repeat(64);
    const lowered = flow()
      .image(refA)
      .agent("inspect the chart")
      .then("now this one")
      .image(refB)
      .then("summarise")
      .lower();
    expect(lowered.steps[0]?.params).toEqual({ image_ref: refA });
    expect(lowered.steps[1]?.params).toEqual({});
    expect(lowered.steps[2]?.params).toEqual({ image_ref: refB });
  });

  it("matches the string DSL for the same topology", () => {
    const viaFlow = flow().agent("p").then("q").lower();
    const viaString = chain("p > q", {
      tasks: { p: task.model("", "p"), q: task.model("", "q") },
    }).lower();
    expect(viaFlow).toEqual(viaString);
  });

  it("context and seed flow through", () => {
    const lowered = flow({ seed: 7 }).agent("a").context("team/ctx/spec").lower();
    expect(lowered.context_bundles).toEqual(["team/ctx/spec"]);
  });

  it("an empty flow is fail-closed", () => {
    expect(() => flow().toChain()).toThrow(ChainParseError);
    expect(() => flow().parallel()).toThrow(ChainParseError);
  });

  it("flow() returns a Flow", () => {
    expect(flow()).toBeInstanceOf(Flow);
  });
});

describe("Agent", () => {
  it("the frozen lane is a single agent step", () => {
    const a = new Agent("You are helpful.", { tools: ["echo"], reasoning: "minimal" });
    const lowered = a.asFlow("do it").lower();
    expect(lowered.steps).toHaveLength(1);
    const step = lowered.steps[0];
    expect(step?.kind).toBe("model");
    expect(step?.tool_contract).toEqual({ echo: "1" });
    expect(step?.params).toEqual({ reasoning: "minimal" });
    expect(step?.prompt).toBe("You are helpful.\n\ndo it");
  });

  it("defaults to the frozen lane", () => {
    expect(new Agent("x").opts.dynamic ?? false).toBe(false);
    expect(new Agent("x", { dynamic: true }).opts.dynamic).toBe(true);
  });

  it("a bare task (no instructions) is the prompt verbatim", () => {
    const lowered = new Agent().asFlow("just do it").lower();
    expect(lowered.steps[0]?.prompt).toBe("just do it");
  });
});

describe("Result.json (g4)", () => {
  it("aliases toJSON", () => {
    const r = new Result("aa", "bb", "COMMITTED", "cc", new TextEncoder().encode("hi"));
    expect(r.json()).toEqual(r.toJSON());
    expect(r.json(false)).toEqual(r.toJSON(false));
    expect(r.json().state).toBe("COMMITTED");
  });
});

describe("V2a g1/g2 — Run-from-handle, await-any wait, zero-config, Agent.stream", () => {
  it("flow().submit() returns a Run; .wait() uses await-any (no static terminal)", async () => {
    const fc = new FakeClient();
    const run = await flow().agent("a").submit({ client: fc });
    expect(run).toBeInstanceOf(Run);
    await run.wait();
    expect(fc.anyCalls).toBe(1);
    expect(fc.termCalls).toBe(0);
  });

  it("flow().run() uses the registry default client when none is passed", async () => {
    const fc = new FakeClient();
    setDefaultClient(fc);
    try {
      const out = await flow().agent("a").run({ wait: false });
      expect(out).toBeInstanceOf(Run);
      expect(getDefaultClient()).toBe(fc);
    } finally {
      setDefaultClient(undefined);
    }
  });

  it("Agent.stream returns a Run", async () => {
    const run = await new Agent("hi").stream("task", { client: new FakeClient() });
    expect(run).toBeInstanceOf(Run);
  });

  // -- .withMcp() — connectors reachable from the single chaining entry point --

  it("withMcp registers connectors before submit, in declaration order", async () => {
    const fc = new FakeClient();
    const names: string[] = [];
    (
      fc as unknown as {
        registerMcpServer: (i: { name: string }) => Promise<unknown>;
      }
    ).registerMcpServer = async (i) => {
      names.push(i.name);
      return {};
    };
    await flow()
      .withMcp({ name: "a", endpoint: "x", args: ["--a"] })
      .agent("hi", { tools: ["a/echo"] })
      .withMcp({ name: "b", transport: "http", endpoint: "https://h/rpc" })
      .run({ client: fc, wait: false });
    expect(names).toEqual(["a", "b"]);
  });

  it("withMcp is digest-invariant (does not change the lowered request)", () => {
    const withConn = JSON.stringify(
      flow().agent("hi").withMcp({ name: "a", endpoint: "x" }).build(),
    );
    const plain = JSON.stringify(flow().agent("hi").build());
    expect(withConn).toBe(plain);
  });

  it("withMcp throws a clear error when the client cannot register connectors", async () => {
    const noMcp = { runChain: async () => "X" };
    await expect(
      flow().withMcp({ name: "a", endpoint: "x" }).agent("hi").run({ client: noMcp, wait: false }),
    ).rejects.toThrow(/register connectors/);
  });
});
