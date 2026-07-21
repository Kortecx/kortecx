/**
 * The New App handle-collision check.
 *
 * An App's catalog key is derived from its NAME (`defaultHandle`), and `SaveApp` is an
 * upsert on that key. So two Apps the user thinks of as different — "Tip Calculator" and
 * "tip calculator" — resolve to the same `apps/local/tip-calculator` and the second save
 * silently REPLACES the first: its envelope, its capability rails, its schedule target.
 * The scaffold that follows then authors into the same branch handle too.
 *
 * The form blocks on a hit rather than auto-suffixing. `my-agent-2` leaves the user unable
 * to tell which App is theirs, and nothing later in the runtime distinguishes them.
 *
 * Pure so it is testable — `ui/` has no component-render harness, and this is the part
 * worth pinning. The caller supplies the already-loaded catalog (`useApps`), so the check
 * costs no request.
 */

import { defaultHandle } from "@kortecx/sdk/web";

/** Just enough of an `AppSummary` to check a key against. */
export interface HandleBearing {
  readonly handle: string;
}

/**
 * The handle `name` would claim, or `null` when the name is blank (nothing to claim yet).
 * Mirrors what `builder.save({ client })` derives internally, so the check and the save
 * can never disagree.
 */
export function derivedHandle(name: string): string | null {
  const trimmed = name.trim();
  return trimmed === "" ? null : defaultHandle(trimmed);
}

/**
 * The handle `name` collides with in `apps`, or `null` if it is free. A blank name never
 * collides — the user has not typed anything to collide with yet.
 */
export function collidingHandle(apps: readonly HandleBearing[], name: string): string | null {
  const handle = derivedHandle(name);
  if (handle === null) {
    return null;
  }
  return apps.some((a) => a.handle === handle) ? handle : null;
}
