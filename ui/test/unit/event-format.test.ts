import { describe, expect, it } from "vitest";
import { eventSummary, eventVisual, failureReasonLabel } from "../../src/lib/event-format";

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

  it("omitResultRef drops the trailing hash (the row renders resolved text instead)", () => {
    const s = eventSummary(
      { seq: 3, kind: "committed", moteId: id, resultRef: ref },
      undefined,
      true,
    );
    expect(s).toContain("committed");
    expect(s).not.toContain("→");
  });

  it("failed / repudiated / effect_staged read naturally", () => {
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id })).toContain("failed");
    expect(eventSummary({ seq: 1, kind: "repudiated", targetMoteId: id })).toContain("repudiated");
    expect(eventSummary({ seq: 1, kind: "effect_staged", moteId: id })).toContain("effect");
  });

  it("a failed row appends the FailureReason label when the wire carried one", () => {
    const s = eventSummary({ seq: 1, kind: "failed", moteId: id, reasonClass: 0 });
    expect(s).toContain("failed");
    expect(s).toContain("TIMED OUT");
  });

  it("a failed row with no reason class shows NO fabricated reason", () => {
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id })).not.toContain("—");
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id, reasonClass: null })).not.toContain(
      "—",
    );
  });

  it("unknown kind falls back to a generic summary", () => {
    expect(eventSummary({ seq: 1, kind: "weird" })).toContain("weird");
  });
});

describe("failureReasonLabel", () => {
  it("maps every journal FailureReason discriminant (0-8)", () => {
    expect(failureReasonLabel(0)).toBe("TIMED OUT");
    expect(failureReasonLabel(1)).toBe("EXECUTOR REFUSED");
    expect(failureReasonLabel(2)).toBe("VALIDATOR REJECTED");
    expect(failureReasonLabel(3)).toBe("WORKER CRASHED");
    expect(failureReasonLabel(4)).toBe("UPSTREAM REPUDIATED");
    expect(failureReasonLabel(5)).toBe("UNSAFE WORLD-MUTATING");
    expect(failureReasonLabel(6)).toBe("COMPENSATED");
    expect(failureReasonLabel(7)).toBe("QUARANTINED");
    expect(failureReasonLabel(8)).toBe("DEAD-LETTERED");
  });

  it("null/undefined → null (no fabricated cause); unknown discriminant → UNKNOWN", () => {
    expect(failureReasonLabel(null)).toBeNull();
    expect(failureReasonLabel(undefined)).toBeNull();
    expect(failureReasonLabel(99)).toBe("UNKNOWN REASON");
  });
});
