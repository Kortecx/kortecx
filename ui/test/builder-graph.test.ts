import { describe, expect, it } from "vitest";
import {
  type BuilderGraph,
  builderTopologyHash,
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
});
