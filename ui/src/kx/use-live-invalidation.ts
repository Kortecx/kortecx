/**
 * Turn the global event tail into cache invalidation.
 *
 * The WS bridge has always been DISPLAY-ONLY: neither `use-global-events` nor
 * `use-event-stream` imports the query client, so every screen that wanted to
 * reflect a change wrote its own `onSuccess` invalidate — about forty of them —
 * and anything that changed WITHOUT a local mutation (another tab, a cron
 * trigger, a workflow step) simply did not appear until a manual refresh.
 *
 * This hook closes that gap in one place: one subscription, mounted once, that
 * maps an event KIND to the query keys that kind can invalidate.
 *
 * ## Fail-closed on an unknown kind, deliberately
 *
 * An unrecognized kind invalidates **NOTHING**. The tempting alternative —
 * "invalidate everything when we do not recognise it" — turns a single unknown
 * event into a full refetch storm against the gateway, and it does so precisely
 * when the client is oldest and least able to explain why. A kind we do not know
 * is a kind we cannot reason about, so we do not act on it.
 *
 * ## KIND_VISUAL stays the ONE kind enumeration
 *
 * The kinds live in `lib/event-format.ts` and are consumed here through
 * `isKnownEventKind`. A second list in this file would be a second thing to
 * update, and the failure of forgetting it would be invisible: the feed would
 * render the event and the cache would ignore it.
 *
 * ## Scope
 *
 * These are RUN-lifecycle events, so what they invalidate is run-shaped: the run
 * list, that run's projection, and the approvals inbox when an effect stages.
 * Catalog writes (apps, workflows, roles) do not ride this channel at all — they
 * are not journal facts — so their local `onSuccess` invalidates stay correct
 * and are untouched.
 */

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { isKnownEventKind } from "../lib/event-format";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";
import { useGlobalEvents } from "./use-global-events";

/** How many buffered deltas the invalidator needs. It only reads the newest few
 *  since the last tick, so a deep ring buffer would be wasted memory. */
const BUFFER = 32;

/**
 * Subscribe to the global tail and invalidate the queries each event affects.
 *
 * Zero state, no return value: this is a side-effect-only hook, mounted once
 * near the root. Mounting it twice would double every invalidation — harmless
 * but wasteful — which is why it lives at the shell rather than per-screen.
 */
export function useLiveInvalidation(): void {
  const qc = useQueryClient();
  const { endpoint, status } = useConnection();
  const { events } = useGlobalEvents({ max: BUFFER, enabled: status === "connected" });

  // The newest delta we have already acted on. Without this the effect would
  // re-invalidate the whole buffer on every render that appends one event.
  const seen = useRef<string | null>(null);

  useEffect(() => {
    if (status !== "connected" || events.length === 0) {
      return;
    }
    const newest = events[0];
    if (!newest) {
      return;
    }
    // `events` is newest-first. Walk forward only to the point we last handled.
    const fresh: typeof events = [];
    for (const e of events) {
      if (deltaId(e) === seen.current) {
        break;
      }
      fresh.push(e);
    }
    if (fresh.length === 0) {
      return;
    }
    seen.current = deltaId(newest);

    const keys = new Set<string>();
    const instances = new Set<string>();
    let approvalsAffected = false;

    for (const e of fresh) {
      // FAIL-CLOSED: an unknown kind invalidates nothing at all.
      if (!isKnownEventKind(e.kind)) {
        continue;
      }
      // Every known kind is a run-lifecycle fact, so the run list is stale.
      keys.add("runs");
      if (e.instanceId) {
        instances.add(e.instanceId);
      }
      if (e.kind === "effect_staged") {
        approvalsAffected = true;
      }
    }

    if (keys.has("runs")) {
      // The run list is keyed by page size, so match on the prefix rather than
      // guessing which limits are mounted.
      void qc.invalidateQueries({ queryKey: ["kx", endpoint, "runs"] });
    }
    for (const instanceId of instances) {
      // The LIVE key specifically, not the projection prefix: a pinned
      // time-travel snapshot (`atSeq`) is pinned on purpose, and refetching it
      // because the run moved on would be the one thing it exists not to do.
      void qc.invalidateQueries({
        queryKey: queryKeys.projection(endpoint, instanceId),
      });
    }
    if (approvalsAffected) {
      // The approvals inbox polls at 4 s because approvals are NOT on this
      // stream — but `effect_staged` IS the event that creates one, so this is
      // the one case where the tail can beat the poll.
      void qc.invalidateQueries({ queryKey: ["kx", endpoint, "pending-approvals"] });
    }
  }, [events, endpoint, status, qc]);
}

/** A stable identity for a delta, so the effect can tell new from already-seen.
 *  Sequence alone is not unique across runs; the pair is. */
function deltaId(d: {
  readonly kind: string;
  readonly seq?: bigint | number;
  readonly instanceId?: string;
}): string {
  return `${d.instanceId ?? ""}:${d.seq ?? ""}:${d.kind}`;
}
