/**
 * Exhaustive Mote enum → visual mappings, with an UNKNOWN fallback for ANY value
 * (including UNSPECIFIED=0 and future server discriminants). Mirrors the SDK's
 * `stateName` philosophy (bindings/typescript/src/types.ts): never crash, never
 * mislabel a new enum value.
 *
 * The integer values are the FROZEN gateway-proto discriminants
 * (crates/kx-proto/proto/kortecx/v1/{gateway,coordinator}.proto). The proto is
 * additive-only, so an unseen value safely renders as UNKNOWN rather than breaking.
 *
 * `tone` names a CSS custom property family in styles/app.css — themes restyle by
 * editing the variables, never this logic (single source of truth).
 */

export type StateTone =
  | "pending"
  | "scheduled"
  | "committed"
  | "failed"
  | "repudiated"
  | "inconsistent"
  | "unknown";

export type NdTone = "pure" | "read-only-nondet" | "world-mutating" | "unknown";

export interface Visual<T extends string> {
  readonly label: string;
  readonly tone: T;
}

// MoteSnapshotState (gateway.proto:40) — 1..6; 0/UNSPECIFIED + unseen → UNKNOWN.
const STATE: Readonly<Record<number, Visual<StateTone>>> = {
  1: { label: "PENDING", tone: "pending" },
  2: { label: "SCHEDULED", tone: "scheduled" },
  3: { label: "COMMITTED", tone: "committed" },
  4: { label: "FAILED", tone: "failed" },
  5: { label: "REPUDIATED", tone: "repudiated" },
  6: { label: "INCONSISTENT", tone: "inconsistent" },
};
const STATE_UNKNOWN: Visual<StateTone> = { label: "UNKNOWN", tone: "unknown" };

export function stateVisual(code: number): Visual<StateTone> {
  return STATE[code] ?? STATE_UNKNOWN;
}

/**
 * A Mote whose state will not transition further under normal flow
 * (COMMITTED/FAILED/REPUDIATED/INCONSISTENT). PENDING/SCHEDULED are in-flight.
 * Used to stop the projection poll once a run is at rest (see use-projection).
 */
const TERMINAL: ReadonlySet<number> = new Set([3, 4, 5, 6]);
export function isTerminalState(code: number): boolean {
  return TERMINAL.has(code);
}

// NdClass (coordinator.proto) — 1..3; 0/UNSPECIFIED + unseen → UNKNOWN.
const ND: Readonly<Record<number, Visual<NdTone>>> = {
  1: { label: "PURE", tone: "pure" },
  2: { label: "READ_ONLY_NONDET", tone: "read-only-nondet" },
  3: { label: "WORLD_MUTATING", tone: "world-mutating" },
};
const ND_UNKNOWN: Visual<NdTone> = { label: "UNKNOWN", tone: "unknown" };

export function ndClassVisual(code: number): Visual<NdTone> {
  return ND[code] ?? ND_UNKNOWN;
}

// PromotionState (gateway.proto:51).
const PROMOTION: Readonly<Record<number, string>> = {
  1: "NOT_APPLICABLE",
  2: "UNPROMOTED",
  3: "PROMOTED",
};
export function promotionLabel(code: number): string {
  return PROMOTION[code] ?? "UNKNOWN";
}
/** UNSPECIFIED(0)/NOT_APPLICABLE(1) carry no signal worth a badge. */
export function promotionIsNotable(code: number): boolean {
  return code === 2 || code === 3;
}

// MoteAnomaly (gateway.proto:59) — null/absent when healthy.
const ANOMALY: Readonly<Record<number, string>> = {
  1: "EFFECT_STAGED_THEN_REPUDIATED",
  2: "QUARANTINED_AT_LEAST_ONCE_EFFECT",
};
export function anomalyLabel(code: number | null): string | null {
  if (code == null || code === 0) return null;
  return ANOMALY[code] ?? "UNKNOWN_ANOMALY";
}
