/**
 * Pure aggregations for the Monitoring view — gateway-WIDE telemetry derived only
 * from data already on the wire (`ListRuns`, `ListReplanRounds`, `ListReactTurns`,
 * `ListCaptureRecords`). No React, no I/O, no new RPC: every value is a deterministic
 * function of the SDK view classes, so the whole module is unit-testable and the
 * section stays a thin renderer (the `lib/metrics.ts` precedent).
 */

import type { CaptureRecord, ReRankTurn, ReactTurn, ReplanRound } from "@kortecx/sdk/web";
import type { RunRecord } from "./recent-runs";

/** A counter keyed by a string label (model id, branch, nd_class, handle). */
export type Tally = Readonly<Record<string, number>>;

function bump(into: Record<string, number>, key: string): void {
  into[key] = (into[key] ?? 0) + 1;
}

export interface RunRollup {
  readonly total: number;
  /** Runs per invoked blueprint handle (unknown handle → "—"). */
  readonly byHandle: Tally;
}

export function summarizeRuns(runs: readonly RunRecord[]): RunRollup {
  const byHandle: Record<string, number> = {};
  for (const r of runs) {
    bump(byHandle, r.handle ?? "—");
  }
  return { total: runs.length, byHandle };
}

export interface ReplanSummary {
  readonly total: number;
  readonly escalated: number;
  readonly failedStepCount: number;
  readonly byModel: Tally;
}

export function summarizeReplan(rounds: readonly ReplanRound[]): ReplanSummary {
  const byModel: Record<string, number> = {};
  let escalated = 0;
  let failedStepCount = 0;
  for (const r of rounds) {
    bump(byModel, r.modelId || "—");
    if (r.escalated) {
      escalated += 1;
    }
    failedStepCount += r.failedStepIds.length;
  }
  return { total: rounds.length, escalated, failedStepCount, byModel };
}

export interface ReactSummary {
  readonly total: number;
  /** Turns per settled branch (pending | answer | tool | dead_lettered). */
  readonly byBranch: Tally;
  readonly byModel: Tally;
  /** Turns that fired a tool (branch === "tool"). */
  readonly toolCalls: number;
}

export function summarizeReact(turns: readonly ReactTurn[]): ReactSummary {
  const byBranch: Record<string, number> = {};
  const byModel: Record<string, number> = {};
  let toolCalls = 0;
  for (const t of turns) {
    bump(byBranch, t.branch || "—");
    bump(byModel, t.modelId || "—");
    if (t.branch === "tool") {
      toolCalls += 1;
    }
  }
  return { total: turns.length, byBranch, byModel, toolCalls };
}

export interface RerankSummary {
  readonly total: number;
  /** Turns that produced an enforced reordering (`outcome === "reranked"`). */
  readonly reranked: number;
  /** Turns per settled outcome (pending | reranked | failed_closed). */
  readonly byOutcome: Tally;
  readonly byModel: Tally;
}

export function summarizeRerank(turns: readonly ReRankTurn[]): RerankSummary {
  const byOutcome: Record<string, number> = {};
  const byModel: Record<string, number> = {};
  let reranked = 0;
  for (const t of turns) {
    bump(byOutcome, t.outcome || "—");
    bump(byModel, t.modelId || "—");
    if (t.outcome === "reranked") {
      reranked += 1;
    }
  }
  return { total: turns.length, reranked, byOutcome, byModel };
}

/** A compact, audit-honest rendering of an enforced re-rank permutation (the
 *  reordered SOURCE indices; SN-8: an exact reordering, never a score). Empty (a
 *  non-`reranked` outcome) → "—"; a long permutation is elided with its length so
 *  the trail row stays single-line. Pure — a deterministic function of the input. */
export function rerankPermutationLabel(permutation: readonly number[]): string {
  if (permutation.length === 0) {
    return "—";
  }
  const MAX = 8;
  if (permutation.length <= MAX) {
    return permutation.join(" ");
  }
  return `${permutation.slice(0, MAX).join(" ")} … (${permutation.length})`;
}

export interface CaptureSummary {
  readonly total: number;
  readonly byNdClass: Tally;
  readonly byBranch: Tally;
}

export function summarizeCaptures(records: readonly CaptureRecord[]): CaptureSummary {
  const byNdClass: Record<string, number> = {};
  const byBranch: Record<string, number> = {};
  for (const r of records) {
    bump(byNdClass, r.ndClass || "—");
    bump(byBranch, r.reactBranch || "—");
  }
  return { total: records.length, byNdClass, byBranch };
}

/** Sort a tally into `[label, count]` rows, biggest first (stable for ties by label). */
export function tallyRows(t: Tally): Array<readonly [string, number]> {
  return Object.entries(t).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}
