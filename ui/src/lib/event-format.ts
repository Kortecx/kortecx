/**
 * Pure display helpers for one event `Delta` (the activity feed's rows). Maps the
 * stable lowercase delta `kind` to a reused state tone + a one-line summary. An
 * unrecognized kind renders as `unknown` rather than crashing — mirrors the
 * `stateVisual` philosophy (the proto is additive-only, new kinds are safe).
 */

import type { StateTone } from "./colors";
import { shortHex } from "./format";

/** The minimal shape of an SDK `Delta` the feed needs (structurally compatible). */
export interface EventLike {
  readonly seq: number;
  readonly kind: string;
  readonly moteId?: string | null;
  readonly resultRef?: string | null;
  readonly targetMoteId?: string | null;
  readonly reasonClass?: number | null;
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
};
const UNKNOWN_VISUAL: EventVisual = { label: "EVENT", tone: "unknown" };

export function eventVisual(kind: string): EventVisual {
  return KIND_VISUAL[kind] ?? UNKNOWN_VISUAL;
}

/** A one-line human summary of a delta (the Mote it concerns + its effect). */
export function eventSummary(d: EventLike): string {
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
    default:
      return `event ${d.kind}`;
  }
}
