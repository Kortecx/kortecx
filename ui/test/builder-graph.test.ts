import { TOOL_ARGS_KEY, flow, proto } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import {
  type BuilderGraph,
  type BuilderStep,
  CONSENSUS_VOTE_KEY,
  PATTERN_PROMPTS,
  type PatternKind,
  builderTopologyHash,
  insertPattern,
  isJsonObject,
  newStep,
  toRequest,
  validateAcyclic,
  validationError,
} from "../src/components/builder/builder-graph";

function graph(steps: ReturnType<typeof newStep>[], edges: BuilderGraph["edges"]): BuilderGraph {
  return { steps, edges };
}

describe("builder-graph", () => {
  it("acyclicity: a linear chain is valid, a cycle is rejected", () => {
    const a = newStep("model", "a");
    const b = newStep("model", "b");
    const chain = graph(
      [a, b],
      [{ id: "e1", source: "a", target: "b", edge: "data", instruction: "" }],
    );
    expect(validateAcyclic(chain).ok).toBe(true);

    const cyclic = graph(
      [a, b],
      [
        { id: "e1", source: "a", target: "b", edge: "data", instruction: "" },
        { id: "e2", source: "b", target: "a", edge: "data", instruction: "" },
      ],
    );
    const r = validateAcyclic(cyclic);
    expect(r.ok).toBe(false);
    expect(r.cycle).toEqual(["a", "b"]);
  });

  it("topology hash is stable across config edits, changes on structure", () => {
    const a = newStep("model", "a");
    const b = newStep("pure", "b");
    const g1 = graph(
      [a, b],
      [{ id: "e", source: "a", target: "b", edge: "data", instruction: "" }],
    );
    const g2 = graph(
      [{ ...a, prompt: "edited", label: "Renamed" }, b],
      [{ id: "e", source: "a", target: "b", edge: "data", instruction: "edited" }],
    );
    expect(builderTopologyHash(g1)).toBe(builderTopologyHash(g2)); // config-only edit
    const g3 = graph([a, b], []); // structural change
    expect(builderTopologyHash(g1)).not.toBe(builderTopologyHash(g3));
  });

  it("validationError catches empty graph, missing model, bad params", () => {
    expect(validationError(graph([], []))).toMatch(/at least one/i);
    const noModel = newStep("model", "a");
    expect(validationError(graph([noModel], []))).toMatch(/needs a model/i);
    const badParams = { ...newStep("pure", "p"), paramsText: "[1,2]" };
    expect(validationError(graph([badParams], []))).toMatch(/JSON object/i);
    const ok = { ...newStep("model", "a"), modelId: "kx-serve:qwen3" };
    expect(validationError(graph([ok], []))).toBeNull();
  });

  it("isJsonObject accepts blank + objects, rejects arrays/scalars/garbage", () => {
    expect(isJsonObject("")).toBe(true);
    expect(isJsonObject('{"k":1}')).toBe(true);
    expect(isJsonObject("[1]")).toBe(false);
    expect(isJsonObject("5")).toBe(false);
    expect(isJsonObject("not json")).toBe(false);
  });

  it("toRequest lowers a 2-agent chain: topology+params only, instruction folds into the child prompt", () => {
    const draft = { ...newStep("model", "draft"), modelId: "kx-serve:m", prompt: "Draft it." };
    const critique = {
      ...newStep("model", "critique"),
      modelId: "kx-serve:m",
      prompt: "Improve it.",
    };
    const g = graph(
      [draft, critique],
      [{ id: "e", source: "draft", target: "critique", edge: "data", instruction: "Be concise." }],
    );
    const req = toRequest(g, 0);
    expect(req.steps).toHaveLength(2);
    // The edge instruction is prepended to the downstream agent's prompt (Tier-1 D141.5).
    expect(req.steps?.[1]?.prompt).toBe("Be concise.\n\nImprove it.");
    expect(req.edges).toHaveLength(1);
    expect(req.edges?.[0]?.parent).toBe(0);
    expect(req.edges?.[0]?.child).toBe(1);
    // SN-8: the request carries NO warrants / no MoteIds — only topology + params.
    expect(Object.keys(req)).not.toContain("warrants");
  });

  it("toRequest omits reasoning by default (digest-neutral) and includes it when set", () => {
    const base = { ...newStep("model", "a"), modelId: "kx-serve:m", prompt: "hi" };
    const def = toRequest(graph([base], []));
    expect(def.steps?.[0]?.params ?? {}).not.toHaveProperty("reasoning");
    const off = toRequest(graph([{ ...base, reasoning: "off" as const }], []));
    // params values are byte-encoded by the SDK builder.
    expect(Object.keys(off.steps?.[0]?.params ?? {})).toContain("reasoning");
  });

  it("PR-6b-2: a tool node lowers to TOOL kind + tool_contract + canonical args", () => {
    const t = {
      ...newStep("tool", "t"),
      toolId: "web-search",
      toolVersion: "1",
      paramsText: '{"q":"hi","n":2}',
    };
    const req = toRequest(graph([t], []));
    expect(req.steps).toHaveLength(1);
    const step = req.steps?.[0];
    expect(step?.kind).toBe(proto.WorkflowStepKind.TOOL);
    expect(step?.toolContract).toEqual({ "web-search": "1" });
    // The args lower to the canonical TOOL_ARGS_KEY blob (sorted keys, compact) —
    // byte-identical to the Py/TS/CLI surfaces (the golden-corpus contract).
    const args = step?.params?.[TOOL_ARGS_KEY];
    expect(new TextDecoder().decode(args as Uint8Array)).toBe('{"n":2,"q":"hi"}');
  });

  it("PR-6b-2: a tool node without a picked tool fails validation", () => {
    expect(validationError(graph([newStep("tool", "t")], []))).toMatch(/needs a registered tool/);
  });
});

