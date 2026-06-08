import { describe, expect, it } from "vitest";
import { eventSummary, eventVisual } from "../../src/lib/event-format";

describe("eventVisual", () => {
  it("maps known kinds to reused state tones", () => {
    expect(eventVisual("committed")).toEqual({ label: "COMMITTED", tone: "committed" });
    expect(eventVisual("failed")).toEqual({ label: "FAILED", tone: "failed" });
    expect(eventVisual("repudiated")).toEqual({ label: "REPUDIATED", tone: "repudiated" });
    expect(eventVisual("effect_staged")).toEqual({ label: "EFFECT STAGED", tone: "scheduled" });
  });

  it("unknown kind → unknown tone (never crashes)", () => {
    expect(eventVisual("future_kind").tone).toBe("unknown");
  });
});

describe("eventSummary", () => {
  const id = "ab".repeat(32);
  const ref = "cd".repeat(32);

  it("committed with a result ref", () => {
    const s = eventSummary({ seq: 3, kind: "committed", moteId: id, resultRef: ref });
    expect(s).toContain("committed");
    expect(s).toContain("→");
  });

  it("committed without a result ref omits the arrow", () => {
    const s = eventSummary({ seq: 3, kind: "committed", moteId: id });
    expect(s).toContain("committed");
    expect(s).not.toContain("→");
  });

  it("failed / repudiated / effect_staged read naturally", () => {
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id })).toContain("failed");
    expect(eventSummary({ seq: 1, kind: "repudiated", targetMoteId: id })).toContain("repudiated");
    expect(eventSummary({ seq: 1, kind: "effect_staged", moteId: id })).toContain("effect");
  });

  it("unknown kind falls back to a generic summary", () => {
    expect(eventSummary({ seq: 1, kind: "weird" })).toContain("weird");
  });
});
