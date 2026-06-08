/**
 * Pure metric derivation from a folded projection (or several concurrent runs).
 * No React, no I/O — every value is a deterministic function of the `ProjectionVM`,
 * so the whole module is unit-testable and the Metrics panel stays a thin renderer.
 *
 * NOTE on latency: the projection carries NO wall-clock timestamps — only the
 * journal `committed_seq`. So `latencySeqSpan` is a SEQ-distance proxy (the spread
 * of commit positions), explicitly NOT milliseconds. Real ms-latency needs the
 * gateway to emit timestamps (a flagged, additive roadmap change — see UI-1 plan).
 */

import type { MoteVM, ProjectionVM } from "../kx/use-projection";
import { type StateTone, isTerminalState, stateVisual } from "./colors";

// The frozen MoteSnapshotState discriminants we name explicitly (mirrors
// ProjectionSummary's `COMMITTED = 3`); everything else is bucketed by tone.
const COMMITTED = 3;
const FAILED = 4;

export interface Metrics {
  /** Total Motes across the folded run(s). */
  readonly total: number;
  /** Count per state tone (single source: `stateVisual`). */
  readonly byState: Readonly<Record<StateTone, number>>;
  readonly committed: number;
  readonly failed: number;
  /** Motes in a terminal state (committed/failed/repudiated/inconsistent). */
  readonly terminal: number;
  /** Motes still in flight (pending/scheduled). */
  readonly inFlight: number;
  /** committed / terminal (0 when nothing is terminal — no divide-by-zero). */
  readonly successRate: number;
  /** failed / terminal (0 when nothing is terminal). */
  readonly failureRate: number;
  /** max(committed_seq) − min(committed_seq) over committed Motes (null if < 2). */
  readonly latencySeqSpan: number | null;
  /** The journal frontier of the folded run(s). */
  readonly currentSeq: number;
}

function emptyByState(): Record<StateTone, number> {
  return {
    pending: 0,
    scheduled: 0,
    committed: 0,
    failed: 0,
    repudiated: 0,
    inconsistent: 0,
    unknown: 0,
  };
}

function fold(motes: readonly MoteVM[], currentSeq: number): Metrics {
  const byState = emptyByState();
  let committed = 0;
  let failed = 0;
  let terminal = 0;
  let inFlight = 0;
  let minSeq: number | null = null;
  let maxSeq: number | null = null;
  let committedWithSeq = 0;

  for (const m of motes) {
    byState[stateVisual(m.stateCode).tone] += 1;
    if (isTerminalState(m.stateCode)) {
      terminal += 1;
    } else {
      inFlight += 1;
    }
    if (m.stateCode === COMMITTED) {
      committed += 1;
      if (m.committedSeq != null) {
        committedWithSeq += 1;
        minSeq = minSeq === null ? m.committedSeq : Math.min(minSeq, m.committedSeq);
        maxSeq = maxSeq === null ? m.committedSeq : Math.max(maxSeq, m.committedSeq);
      }
    } else if (m.stateCode === FAILED) {
      failed += 1;
    }
  }

  return {
    total: motes.length,
    byState,
    committed,
    failed,
    terminal,
    inFlight,
    successRate: terminal > 0 ? committed / terminal : 0,
    failureRate: terminal > 0 ? failed / terminal : 0,
    latencySeqSpan:
      committedWithSeq >= 2 && minSeq !== null && maxSeq !== null ? maxSeq - minSeq : null,
    currentSeq,
  };
}

/** Metrics for one run's projection. */
export function deriveMetrics(p: ProjectionVM): Metrics {
  return fold(p.motes, p.currentSeq);
}

/** Aggregate metrics across several concurrent runs (frontier = the max seq). */
export function foldMetrics(ps: readonly ProjectionVM[]): Metrics {
  const motes = ps.flatMap((p) => p.motes);
  const currentSeq = ps.reduce((mx, p) => Math.max(mx, p.currentSeq), 0);
  return fold(motes, currentSeq);
}

/** A whole-number percentage for display (e.g. 0.6667 → "67%"). */
export function asPercent(rate: number): string {
  return `${Math.round(rate * 100)}%`;
}
