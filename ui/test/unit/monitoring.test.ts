import { CaptureRecord, MoteTelemetryRow, ReactTurn, ReplanRound } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import {
  summarizeCaptures,
  summarizeReact,
  summarizeReplan,
  summarizeRuns,
  summarizeTelemetryByModel,
  tallyRows,
  wallClockPercentiles,
} from "../../src/lib/monitoring";
import type { RunRecord } from "../../src/lib/recent-runs";

/** Build a telemetry row with only the fields the rollup reads. */
function tel(
  modelId: string,
  wallClockMs: number,
  outputTokens: number | null = null,
  seq = 0,
): MoteTelemetryRow {
  return new MoteTelemetryRow("m", "", wallClockMs, null, outputTokens, modelId, "", 0, seq);
}

function run(handle: string | null): RunRecord {
  return {
    instanceId: handle ?? "x",
    terminalMoteId: null,
    recipeFingerprint: null,
    handle,
    startedAt: 0,
  };
}

describe("summarizeRuns", () => {
  it("counts runs by handle (null → '—')", () => {
    const r = summarizeRuns([run("kx/recipes/echo"), run("kx/recipes/echo"), run(null)]);
    expect(r.total).toBe(3);
    expect(r.byHandle["kx/recipes/echo"]).toBe(2);
    expect(r.byHandle["—"]).toBe(1);
  });
  it("empty → zeroed rollup", () => {
    const r = summarizeRuns([]);
    expect(r.total).toBe(0);
    expect(tallyRows(r.byHandle)).toEqual([]);
  });
});

describe("summarizeReplan", () => {
  it("rolls up escalations, failed steps, and models", () => {
    const rounds = [
      new ReplanRound(0, "aa", "qwen3", ["s1", "s2"], false, 10),
      new ReplanRound(1, "bb", "qwen3", ["s3"], true, 12),
    ];
    const s = summarizeReplan(rounds);
    expect(s.total).toBe(2);
    expect(s.escalated).toBe(1);
    expect(s.failedStepCount).toBe(3);
    expect(s.byModel.qwen3).toBe(2);
  });
});

describe("summarizeReact", () => {
  it("counts branches, models, and tool calls", () => {
    const turns = [
      new ReactTurn(0, "t0", "i0", "qwen3", "answer", "", "", 8, 6, 20),
      new ReactTurn(1, "t1", "i0", "qwen3", "tool", "fs-read", "1", 8, 6, 22),
      new ReactTurn(2, "t2", "i0", "qwen3", "tool", "fs-read", "1", 8, 6, 24),
    ];
    const s = summarizeReact(turns);
    expect(s.total).toBe(3);
    expect(s.toolCalls).toBe(2);
    expect(s.byBranch.tool).toBe(2);
    expect(s.byBranch.answer).toBe(1);
    expect(s.byModel.qwen3).toBe(3);
  });
});

describe("summarizeCaptures", () => {
  it("counts by nd_class and react branch", () => {
    const recs = [
      new CaptureRecord("m0", "i0", "r0", "PURE", 30, null, ""),
      new CaptureRecord("m1", "i0", "r1", "WORLD_MUTATING", 31, 1, "tool"),
    ];
    const s = summarizeCaptures(recs);
    expect(s.total).toBe(2);
    expect(s.byNdClass.PURE).toBe(1);
    expect(s.byNdClass.WORLD_MUTATING).toBe(1);
    expect(s.byBranch.tool).toBe(1);
    expect(s.byBranch["—"]).toBe(1);
  });
});

describe("tallyRows", () => {
  it("sorts by count desc, then label asc", () => {
    expect(tallyRows({ b: 1, a: 1, c: 3 })).toEqual([
      ["c", 3],
      ["a", 1],
      ["b", 1],
    ]);
  });
});

describe("summarizeTelemetryByModel", () => {
  it("empty window → zeroed rollup", () => {
    expect(summarizeTelemetryByModel([])).toEqual({ windowSize: 0, rows: [] });
  });

  it("groups by model with nearest-rank p50/p95 and summed output tokens", () => {
    // qwen3 walls [10,20,30,40,100]: p50 = rank ceil(.5*5)-1=2 → 30; p95 = ceil(.95*5)-1=4 → 100.
    const s = summarizeTelemetryByModel([
      tel("qwen3", 10, 5),
      tel("qwen3", 20, null),
      tel("qwen3", 30, 7),
      tel("qwen3", 40, 0),
      tel("qwen3", 100, 8),
    ]);
    expect(s.windowSize).toBe(5);
    expect(s.rows).toHaveLength(1);
    expect(s.rows[0]?.modelId).toBe("qwen3");
    expect(s.rows[0]?.count).toBe(5);
    expect(s.rows[0]?.p50WallMs).toBe(30);
    expect(s.rows[0]?.p95WallMs).toBe(100);
    // null contributes 0, never a fabricated count.
    expect(s.rows[0]?.totalOutputTokens).toBe(5 + 7 + 0 + 8);
  });

  it("a single-row window clamps both percentiles to the sole value", () => {
    const s = summarizeTelemetryByModel([tel("m", 42, 3)]);
    expect(s.rows[0]?.p50WallMs).toBe(42);
    expect(s.rows[0]?.p95WallMs).toBe(42);
  });

  it("a two-row window picks defensible nearest-rank values", () => {
    // walls [5,9]: p50 ceil(.5*2)-1=0 → 5; p95 ceil(.95*2)-1=1 → 9.
    const s = summarizeTelemetryByModel([tel("m", 9), tel("m", 5)]);
    expect(s.rows[0]?.p50WallMs).toBe(5);
    expect(s.rows[0]?.p95WallMs).toBe(9);
  });

  it("excludes non-model motes (empty modelId) but counts them in windowSize", () => {
    const s = summarizeTelemetryByModel([tel("", 50), tel("qwen3", 12, 4), tel("", 70)]);
    expect(s.windowSize).toBe(3); // honest: "over the last 3 motes"
    expect(s.rows.map((r) => r.modelId)).toEqual(["qwen3"]);
    expect(s.rows[0]?.count).toBe(1);
  });

  it("sorts rows by count desc, then model id asc", () => {
    const s = summarizeTelemetryByModel([
      tel("bbb", 1),
      tel("aaa", 1),
      tel("ccc", 1),
      tel("ccc", 2),
    ]);
    expect(s.rows.map((r) => r.modelId)).toEqual(["ccc", "aaa", "bbb"]);
  });
});

describe("wallClockPercentiles", () => {
  it("empty → zeroed", () => {
    expect(wallClockPercentiles([])).toEqual({
      count: 0,
      p50WallMs: 0,
      p95WallMs: 0,
      totalOutputTokens: 0,
    });
  });

  it("covers ALL rows (incl. non-model motes) and sums real output tokens only", () => {
    const w = wallClockPercentiles([tel("", 100, null), tel("qwen3", 10, 4), tel("qwen3", 30, 6)]);
    expect(w.count).toBe(3);
    // walls [10,30,100]: p50 → 30, p95 → 100.
    expect(w.p50WallMs).toBe(30);
    expect(w.p95WallMs).toBe(100);
    expect(w.totalOutputTokens).toBe(10);
  });
});
