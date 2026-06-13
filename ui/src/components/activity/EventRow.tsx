import type { Delta } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { rowEntrance } from "../../app/motion";
import type { DecodedContent } from "../../lib/content-decode";
import { eventSummary, eventVisual } from "../../lib/event-format";
import { ResultPreview } from "../ResultPreview";

/**
 * One event delta as a feed row (kind pill · summary · resolved result · seq).
 * A `committed` row shows the RESOLVED result text as the headline (D142.2) when
 * its content is supplied by the feed's batch resolve, with the digest as a
 * trailing chip — so the feed reads as outputs, not hashes. Pure presentational.
 */
export function EventRow({
  delta,
  content,
  missing = false,
  resolving = false,
  index = 0,
}: {
  delta: Delta;
  /** The resolved committed result (from the feed's batch fetch); undefined while resolving. */
  content?: DecodedContent;
  missing?: boolean;
  resolving?: boolean;
  index?: number;
}) {
  const v = eventVisual(delta.kind);
  const showResult = delta.kind === "committed" && Boolean(delta.resultRef);
  return (
    <m.li
      className="event-row"
      data-testid="event-row"
      data-kind={delta.kind}
      {...rowEntrance(index)}
    >
      <span className={`pill pill--${v.tone}`}>{v.label}</span>
      <span className="event-row__summary">{eventSummary(delta, undefined, showResult)}</span>
      {showResult ? (
        <ResultPreview
          resultRef={delta.resultRef ?? null}
          content={content}
          missing={missing}
          loading={resolving}
          max={80}
        />
      ) : null}
      <span className="mono event-row__seq">#{delta.seq}</span>
    </m.li>
  );
}
