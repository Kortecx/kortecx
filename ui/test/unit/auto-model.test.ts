import type { ModelSummary } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { resolveAutoModel } from "../../src/lib/auto-model";

function model(modelId: string, active = false): ModelSummary {
  return { modelId, active, modalities: [] } as unknown as ModelSummary;
}

describe("resolveAutoModel (Model Control v2 — shared Auto resolution)", () => {
  it("returns undefined when nothing is served", () => {
    expect(resolveAutoModel(undefined, undefined)).toBeUndefined();
    expect(resolveAutoModel([], "gemma-4-12b")).toBeUndefined();
  });

  it("prefers the server-active model over a client default and the first listed", () => {
    const models = [model("a"), model("b", true), model("c")];
    expect(resolveAutoModel(models, "a")).toBe("b");
  });

  it("falls back to a SERVED client-local default when no model is active", () => {
    expect(resolveAutoModel([model("a"), model("b")], "b")).toBe("b");
  });

  it("ignores an UNSERVED client default and uses the first listed (never names a stale model)", () => {
    expect(resolveAutoModel([model("a"), model("b")], "gone")).toBe("a");
  });

  it("uses the first listed when there is neither an active model nor a default", () => {
    expect(resolveAutoModel([model("a"), model("b")], undefined)).toBe("a");
  });
});
