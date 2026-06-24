/**
 * POC-5d: `runApp({args})` folds the App's input_schema inputs into the entry model
 * step's prompt. The pure `injectAppArgs` is the digest-relevant core: a NO-OP when
 * args are empty/absent (byte-identical compile), and it never mutates the source.
 */

import { describe, expect, it } from "vitest";
import type { DagSpecJson } from "../src/chains.js";
import { injectAppArgs } from "../src/client.js";

const BP: DagSpecJson = {
  seed: 0,
  steps: [{ kind: "pure" }, { kind: "model", model_id: "m", prompt: "Answer the question." }],
};

describe("injectAppArgs (POC-5d run inputs)", () => {
  it("is a no-op (same reference) when args are empty/absent", () => {
    expect(injectAppArgs(BP, undefined)).toBe(BP);
    expect(injectAppArgs(BP, {})).toBe(BP);
  });

  it("folds args into the FIRST model step's prompt, leaving others untouched", () => {
    const out = injectAppArgs(BP, { topic: "kortecx", n: "3" });
    expect(out).not.toBe(BP); // a new object — never mutates the source
    expect(out.steps[0]).toEqual({ kind: "pure" }); // the pure step is unchanged
    const model = out.steps[1];
    expect(model?.prompt).toContain("Answer the question.");
    expect(model?.prompt).toContain("Inputs:");
    expect(model?.prompt).toContain("- topic: kortecx");
    expect(model?.prompt).toContain("- n: 3");
    // The source is never mutated.
    expect(BP.steps[1]?.prompt).toBe("Answer the question.");
  });

  it("no model step ⇒ unchanged (args have nowhere to fold)", () => {
    const toolOnly: DagSpecJson = {
      seed: 0,
      steps: [{ kind: "tool", tool_contract: { "x/y": "1" } }],
    };
    expect(injectAppArgs(toolOnly, { a: "b" })).toBe(toolOnly);
  });
});