describe("validationError allowEmptyModel (POC-5d App lineage)", () => {
  it("empty modelId is invalid by default, valid with allowEmptyModel", () => {
    const a = newStep("model", "a");
    const g: BuilderGraph = { steps: [a], edges: [] };
    expect(validationError(g)).toMatch(/needs a model/i);
    expect(validationError(g, { allowEmptyModel: true })).toBeNull();
  });
});

// -- orchestration pattern macros -------------------------------------------

const edgeKey = (e: BuilderGraph["edges"][number]) => `${e.source}->${e.target}`;

describe("insertPattern — orchestration macros", () => {
  it("swarm: N model participants fan into one MODEL gather", () => {
    const p = insertPattern("swarm", 0);
    expect(p.steps.map((s) => s.kind)).toEqual(["model", "model", "model"]);
    expect(p.steps.map((s) => s.id)).toEqual(["s0", "s1", "s2"]);
    expect(p.firstId).toBe("s0");
    expect(p.nextId).toBe(3);
    expect(p.steps[2]?.label).toBe("Gather");
    expect(p.steps[2]?.prompt).toBe(PATTERN_PROMPTS.swarmGather);
    expect(p.edges.map(edgeKey).sort()).toEqual(["s0->s2", "s1->s2"]);
    expect(p.edges.every((e) => e.edge === "data")).toBe(true);
  });

  it("supervisor: planner → workers (in parallel) → gather", () => {
    const p = insertPattern("supervisor", 0);
    expect(p.steps.map((s) => s.label)).toEqual(["Planner", "Worker", "Worker", "Gather"]);
    expect(p.steps.every((s) => s.kind === "model")).toBe(true);
    expect(p.steps[0]?.prompt).toBe(PATTERN_PROMPTS.supervisorPlanner);
    expect(p.steps[3]?.prompt).toBe(PATTERN_PROMPTS.supervisorGather);
    expect(p.edges.map(edgeKey).sort()).toEqual(["s0->s1", "s0->s2", "s1->s3", "s2->s3"]);
  });

  it("consensus (judge): voters → a MODEL judge that selects the best", () => {
    const p = insertPattern("consensusJudge", 0);
    expect(p.steps.map((s) => s.kind)).toEqual(["model", "model", "model"]);
    expect(p.steps[2]?.label).toBe("Judge");
    expect(p.steps[2]?.prompt).toBe(PATTERN_PROMPTS.consensusJudge);
    expect(p.edges.map(edgeKey).sort()).toEqual(["s0->s2", "s1->s2"]);
  });

  it("consensus (majority): voters → a PURE exact-equality vote sink (honest-gated)", () => {
    const p = insertPattern("consensusMajority", 0);
    expect(p.steps.map((s) => s.kind)).toEqual(["model", "model", "pure"]);
    const sink = p.steps[2];
    expect(sink?.label).toBe("Majority vote");
    expect(JSON.parse(sink?.paramsText ?? "{}")).toEqual({ [CONSENSUS_VOTE_KEY]: "majority" });
    expect(p.edges.map(edgeKey).sort()).toEqual(["s0->s2", "s1->s2"]);
  });

  it("mints ids from the given counter and reports the next free id", () => {
    const p = insertPattern("consensusJudge", 5);
    expect(p.steps.map((s) => s.id)).toEqual(["s5", "s6", "s7"]);
    expect(p.firstId).toBe("s5");
    expect(p.nextId).toBe(8);
  });

  it("defaults to 2 participants but honors a larger count", () => {
    const p = insertPattern("swarm", 0, 4);
    expect(p.steps.filter((s) => s.label === "Agent")).toHaveLength(4);
    expect(p.edges).toHaveLength(4); // each participant → gather
    expect(insertPattern("swarm", 0, 0).steps.filter((s) => s.label === "Agent")).toHaveLength(1);
  });

  it("a fresh pattern is submittable once each agent gets a model (honest gate)", () => {
    const p = insertPattern("supervisor", 0);
    // As inserted, the model nodes have no model → the canvas surfaces the gate.
    expect(validationError({ steps: p.steps, edges: p.edges })).toMatch(/needs a model/i);
    const withModels = { steps: p.steps.map(pinModel), edges: p.edges };
    expect(validationError(withModels)).toBeNull();
  });
});

