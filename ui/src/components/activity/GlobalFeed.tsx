import type { GlobalDelta } from "@kortecx/sdk/web";
import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useMemo, useState } from "react";
import { rowEntrance } from "../../app/motion";
import { type RunScopedRef, useResultMapMulti } from "../../kx/use-content-batch";
import { useGlobalEvents } from "../../kx/use-global-events";
import { useRecipeNames } from "../../kx/use-recipes";
import type { DecodedContent } from "../../lib/content-decode";
import { download } from "../../lib/download";
import {
  type EventLike,
  FEED_KINDS,
  eventSummary,
  eventVisual,
  exportFeedFilename,
  failureReasonLabel,
  feedToNdjson,
  matchesFeedFilter,
  tallyEventsByKind,
} from "../../lib/event-format";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ResultPreview } from "../ResultPreview";

/**
 * The GLOBAL cross-run live feed (Batch C) — every journal delta on the node,
 * newest-first, each row attributed to its run by the registration watermark.
 * One component, two homes: the Activity drawer's landing (pass `onSelectRun`
 * to drill into the run-scoped panel) and the Monitoring "Live feed" tab
 * (no handler ⇒ rows link to `/workflows/$instanceId`).
 *
 * W1a-3 adds OPT-IN triage affordances: kind toggle chips with per-kind count
 * badges, a run-id/free-text filter, and an NDJSON export of the (filtered)
 * buffer. All client-side over the buffered deltas; chips default all-on so the
 * default rendering is unchanged.
 */
export function GlobalFeed({ onSelectRun }: { onSelectRun?: (instanceId: string) => void }) {
  const stream = useGlobalEvents();
  const recipeNames = useRecipeNames();
  // Triage filter state (client-side over the buffer). `enabled` starts with
  // every kind on; an empty query shows everything — the default is unchanged.
  const [enabled, setEnabled] = useState<ReadonlySet<string>>(() => new Set(FEED_KINDS));
  const [query, setQuery] = useState("");

  const tally = useMemo(() => tallyEventsByKind(stream.events), [stream.events]);
  // All kinds on ⇒ `null` (also shows future/unknown kinds); any off ⇒ the set.
  const filter = useMemo(() => {
    const allOn = FEED_KINDS.every((k) => enabled.has(k));
    return { kinds: allOn ? null : enabled, query };
  }, [enabled, query]);
  const names = recipeNames.data;
  const recipeName = (d: EventLike) =>
    d.recipeFingerprint ? names?.[d.recipeFingerprint] : undefined;
  const visible = useMemo(
    () =>
      stream.events.filter((d) =>
        matchesFeedFilter(
          d,
          filter,
          d.recipeFingerprint ? names?.[d.recipeFingerprint] : undefined,
        ),
      ),
    [stream.events, filter, names],
  );

  // Resolve the visible committed deltas' results, grouped by their owning run
  // (GetContentBatch is run-scoped — one batch per distinct run in the window).
  const pairs = useMemo<RunScopedRef[]>(
    () =>
      visible.flatMap((d) =>
        d.kind === "committed" && d.resultRef && d.instanceId
          ? [{ instanceId: d.instanceId, ref: d.resultRef }]
          : [],
      ),
    [visible],
  );
  const { byRef, isLoading } = useResultMapMulti(pairs);

  const toggleKind = (kind: string) =>
    setEnabled((prev) => {
      const next = new Set(prev);
      if (next.has(kind)) {
        next.delete(kind);
      } else {
        next.add(kind);
      }
      return next;
    });

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
      <div className="feed__toolbar" data-testid="feed-toolbar">
        <fieldset className="feed__chips" aria-label="Filter events by kind">
          {FEED_KINDS.map((kind) => {
            const v = eventVisual(kind);
            const on = enabled.has(kind);
            const count = tally[kind] ?? 0;
            return (
              <button
                type="button"
                key={kind}
                className={`chip chip--toggle${on ? " chip--on" : ""}`}
                aria-pressed={on}
                onClick={() => toggleKind(kind)}
                data-testid={`feed-chip-${kind}`}
                title={`${on ? "Hide" : "Show"} ${v.label.toLowerCase()} events`}
              >
                <span className={`chip__dot chip__dot--${v.tone}`} aria-hidden="true" />
                {v.label}
                <span className="chip__count" data-testid={`feed-count-${kind}`}>
                  {count}
                </span>
              </button>
            );
          })}
        </fieldset>
        <input
          type="text"
          className="feed__filter"
          placeholder="Filter by run id, mote, or reason…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Filter events by run id or text"
          data-testid="feed-filter"
        />
        <button
          type="button"
          className="linkbtn"
          onClick={() =>
            download(exportFeedFilename(), feedToNdjson(visible), "application/x-ndjson")
          }
          disabled={visible.length === 0}
          data-testid="feed-export"
          title="Export the filtered feed as NDJSON"
        >
          Export ({visible.length})
        </button>
      </div>
      {stream.dropped ? (
        <p className="muted" data-testid="global-feed-dropped">
          Stream ended —{" "}
          <button type="button" className="linkbtn" onClick={stream.retry}>
            resume the live tail
          </button>
          .
        </p>
      ) : null}
      {visible.length === 0 ? (
        <p className="muted" data-testid="feed-empty-filtered">
          No events match the current filter.
        </p>
      ) : (
        <ul className="feed__list">
          {visible.map((d, i) => {
            const vm = d.resultRef ? byRef.get(d.resultRef) : undefined;
            return (
              <GlobalEventRow
                key={`${d.seq}:${d.kind}:${d.moteId ?? d.instanceId}`}
                delta={d}
                index={i}
                recipeName={recipeName(d)}
                onSelectRun={onSelectRun}
                content={vm?.content}
                missing={vm?.missing ?? false}
                resolving={isLoading}
              />
            );
          })}
        </ul>
      )}
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
  // Elevate the failure reason to a visible, scannable badge (W1a-3) — and omit
  // it from the prose summary so it never shows twice.
  const reason = delta.kind === "failed" ? failureReasonLabel(delta.reasonClass) : null;
  return (
    <m.li
      className="event-row"
      data-testid="global-event-row"
      data-kind={delta.kind}
      {...rowEntrance(index)}
    >
      <span className={`pill pill--${v.tone}`}>{v.label}</span>
      {reason ? (
        <span className="badge badge--failed" data-testid="event-reason-badge">
          {reason}
        </span>
      ) : null}
      <span className="event-row__summary">
        {eventSummary(delta, recipeName, showResult, true)}
      </span>
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
