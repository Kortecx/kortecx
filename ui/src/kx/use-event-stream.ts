/**
 * Live event tail for one run over the R5 WebSocket bridge (`wsEvents`). A browser
 * cannot speak gRPC server-streaming, so the WS bridge is the only in-browser live
 * surface (the bridge endpoint is the connection's `wsEndpoint`, default :50152).
 *
 * Events accumulate newest-first in a BOUNDED ring buffer (a runaway run can't grow
 * the DOM without limit). On unmount we call the iterator's `.return()` so the
 * generator's `finally` closes the socket immediately — no leaked connection, no
 * post-unmount setState.
 */

import type { Delta } from "@kortecx/sdk/web";
import { useEffect, useState } from "react";
import { useConnection } from "./connection-context";

const DEFAULT_MAX = 500;

export interface EventStreamState {
  /** Deltas newest-first, capped at `max`. */
  readonly events: Delta[];
  /** The stream ended or errored (offer a Refresh). */
  readonly dropped: boolean;
  /** Currently subscribed to the live tail. */
  readonly active: boolean;
}

export function useEventStream(
  instanceId: string | undefined,
  opts: { since?: number; max?: number } = {},
): EventStreamState {
  const { client, status } = useConnection();
  const since = opts.since ?? 0;
  const max = opts.max ?? DEFAULT_MAX;
  const [events, setEvents] = useState<Delta[]>([]);
  const [dropped, setDropped] = useState(false);
  const [active, setActive] = useState(false);

  useEffect(() => {
    if (status !== "connected" || !client || !instanceId) {
      return;
    }
    let cancelled = false;
    setEvents([]);
    setDropped(false);
    setActive(true);

    const iterator = client.wsEvents(instanceId, { since: BigInt(since) })[Symbol.asyncIterator]();

    (async () => {
      try {
        for (;;) {
          const { value, done } = await iterator.next();
          if (done || cancelled) {
            break;
          }
          setEvents((prev) => {
            const next = [value, ...prev];
            return next.length > max ? next.slice(0, max) : next;
          });
        }
      } catch {
        if (!cancelled) {
          setDropped(true);
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
  }, [client, status, instanceId, since, max]);

  return { events, dropped, active };
}
