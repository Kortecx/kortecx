import { CaptureRecord, ReRankTurn, ReactTurn, ReplanRound } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import {
  rerankPermutationLabel,
  summarizeCaptures,
  summarizeReact,
  summarizeReplan,
  summarizeRerank,
  summarizeRuns,
  tallyRows,
} from "../../src/lib/monitoring";
import type { RunRecord } from "../../src/lib/recent-runs";

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

describe("summarizeRerank", () => {
  it("counts outcomes, models, and enforced reorderings", () => {
    const turns = [
      new ReRankTurn(0, "aa", "i0", "qwen3", "reranked", 3, [2, 0, 1], 30),
      new ReRankTurn(0, "bb", "i0", "qwen3", "pending", 3, [], 28),
      new ReRankTurn(0, "cc", "i1", "gemma3", "failed_closed", 4, [], 26),
    ];
    const s = summarizeRerank(turns);
    expect(s.total).toBe(3);
    expect(s.reranked).toBe(1);
    expect(s.byOutcome.reranked).toBe(1);
    expect(s.byOutcome.pending).toBe(1);
    expect(s.byOutcome.failed_closed).toBe(1);
    expect(s.byModel.qwen3).toBe(2);
    expect(s.byModel.gemma3).toBe(1);
  });

  it("empty → zeroed rollup", () => {
    const s = summarizeRerank([]);
    expect(s.total).toBe(0);
    expect(s.reranked).toBe(0);
    expect(tallyRows(s.byOutcome)).toEqual([]);
  });
});

describe("rerankPermutationLabel", () => {
  it("an empty permutation (non-reranked outcome) → '—'", () => {
    expect(rerankPermutationLabel([])).toBe("—");
  });
  it("a short permutation renders space-joined", () => {
    expect(rerankPermutationLabel([2, 0, 1])).toBe("2 0 1");
  });
  it("a long permutation is elided with its length", () => {
    expect(rerankPermutationLabel([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])).toBe("0 1 2 3 4 5 6 7 … (10)");
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
