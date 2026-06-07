import { describe, expect, it } from "vitest";
import {
  anomalyLabel,
  isTerminalState,
  ndClassVisual,
  promotionIsNotable,
  promotionLabel,
  stateVisual,
} from "../../src/lib/colors";

describe("stateVisual", () => {
  it.each([
    [1, "pending", "PENDING"],
    [2, "scheduled", "SCHEDULED"],
    [3, "committed", "COMMITTED"],
    [4, "failed", "FAILED"],
    [5, "repudiated", "REPUDIATED"],
    [6, "inconsistent", "INCONSISTENT"],
    [0, "unknown", "UNKNOWN"],
    [7, "unknown", "UNKNOWN"],
    [-1, "unknown", "UNKNOWN"],
    [999, "unknown", "UNKNOWN"],
  ])("state %i → tone %s / label %s", (code, tone, label) => {
    const v = stateVisual(code);
    expect(v.tone).toBe(tone);
    expect(v.label).toBe(label);
  });

  it("never returns undefined for ANY integer", () => {
    for (let i = -10; i < 100; i++) {
      const v = stateVisual(i);
      expect(v.tone).toBeTruthy();
      expect(v.label).toBeTruthy();
    }
  });
});

describe("isTerminalState", () => {
  it("COMMITTED/FAILED/REPUDIATED/INCONSISTENT are terminal", () => {
    for (const c of [3, 4, 5, 6]) {
      expect(isTerminalState(c)).toBe(true);
    }
  });
  it("PENDING/SCHEDULED/UNSPECIFIED/unknown are not terminal", () => {
    for (const c of [0, 1, 2, 7, 99]) {
      expect(isTerminalState(c)).toBe(false);
    }
  });
});

describe("ndClassVisual", () => {
  it.each([
    [1, "pure", "PURE"],
    [2, "read-only-nondet", "READ_ONLY_NONDET"],
    [3, "world-mutating", "WORLD_MUTATING"],
    [0, "unknown", "UNKNOWN"],
    [42, "unknown", "UNKNOWN"],
  ])("nd_class %i → %s / %s", (code, tone, label) => {
    const v = ndClassVisual(code);
    expect(v.tone).toBe(tone);
    expect(v.label).toBe(label);
  });
});

describe("promotion", () => {
  it("labels", () => {
    expect(promotionLabel(1)).toBe("NOT_APPLICABLE");
    expect(promotionLabel(2)).toBe("UNPROMOTED");
    expect(promotionLabel(3)).toBe("PROMOTED");
    expect(promotionLabel(0)).toBe("UNKNOWN");
    expect(promotionLabel(99)).toBe("UNKNOWN");
  });
  it("only PROMOTED/UNPROMOTED are notable", () => {
    expect(promotionIsNotable(2)).toBe(true);
    expect(promotionIsNotable(3)).toBe(true);
    expect(promotionIsNotable(0)).toBe(false);
    expect(promotionIsNotable(1)).toBe(false);
  });
});

describe("anomalyLabel", () => {
  it("absent/healthy → null", () => {
    expect(anomalyLabel(null)).toBeNull();
    expect(anomalyLabel(0)).toBeNull();
  });
  it("known anomalies → label", () => {
    expect(anomalyLabel(1)).toBe("EFFECT_STAGED_THEN_REPUDIATED");
    expect(anomalyLabel(2)).toBe("QUARANTINED_AT_LEAST_ONCE_EFFECT");
  });
  it("unseen anomaly → UNKNOWN_ANOMALY (never crash)", () => {
    expect(anomalyLabel(99)).toBe("UNKNOWN_ANOMALY");
  });
});
