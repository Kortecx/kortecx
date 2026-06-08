/**
 * Pure display formatters for the grants inspector (no React). Isolated so the
 * action/status rendering is unit-testable, mirroring the Rust core's
 * module-per-concern discipline.
 */

import type { GrantView } from "@kortecx/sdk/web";

/** Render a grant's catalog actions as a `·`-joined string (`"—"` if none). */
export function formatActions(actions: readonly string[]): string {
  return actions.length > 0 ? actions.join(" · ") : "—";
}

/** A human-readable status label for a grant row. */
export function grantStatusLabel(grant: GrantView): string {
  switch (grant.status) {
    case "revoked":
      return "Revoked";
    case "root":
      return "Root";
    default:
      return "Delegated";
  }
}
