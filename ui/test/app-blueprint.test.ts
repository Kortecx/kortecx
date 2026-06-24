import { Chain, type DagSpecJson } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import {
  appBlueprintToBuilderGraph,
  builderGraphToBlueprint,
} from "../src/components/builder/app-blueprint";

/** Round-trip a blueprint through the editor model and back to a blueprint. */
function roundtrip(bp: DagSpecJson): DagSpecJson {
  const { graph, unmodeled } = appBlueprintToBuilderGraph(bp);
  return builderGraphToBlueprint(graph, unmodeled);
}

/**
 * The CORPUS — every kind/form the App lineage editor must round-trip. Each must
 * satisfy the digest-safety property: the round-tripped blueprint COMPILES (via
 * Chain.fromBlueprint) byte-identical to the original. (Chain.fromBlueprint normalizes
 * both the SDK folded form and the CLI args-separated form, so the editor may emit
 * either — what matters is the compiled request, which is what the server admits.)
 */
const CORPUS: Record<string, DagSpecJson> = {
  "single model": { seed: 0, steps: [{ kind: "model", model_id: "m", prompt: "hi" }] },
  "pure step": { seed: 0, steps: [{ kind: "pure", params: { x: "1" } }] },
  "model with params + reasoning": {
    seed: 3,
    steps: [{ kind: "model", model_id: "m", prompt: "go", params: { reasoning: "off", k: "2" } }],
  },
  "agentic model (top-level budget)": {
    seed: 0,
    steps: [
      {
        kind: "model",
        prompt: "use the tool",
        tool_contract: { "mcp-echo/echo": "1" },
        max_turns: 6,
        max_tool_calls: 4,
      },
    ],
  },
  "agentic model (params-folded budget)": {
    seed: 0,
    steps: [
      {
        kind: "model",
        prompt: "use the tool",
        tool_contract: { "mcp-echo/echo": "1" },
        params: { max_turns: "6", max_tool_calls: "4" },
      },
    ],
  },
  "tool step (args map)": {
    seed: 0,
    steps: [{ kind: "tool", tool_contract: { "mcp-echo/echo": "1" }, args: { msg: "x", n: "3" } }],
  },
  "tool step (folded args)": {
    seed: 0,
    steps: [
      {
        kind: "tool",
        tool_contract: { "mcp-echo/echo": "1" },
        params: { "kx.tool.args": '{"msg":"x","n":"3"}' },
      },
    ],
  },
  "multi-step data + control DAG": {
    seed: 7,
    steps: [
      { kind: "model", model_id: "m", prompt: "first" },
      { kind: "model", model_id: "m", prompt: "second" },
      { kind: "pure" },
    ],
    edges: [
      { parent: 0, child: 1, edge: "data" },
      { parent: 1, child: 2, edge: "control" },
    ],
  },
  "context bundles + execution mode": {
    seed: 2,
    execution_mode: "frozen",
    context_bundles: ["ctx/local/notes"],
    steps: [{ kind: "model", model_id: "m", prompt: "ground" }],
  },
};

describe("app-blueprint round-trip (digest-safety)", () => {
  for (const [name, bp] of Object.entries(CORPUS)) {
    it(`compiles identically after a round-trip: ${name}`, () => {
      const original = Chain.fromBlueprint(bp);
      const cycled = Chain.fromBlueprint(roundtrip(bp));
      expect(cycled).toEqual(original);
    });
  }

  it("preserves blueprint-level seed / execution_mode / context_bundles", () => {
    const bp = CORPUS["context bundles + execution mode"] as DagSpecJson;
    const out = roundtrip(bp);
    expect(out.seed).toBe(2);
    expect(out.execution_mode).toBe("frozen");
    expect(out.context_bundles).toEqual(["ctx/local/notes"]);
  });

  it("preserves a per-edge non_cascade flag", () => {
    const bp: DagSpecJson = {
      seed: 0,
      steps: [{ kind: "model", model_id: "m", prompt: "a" }, { kind: "pure" }],
      edges: [{ parent: 0, child: 1, non_cascade: true }],
    };
    const out = roundtrip(bp);
    expect(out.edges?.[0]?.non_cascade).toBe(true);
  });

  it("inferKind parity: an agentic model step stays kind=model (not tool)", () => {
    const { graph } = appBlueprintToBuilderGraph({
      seed: 0,
      steps: [{ prompt: "go", tool_contract: { "mcp-echo/echo": "1" } }],
    });
    const s0 = graph.steps[0];
    expect(s0?.kind).toBe("model");
    expect(s0?.toolContract).toEqual({ "mcp-echo/echo": "1" });
  });

  it("refuses editing an unrepresentable (exec / body_signature_id) blueprint", () => {
    const withExec = appBlueprintToBuilderGraph({
      seed: 0,
      steps: [{ kind: "exec", body_signature_id: "a".repeat(64) }],
    });
    expect(withExec.unmodeled.refuseEdit).toBe(true);
    expect(withExec.unmodeled.reason).toBeTruthy();

    const ok = appBlueprintToBuilderGraph(CORPUS["single model"] as DagSpecJson);
    expect(ok.unmodeled.refuseEdit).toBe(false);
  });

  it("strips budget/reasoning/tool-args keys from the editable paramsText", () => {
    const { graph } = appBlueprintToBuilderGraph({
      seed: 0,
      steps: [
        {
          kind: "model",
          prompt: "go",
          tool_contract: { "mcp-echo/echo": "1" },
          params: { max_turns: "6", max_tool_calls: "4", reasoning: "off", keep: "1" },
        },
      ],
    });
    const s = graph.steps[0];
    expect(s?.maxTurns).toBe(6);
    expect(s?.maxToolCalls).toBe(4);
    expect(s?.reasoning).toBe("off");
    // only the genuine free param survives in the editor text
    expect(JSON.parse(s?.paramsText ?? "{}")).toEqual({ keep: "1" });
  });
});
