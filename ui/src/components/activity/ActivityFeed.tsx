import type { Delta } from "@kortecx/sdk/web";
import { EmptyState } from "../EmptyState";
import { EventRow } from "./EventRow";

/** The live event list (newest-first). Handles empty, listening, and dropped tails. */
export function ActivityFeed({
  events,
  dropped,
  active,
}: {
  events: Delta[];
  dropped: boolean;
  active: boolean;
}) {
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
        {events.map((d, i) => (
          <EventRow
            key={`${d.seq}:${d.kind}:${d.moteId ?? d.targetMoteId ?? ""}`}
            delta={d}
            index={i}
          />
        ))}
      </ul>
    </div>
  );
}
