/**
 * The fluent Flow builder + first-class Agent — pure unit tests (no server).
 *
 * A Flow is sugar over the combinator API, so the core assertion is PARITY: a fluent
 * chain lowers byte-identically to the equivalent combinator/DSL chain.
 */

import { describe, expect, it } from "vitest";
import { Agent } from "../src/agent.js";
import { ChainParseError, chain, chainFrom, par, seq, task } from "../src/chains.js";
import { Flow, flow } from "../src/flow.js";

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
