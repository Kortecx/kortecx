/**
 * Pure display formatters for the teams viewer (no React). Isolated so the
 * caps/role/warrant rendering is unit-testable and the components stay declarative,
 * mirroring the Rust core's module-per-concern discipline.
 */

import type { TeamMember, WarrantView } from "@kortecx/sdk/web";

/** Render a member's catalog action caps as a stable, sorted `·`-joined string. */
export function formatActionCaps(caps: readonly string[]): string {
  return caps.length > 0 ? [...caps].sort().join(" · ") : "—";
}

/** The badge kind for a member's role (a `Delegate`-holding member is a delegate). */
export function roleBadgeKind(member: TeamMember): "delegate" | "member" {
  return member.isDelegate ? "delegate" : "member";
}

/** One labelled row of a resolved-warrant projection. */
export interface WarrantRow {
  readonly label: string;
  readonly value: string;
}

/**
 * The resolved-warrant projection as labelled display rows — the headline ceilings +
 * scopes a member's warrant conveys on the inspected asset (never a secret).
 */
export function warrantRows(w: WarrantView): WarrantRow[] {
  return [
    { label: "Executor", value: w.executorClass },
    { label: "Model route", value: w.modelRoute },
    { label: "Max calls", value: String(w.maxCalls) },
    { label: "Network", value: w.netScope },
    { label: "Filesystem", value: w.fsScope.length > 0 ? w.fsScope : "None" },
    { label: "CPU (milli)", value: String(w.cpuMilli) },
    { label: "Wall clock (ms)", value: String(w.wallClockMs) },
  ];
}
