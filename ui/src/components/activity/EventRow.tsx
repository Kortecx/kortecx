import type { Delta } from "@kortecx/sdk/web";
import { eventSummary, eventVisual } from "../../lib/event-format";

/** One event delta as a feed row (kind pill · summary · seq). Pure presentational. */
export function EventRow({ delta }: { delta: Delta }) {
  const v = eventVisual(delta.kind);
  return (
    <li className="event-row" data-testid="event-row" data-kind={delta.kind}>
      <span className={`pill pill--${v.tone}`}>{v.label}</span>
      <span className="event-row__summary">{eventSummary(delta)}</span>
      <span className="mono event-row__seq">#{delta.seq}</span>
    </li>
  );
}
