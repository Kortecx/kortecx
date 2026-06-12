import type { GlobalDelta } from "@kortecx/sdk/web";
import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { rowEntrance } from "../../app/motion";
import { useGlobalEvents } from "../../kx/use-global-events";
import { useRecipeNames } from "../../kx/use-recipes";
import { eventSummary, eventVisual } from "../../lib/event-format";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";

/**
 * The GLOBAL cross-run live feed (Batch C) — every journal delta on the node,
 * newest-first, each row attributed to its run by the registration watermark.
 * One component, two homes: the Activity drawer's landing (pass `onSelectRun`
 * to drill into the run-scoped panel) and the Monitoring "Live feed" tab
 * (no handler ⇒ rows link to `/workflows/$instanceId`).
 */
export function GlobalFeed({ onSelectRun }: { onSelectRun?: (instanceId: string) => void }) {
  const stream = useGlobalEvents();
  const recipeNames = useRecipeNames();

  if (stream.notWired) {
    return (
      <EmptyState
        title="Live feed unavailable"
        detail="The global event tail needs a Batch-C gateway and a reachable WS bridge — upgrade the serve, or retry."
        action={
          <button
            type="button"
            className="linkbtn"
            onClick={stream.retry}
            data-testid="global-feed-retry"
          >
            Retry
          </button>
        }
      />
    );
  }
  if (stream.events.length === 0) {
    return (
      <EmptyState
        title={stream.active ? "Listening for events…" : "No events yet"}
        detail="Every run on this gateway streams here as it registers and commits — invoke a recipe to see it live."
      />
    );
  }
  return (
    <div className="feed" data-testid="global-feed">
      {stream.dropped ? (
        <p className="muted" data-testid="global-feed-dropped">
          Stream ended —{" "}
          <button type="button" className="linkbtn" onClick={stream.retry}>
            resume the live tail
          </button>
          .
        </p>
      ) : null}
      <ul className="feed__list">
        {stream.events.map((d, i) => (
          <GlobalEventRow
            key={`${d.seq}:${d.kind}:${d.moteId ?? d.instanceId}`}
            delta={d}
            index={i}
            recipeName={d.recipeFingerprint ? recipeNames.data?.[d.recipeFingerprint] : undefined}
            onSelectRun={onSelectRun}
          />
        ))}
      </ul>
    </div>
  );
}

/** One global delta as a feed row: kind pill · summary · run link · seq. */
function GlobalEventRow({
  delta,
  index,
  recipeName,
  onSelectRun,
}: {
  delta: GlobalDelta;
  index: number;
  recipeName?: string;
  onSelectRun?: (instanceId: string) => void;
}) {
  const v = eventVisual(delta.kind);
  return (
    <m.li
      className="event-row"
      data-testid="global-event-row"
      data-kind={delta.kind}
      {...rowEntrance(index)}
    >
      <span className={`pill pill--${v.tone}`}>{v.label}</span>
      <span className="event-row__summary">{eventSummary(delta, recipeName)}</span>
      {delta.instanceId ? (
        onSelectRun ? (
          <button
            type="button"
            className="linkbtn mono"
            onClick={() => onSelectRun(delta.instanceId)}
            data-testid="global-event-run"
            title="Open this run"
          >
            {shortHex(delta.instanceId)}
          </button>
        ) : (
          <Link
            to="/workflows/$instanceId"
            params={{ instanceId: delta.instanceId }}
            className="linkbtn mono"
            data-testid="global-event-run"
            title="Open this run"
          >
            {shortHex(delta.instanceId)}
          </Link>
        )
      ) : null}
      <span className="mono event-row__seq">#{delta.seq}</span>
    </m.li>
  );
}
