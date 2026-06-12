/**
 * The GLOBAL cross-run live tail over the WS bridge's `/events/all` channel
 * (Batch C `wsAllEvents`) — the `useEventStream` twin without a run scope. Every
 * delta carries its watermark-attributed `instanceId` (empty before any run
 * registers), so the feed can link each row to its run.
 *
 * Same ring-buffer + teardown discipline as the per-run hook. One extra state:
 * `notWired` — the stream failed before ANY frame arrived, which is what an
 * OLDER gateway looks like (its WS bridge 400s the `/events/all` path). It can
 * also be a plain connection failure, so the copy stays honest ("unavailable",
 * offer a retry) rather than claiming the gateway is old.
 */

import type { GlobalDelta } from "@kortecx/sdk/web";
import { useCallback, useEffect, useState } from "react";
import { useConnection } from "./connection-context";

const DEFAULT_MAX = 500;

export interface GlobalEventStreamState {
  /** Deltas newest-first, capped at `max`. */
  readonly events: GlobalDelta[];
  /** The stream ended or errored after frames arrived (offer a Refresh). */
  readonly dropped: boolean;
  /** Currently subscribed to the live tail. */
  readonly active: boolean;
  /** The stream failed before any frame — an older gateway or a dead bridge. */
  readonly notWired: boolean;
  /** Tear down + resubscribe (the Refresh affordance). */
  readonly retry: () => void;
}

export function useGlobalEvents(
  opts: { since?: number; max?: number; enabled?: boolean } = {},
): GlobalEventStreamState {
  const { client, status } = useConnection();
  const since = opts.since ?? 0;
  const max = opts.max ?? DEFAULT_MAX;
  const enabled = opts.enabled ?? true;
  const [events, setEvents] = useState<GlobalDelta[]>([]);
  const [dropped, setDropped] = useState(false);
  const [active, setActive] = useState(false);
  const [notWired, setNotWired] = useState(false);
  const [epoch, setEpoch] = useState(0);

  const retry = useCallback(() => setEpoch((e) => e + 1), []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `epoch` is the deliberate resubscribe trigger (the retry affordance) — not read inside, only forcing the effect to re-run.
  useEffect(() => {
    if (status !== "connected" || !client || !enabled) {
      return;
    }
    let cancelled = false;
    let sawFrame = false;
    setEvents([]);
    setDropped(false);
    setNotWired(false);
    setActive(true);

    const iterator = client.wsAllEvents({ since: BigInt(since) })[Symbol.asyncIterator]();

    (async () => {
      try {
        for (;;) {
          const { value, done } = await iterator.next();
          if (done || cancelled) {
            break;
          }
          sawFrame = true;
          setEvents((prev) => {
            const next = [value, ...prev];
            return next.length > max ? next.slice(0, max) : next;
          });
        }
      } catch {
        if (!cancelled) {
          if (sawFrame) {
            setDropped(true);
          } else {
            setNotWired(true);
          }
        }
      } finally {
        if (!cancelled) {
          setActive(false);
        }
      }
    })();

    return () => {
      cancelled = true;
      // Resume the suspended generator at its `finally` → close the socket now.
      void iterator.return?.(undefined);
    };
  }, [client, status, since, max, enabled, epoch]);

  return { events, dropped, active, notWired, retry };
}
