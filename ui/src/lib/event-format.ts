/**
 * Pure display helpers for one event `Delta` (the activity feed's rows). Maps the
 * stable lowercase delta `kind` to a reused state tone + a one-line summary. An
 * unrecognized kind renders as `unknown` rather than crashing — mirrors the
 * `stateVisual` philosophy (the proto is additive-only, new kinds are safe).
 */

import type { StateTone } from "./colors";
import { shortHex } from "./format";

/** The minimal shape of an SDK `Delta` the feed needs (structurally compatible
 *  with both the per-run `Delta` and the Batch C `GlobalDelta`). */
export interface EventLike {
  readonly seq: number;
  readonly kind: string;
  readonly moteId?: string | null;
  readonly resultRef?: string | null;
  readonly targetMoteId?: string | null;
  readonly reasonClass?: number | null;
  readonly recipeFingerprint?: string | null;
}

export interface EventVisual {
  readonly label: string;
  /** Reuses the Mote-state tone palette so `.pill--<tone>` styling is shared. */
  readonly tone: StateTone;
}

const KIND_VISUAL: Readonly<Record<string, EventVisual>> = {
  committed: { label: "COMMITTED", tone: "committed" },
  failed: { label: "FAILED", tone: "failed" },
  repudiated: { label: "REPUDIATED", tone: "repudiated" },
  effect_staged: { label: "EFFECT STAGED", tone: "scheduled" },
  // Batch C: the global tail surfaces run starts (the per-run cursor never does).
  run_registered: { label: "RUN STARTED", tone: "scheduled" },
};
const UNKNOWN_VISUAL: EventVisual = { label: "EVENT", tone: "unknown" };

export function eventVisual(kind: string): EventVisual {
  return KIND_VISUAL[kind] ?? UNKNOWN_VISUAL;
}

/** A one-line human summary of a delta (the Mote it concerns + its effect).
 *  `recipeName` labels a `run_registered` row when the fingerprint→handle join
 *  resolved one (else the row degrades to the fingerprint hex). */
export function eventSummary(d: EventLike, recipeName?: string): string {
  switch (d.kind) {
    case "committed":
      return `Mote ${shortHex(d.moteId ?? "")} committed${
        d.resultRef ? ` → ${shortHex(d.resultRef)}` : ""
      }`;
    case "failed":
      return `Mote ${shortHex(d.moteId ?? "")} failed`;
    case "repudiated":
      return `Mote ${shortHex(d.targetMoteId ?? "")} repudiated`;
    case "effect_staged":
      return `Mote ${shortHex(d.moteId ?? "")} staged an effect`;
    case "run_registered":
      return recipeName
        ? `Run started — ${recipeName}`
        : `Run started${d.recipeFingerprint ? ` — ${shortHex(d.recipeFingerprint)}` : ""}`;
    default:
      return `event ${d.kind}`;
  }
}
