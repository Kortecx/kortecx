import { describe, expect, it } from "vitest";
import { splitReasoning } from "../src/lib/split-reasoning";

describe("splitReasoning", () => {
  it("splits a leading <think> block from the answer", () => {
    const r = splitReasoning("<think>let me reason</think>The answer.");
    expect(r.reasoning).toBe("let me reason");
    expect(r.answer).toBe("The answer.");
  });

  it("tolerates leading whitespace + trims the answer", () => {
    const r = splitReasoning("  <think> step </think>\n\nHello");
    expect(r.reasoning).toBe("step");
    expect(r.answer).toBe("Hello");
  });

  it("no <think> ⇒ the whole text is the answer (no reasoning)", () => {
    const r = splitReasoning("Just an answer.");
    expect(r.reasoning).toBeUndefined();
    expect(r.answer).toBe("Just an answer.");
  });

  it("unclosed <think> fails OPEN to the whole text (never hide content)", () => {
    const r = splitReasoning("<think>still thinking with no close");
    expect(r.reasoning).toBeUndefined();
    expect(r.answer).toBe("<think>still thinking with no close");
  });

  it("reasoning-only (empty answer) is preserved", () => {
    const r = splitReasoning("<think>only thoughts</think>");
    expect(r.reasoning).toBe("only thoughts");
    expect(r.answer).toBe("");
  });

  it("only the LEADING block is split (a mid-string <think> stays in the answer)", () => {
    const r = splitReasoning("Answer mentions <think> as a literal.");
    expect(r.reasoning).toBeUndefined();
    expect(r.answer).toBe("Answer mentions <think> as a literal.");
  });
});
