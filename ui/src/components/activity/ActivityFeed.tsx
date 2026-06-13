import type { Delta } from "@kortecx/sdk/web";
import { useMemo } from "react";
import { useResultMap } from "../../kx/use-content-batch";
import { EmptyState } from "../EmptyState";
import { EventRow } from "./EventRow";

/**
 * The live event list (newest-first). Handles empty, listening, and dropped tails.
 * Committed rows resolve their result TEXT in one batch round trip (the N+1
 * collapse, run-scoped) so the feed shows outputs, not hashes (D142.2). When
 * `instanceId` is absent the rows degrade to the summary (no resolution).
 */
export function ActivityFeed({
  events,
  dropped,
  active,
  instanceId,
}: {
  events: Delta[];
  dropped: boolean;
  active: boolean;
  instanceId?: string;
}) {
  // Resolve the visible committed deltas' results (bounded to what's on screen).
  // No instance scope ⇒ no fetch (the query stays disabled on an empty ref set).
  const refs = useMemo(
    () =>
      instanceId
        ? events.flatMap((d) => (d.kind === "committed" && d.resultRef ? [d.resultRef] : []))
        : [],
    [events, instanceId],
  );
  const { byRef, isLoading } = useResultMap(instanceId ?? "", refs);

  if (events.length === 0) {
    return (
      <EmptyState
        title={active ? "Listening for events…" : "No events yet"}
        detail="Events appear here as the run commits, fails, or repudiates Motes."
      />
    );
  }
  return (
    <div className="feed" data-testid="activity-feed">
      {dropped ? (
        <p className="muted" data-testid="feed-dropped">
          Stream ended — Refresh to resume the live tail.
        </p>
      ) : null}
      <ul className="feed__list">
        {events.map((d, i) => {
          const vm = d.resultRef ? byRef.get(d.resultRef) : undefined;
          return (
            <EventRow
              key={`${d.seq}:${d.kind}:${d.moteId ?? d.targetMoteId ?? ""}`}
              delta={d}
              content={vm?.content}
              missing={vm?.missing ?? false}
              resolving={isLoading}
              index={i}
            />
          );
        })}
      </ul>
    </div>
  );
}
