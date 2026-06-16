import { m } from "framer-motion";
import { type FormEvent, useState } from "react";
import { fadeUp, rowEntrance } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDatasetQuery } from "../../kx/use-datasets";
import { useFuzzyDiscovery } from "../../kx/use-fuzzy-discovery";
import { decodeContent } from "../../lib/content-decode";
import { AssetViewer } from "../AssetViewer";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { GlowCard } from "../ds/GlowCard";
import { EmbedderNotice, isNoEmbedder } from "./EmbedderNotice";

/** The server-side top-k cap (`MAX_K` in the host); the slider matches it. */
const MAX_K = 64;

type Mode = "search" | "discover";

/** A short, single-line preview of a hit's text. */
function snippet(text: string, max = 140): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max)}…` : flat;
}

/**
 * Semantic search over the selected dataset. Two modes (a segmented toggle):
 *  - **Search** (`QueryDataset`) returns hits WITH their document bytes — click a
 *    hit to render it inline through the shared {@link AssetViewer} (text / JSON /
 *    markdown / image / video / audio).
 *  - **Discover** (`FuzzyDiscovery`, Slice-B) is the advisory fuzzy-in / exact-out
 *    primitive: it returns ONLY content-addressed refs + a DISPLAY-ONLY score —
 *    resolve bytes by the EXACT ref (the SDK / programmatic path). No content is
 *    shown here, honestly (the refs are the result; SN-8).
 *
 * The query text is embedded server-side (the `inference` feature); without an
 * embedder the gateway returns FAILED_PRECONDITION and the panel shows the
 * {@link EmbedderNotice}. Every `score` is DISPLAY-only (SN-8) — a ranking aid,
 * never an identity input.
 */
export function QueryPanel({ dataset }: { dataset: string | null }) {
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [k, setK] = useState(10);
  const [mode, setMode] = useState<Mode>("search");
  const [openRef, setOpenRef] = useState<string | null>(null);

  // Only the active mode's query runs (the other is disabled via an undefined dataset).
  const searchDs = mode === "search" ? (dataset ?? undefined) : undefined;
  const discoverDs = mode === "discover" ? (dataset ?? undefined) : undefined;
  const hits = useDatasetQuery(searchDs, query, k);
  const fuzzy = useFuzzyDiscovery(discoverDs, query, k);
  const active = mode === "search" ? hits : fuzzy;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setQuery(draft.trim());
    setOpenRef(null);
  };

  const setModeAndReset = (next: Mode) => {
    setMode(next);
    setOpenRef(null);
  };

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="dataset-query-panel">
      <h2>Search</h2>
      <fieldset className="view-toggle" aria-label="Search mode" data-testid="dataset-mode">
        <button
          type="button"
          data-testid="dataset-mode-search"
          aria-pressed={mode === "search"}
          onClick={() => setModeAndReset("search")}
        >
          Search
        </button>
        <button
          type="button"
          data-testid="dataset-mode-discover"
          aria-pressed={mode === "discover"}
          onClick={() => setModeAndReset("discover")}
        >
          Discover
        </button>
      </fieldset>
      <form onSubmit={onSubmit} className="dataset-query-form">
        <input
          type="text"
          data-testid="dataset-query-input"
          placeholder={dataset ? `Search ${dataset}…` : "Select a dataset first"}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          disabled={!dataset}
          aria-label="Query"
        />
        <button
          type="submit"
          data-testid="dataset-query-submit"
          disabled={!dataset || draft.trim().length === 0}
        >
          {mode === "search" ? "Search" : "Discover"}
        </button>
      </form>
      <label className="dataset-k">
        <span className="muted">Top-k: {k}</span>
        <input
          type="range"
          min={1}
          max={MAX_K}
          value={k}
          data-testid="dataset-k-slider"
          onChange={(e) => setK(Number(e.target.value))}
          aria-label="Number of results (k)"
        />
      </label>

      {mode === "discover" ? (
        <p className="muted dataset-discover-note">
          Advisory discovery — returns content-addressed refs + a display-only score (SN-8). Resolve
          bytes by the exact ref via the SDK.
        </p>
      ) : null}

      {active.isFetching ? <EmptyState title="Searching…" /> : null}
      {active.isError ? (
        isNoEmbedder(active.error) ? (
          <EmbedderNotice />
        ) : (
          <EmptyState title="Search failed" detail={toUiError(active.error).message} />
        )
      ) : null}
      {active.data && active.data.length === 0 && query ? (
        <EmptyState title="No matches" detail="Nothing in this corpus matched the query." />
      ) : null}

      {mode === "search" && hits.data && hits.data.length > 0 ? (
        <ol className="dataset-hits" data-testid="dataset-hits">
          {hits.data.map((h, i) => {
            const open = openRef === h.contentRef;
            return (
              <m.li
                key={h.contentRef}
                className="dataset-hit"
                data-testid="dataset-hit"
                {...rowEntrance(i, 0)}
              >
                <button
                  type="button"
                  className="dataset-hit__row"
                  data-testid="dataset-hit-toggle"
                  aria-expanded={open}
                  onClick={() => setOpenRef(open ? null : h.contentRef)}
                >
                  <span
                    className="dataset-hit__score"
                    title="Display-only similarity (never an identity input — SN-8)"
                  >
                    {h.score.toFixed(3)}
                  </span>
                  <span className="dataset-hit__text">{snippet(h.text)}</span>
                </button>
                <DigestChip hex={h.contentRef} label="doc" />
                {open ? (
                  <div className="dataset-hit__detail" data-testid="dataset-hit-detail">
                    <AssetViewer
                      content={decodeContent(h.content)}
                      stem={h.contentRef.slice(0, 12)}
                    />
                  </div>
                ) : null}
              </m.li>
            );
          })}
        </ol>
      ) : null}

      {mode === "discover" && fuzzy.data && fuzzy.data.length > 0 ? (
        <ol className="dataset-hits" data-testid="dataset-fuzzy-hits">
          {fuzzy.data.map((h, i) => (
            <m.li
              key={h.contentRef}
              className="dataset-hit"
              data-testid="dataset-fuzzy-hit"
              {...rowEntrance(i, 0)}
            >
              <span
                className="dataset-hit__score"
                title="Display-only similarity (never an identity input — SN-8)"
              >
                {(h.score * 100).toFixed(1)}%
              </span>
              <DigestChip hex={h.contentRef} label="ref" />
            </m.li>
          ))}
        </ol>
      ) : null}
    </GlowCard>
  );
}
