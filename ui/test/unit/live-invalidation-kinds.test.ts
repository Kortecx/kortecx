/**
 * The fail-closed primitive behind `use-live-invalidation`.
 *
 * The hook ACTS on events — it invalidates cache — so it cannot be permissive
 * about kinds the way rendering can. `eventVisual` falls through to a generic
 * row for an unknown kind, which is honest; `isKnownEventKind` must say NO, so
 * an unrecognized event invalidates nothing rather than triggering a
 * refetch-everything storm exactly when the client is oldest.
 *
 * These two functions read the SAME table on purpose. The test pins that: a
 * second list would be a second thing to update, and forgetting it would be
 * invisible — the feed would render the event while the cache ignored it.
 */

import { describe, expect, it } from "vitest";
import { FEED_KINDS, eventVisual, isKnownEventKind } from "../../src/lib/event-format";

describe("isKnownEventKind", () => {
  it("accepts every kind the feed enumerates", () => {
    // FEED_KINDS is the display order of the same table. If a kind can be shown
    // it must be actionable, or the two halves have drifted.
    expect(FEED_KINDS.length).toBeGreaterThan(0);
    for (const kind of FEED_KINDS) {
      expect(isKnownEventKind(kind), `${kind} should be known`).toBe(true);
    }
  });

  it("REFUSES an unknown kind, so the hook invalidates nothing", () => {
    for (const kind of ["", "telepathy", "committed_v2", "COMMITTED", "run_registered "]) {
      expect(isKnownEventKind(kind), `${kind} should be unknown`).toBe(false);
    }
  });

  it("does not accept inherited Object properties", () => {
    // A plain `KIND_VISUAL[kind] !== undefined` check would answer TRUE for
    // "constructor" / "toString" and let a crafted kind through. `Object.hasOwn`
    // is what makes that impossible.
    for (const kind of ["constructor", "toString", "__proto__", "hasOwnProperty"]) {
      expect(isKnownEventKind(kind), `${kind} must not be treated as an event`).toBe(false);
    }
  });

  it("agrees with eventVisual about which kinds are real", () => {
    for (const kind of FEED_KINDS) {
      // A known kind has a real label, not the unknown fallback.
      expect(eventVisual(kind).label).not.toBe("EVENT");
    }
    // And the fallback is still reachable — otherwise the assertion above is
    // satisfied by every kind having a label, which proves nothing.
    expect(eventVisual("telepathy").label).toBe("EVENT");
  });
});
