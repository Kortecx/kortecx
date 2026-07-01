/**
 * Pure aggregations for the Monitoring view — gateway-WIDE telemetry derived only
 * from data already on the wire (`ListRuns`, `ListReplanRounds`, `ListReactTurns`,
 * `ListCaptureRecords`). No React, no I/O, no new RPC: every value is a deterministic
 * function of the SDK view classes, so the whole module is unit-testable and the
 * section stays a thin renderer (the `lib/metrics.ts` precedent).
 */

import type {
  AlertSummary,
  CaptureRecord,
  MoteTelemetryRow,
  ReRankTurn,
  ReactTurn,
  ReplanRound,
} from "@kortecx/sdk/web";
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

/** A rollup of the loaded alerts page (W1a-2). `total` is this-page only (the
 *  inbox is cursor-paged); `errors`/`refusals` split by the wire severity. */
export interface AlertSummaryRollup {
  readonly total: number;
  readonly errors: number;
  readonly refusals: number;
  readonly byReason: Tally;
}

/** Roll up the loaded alerts page by severity + reason — pure, so the section
 *  stays a renderer and the split is unit-testable (no fabricated counts). */
export function summarizeAlerts(alerts: readonly AlertSummary[]): AlertSummaryRollup {
  const byReason: Record<string, number> = {};
  let errors = 0;
  let refusals = 0;
  for (const a of alerts) {
    bump(byReason, a.reasonClass || "—");
    if (a.severity === "refused") {
      refusals += 1;
    } else {
      errors += 1;
    }
  }
  return { total: alerts.length, errors, refusals, byReason };
}

/** Sort a tally into `[label, count]` rows, biggest first (stable for ties by label). */
export function tallyRows(t: Tally): Array<readonly [string, number]> {
  return Object.entries(t).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/** A per-model execution-telemetry rollup row. Wall-clock percentiles are
 *  host-measured (`MoteTelemetryRow.wallClockMs`); `totalOutputTokens` sums only
 *  the real `outputTokens` (model motes on an inference build) — a `null` count
 *  contributes 0, never a fabricated number (GR15). `inputTokens` is NEVER summed
 *  (the OSS backend reports no input count). */
export interface ModelRollupRow {
  readonly modelId: string;
  readonly count: number;
  readonly p50WallMs: number;
  readonly p95WallMs: number;
  readonly totalOutputTokens: number;
}

export interface TelemetryByModel {
  /** Rows the rollup was computed over — the honest "N" in "over the last N motes".
   *  `useTelemetry` is cursor-paged, so this is the LOADED window, not all-time. */
  readonly windowSize: number;
  /** Per-model rows, biggest first (stable for ties by model id). */
  readonly rows: readonly ModelRollupRow[];
}

/** The nearest-rank percentile of an ASCENDING-sorted, non-empty array. `q` in
 *  [0,1]; clamps so q≤0 → first and a length-1 array → its sole element. Chosen
 *  over interpolation as the simplest defensible percentile for small windows. */
function nearestRank(sortedAsc: readonly number[], q: number): number {
  const n = sortedAsc.length;
  if (n === 0) {
    return 0;
  }
  const rank = Math.ceil(q * n) - 1;
  const idx = Math.min(n - 1, Math.max(0, rank));
  return sortedAsc[idx] ?? 0;
}

/**
 * Roll up execution-telemetry rows by the model that ran. Rows with no model
 * (`modelId === ""` — tool/non-model motes) are EXCLUDED so the per-model table
 * stays honest (they would otherwise pollute a synthetic bucket). `windowSize`
 * is the RAW input length (so a caller can truthfully say "over the last N motes",
 * even when every row was a non-model mote). Pure: a deterministic function of the
 * rows, unit-testable, no React/IO — the `summarizeRuns` precedent.
 */
export function summarizeTelemetryByModel(rows: readonly MoteTelemetryRow[]): TelemetryByModel {
  const groups = new Map<string, { walls: number[]; outTokens: number }>();
  for (const r of rows) {
    if (r.modelId === "") {
      continue;
    }
    const g = groups.get(r.modelId) ?? { walls: [], outTokens: 0 };
    g.walls.push(r.wallClockMs);
    g.outTokens += r.outputTokens ?? 0;
    groups.set(r.modelId, g);
  }
  const out: ModelRollupRow[] = [];
  for (const [modelId, g] of groups) {
    const sorted = [...g.walls].sort((a, b) => a - b);
    out.push({
      modelId,
      count: sorted.length,
      p50WallMs: nearestRank(sorted, 0.5),
      p95WallMs: nearestRank(sorted, 0.95),
      totalOutputTokens: g.outTokens,
    });
  }
  out.sort((a, b) => b.count - a.count || a.modelId.localeCompare(b.modelId));
  return { windowSize: rows.length, rows: out };
}

export interface WallStats {
  /** Rows the stats cover (the loaded telemetry window). */
  readonly count: number;
  readonly p50WallMs: number;
  readonly p95WallMs: number;
  /** Σ of the real `outputTokens` in the window (null → 0; never fabricated). */
  readonly totalOutputTokens: number;
}

/** Window-wide wall-clock percentiles + output-token total over ALL telemetry rows
 *  (model and non-model motes — wall time is host-measured for every commit). The
 *  honest aggregate for a "latency / tokens, last N motes" KPI. Pure. */
export function wallClockPercentiles(rows: readonly MoteTelemetryRow[]): WallStats {
  const sorted = rows.map((r) => r.wallClockMs).sort((a, b) => a - b);
  let totalOutputTokens = 0;
  for (const r of rows) {
    totalOutputTokens += r.outputTokens ?? 0;
  }
  return {
    count: sorted.length,
    p50WallMs: nearestRank(sorted, 0.5),
    p95WallMs: nearestRank(sorted, 0.95),
    totalOutputTokens,
  };
}