/** Pin a served model on every MODEL node (the one-shot builder requires an explicit
 *  model; the SDK's App convention leaves it empty — the one intended difference). */
function pinModel(s: BuilderStep): BuilderStep {
  return s.kind === "model" ? { ...s, modelId: "kx-serve:m" } : s;
}

/** A DAG *shape* signature — step kinds (in lowered order) + sorted parent→child
 *  edges + any per-step consensus-vote marker. Ignores model id + prompts (author
 *  content that legitimately differs between the one-shot builder and the SDK). */
function shapeOf(req: ReturnType<typeof toRequest>) {
  const dec = new TextDecoder();
  return {
    kinds: (req.steps ?? []).map((s) => s.kind),
    edges: (req.edges ?? []).map((e) => `${e.parent}->${e.child}`).sort(),
    votes: (req.steps ?? []).map((s) => {
      const v = s.params?.[CONSENSUS_VOTE_KEY];
      return v ? dec.decode(v as Uint8Array) : null;
    }),
  };
}

describe("insertPattern lowering ≡ the SDK flow() lowering (UI arm of tri-surface parity)", () => {
  it.each([
    ["swarm", () => flow().swarm(["a", "b"])],
    ["supervisor", () => flow().supervisor(["a", "b"])],
    ["consensusJudge", () => flow().consensus(["a", "b"], { vote: "judge" })],
    ["consensusMajority", () => flow().consensus(["a", "b"], { vote: "majority" })],
  ] as const)("%s: the UI macro lowers to the same DAG shape as the SDK", (kind, sdk) => {
    const macro = insertPattern(kind as PatternKind, 0);
    const uiReq = toRequest({ steps: macro.steps.map(pinModel), edges: macro.edges }, 0);
    expect(shapeOf(uiReq)).toEqual(shapeOf(sdk().build()));
  });

  it("the majority sink lowers the vote marker to config bytes (server-reduced)", () => {
    const macro = insertPattern("consensusMajority", 0);
    const req = toRequest({ steps: macro.steps.map(pinModel), edges: macro.edges }, 0);
    const sink = req.steps?.[2];
    expect(sink?.kind).toBe(proto.WorkflowStepKind.PURE);
    const v = sink?.params?.[CONSENSUS_VOTE_KEY];
    expect(new TextDecoder().decode(v as Uint8Array)).toBe("majority");
  });
});
