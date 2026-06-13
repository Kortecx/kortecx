import type { GlobalDelta } from "@kortecx/sdk/web";
import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useMemo } from "react";
import { rowEntrance } from "../../app/motion";
import { type RunScopedRef, useResultMapMulti } from "../../kx/use-content-batch";
import { useGlobalEvents } from "../../kx/use-global-events";
import { useRecipeNames } from "../../kx/use-recipes";
import type { DecodedContent } from "../../lib/content-decode";
import { eventSummary, eventVisual } from "../../lib/event-format";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ResultPreview } from "../ResultPreview";

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
  // Resolve the visible committed deltas' results, grouped by their owning run
  // (GetContentBatch is run-scoped — one batch per distinct run in the window).
  const pairs = useMemo<RunScopedRef[]>(
    () =>
      stream.events.flatMap((d) =>
        d.kind === "committed" && d.resultRef && d.instanceId
          ? [{ instanceId: d.instanceId, ref: d.resultRef }]
          : [],
      ),
    [stream.events],
  );
  const { byRef, isLoading } = useResultMapMulti(pairs);

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
        {stream.events.map((d, i) => {
          const vm = d.resultRef ? byRef.get(d.resultRef) : undefined;
          return (
            <GlobalEventRow
              key={`${d.seq}:${d.kind}:${d.moteId ?? d.instanceId}`}
              delta={d}
              index={i}
              recipeName={d.recipeFingerprint ? recipeNames.data?.[d.recipeFingerprint] : undefined}
              onSelectRun={onSelectRun}
              content={vm?.content}
              missing={vm?.missing ?? false}
              resolving={isLoading}
            />
          );
        })}
      </ul>
    </div>
  );
}

/** One global delta as a feed row: kind pill · summary · resolved result · run
 *  link · seq. A committed row shows the result TEXT (D142.2); the run link is a
 *  sibling of the chip (both inside the row, never nested). */
function GlobalEventRow({
  delta,
  index,
  recipeName,
  onSelectRun,
  content,
  missing = false,
  resolving = false,
}: {
  delta: GlobalDelta;
  index: number;
  recipeName?: string;
  onSelectRun?: (instanceId: string) => void;
  content?: DecodedContent;
  missing?: boolean;
  resolving?: boolean;
}) {
  const v = eventVisual(delta.kind);
  const showResult = delta.kind === "committed" && Boolean(delta.resultRef);
  return (
    <m.li
      className="event-row"
      data-testid="global-event-row"
      data-kind={delta.kind}
      {...rowEntrance(index)}
    >
      <span className={`pill pill--${v.tone}`}>{v.label}</span>
      <span className="event-row__summary">{eventSummary(delta, recipeName, showResult)}</span>
      {showResult ? (
        <ResultPreview
          resultRef={delta.resultRef ?? null}
          content={content}
          missing={missing}
          loading={resolving}
          max={80}
        />
      ) : null}
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
