import { m } from "framer-motion";
import { type FormEvent, useState } from "react";
import { fadeUp, rowEntrance } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDatasetQuery } from "../../kx/use-datasets";
import { EmptyState } from "../EmptyState";
import { GlowCard } from "../ds/GlowCard";
import { EmbedderNotice, isNoEmbedder } from "./EmbedderNotice";

/**
 * Semantic search over the selected dataset. The query text is embedded server-side
 * (the `inference` feature); without an embedder the gateway returns
 * FAILED_PRECONDITION and the panel shows the {@link EmbedderNotice}. Each hit's
 * score is DISPLAY-only (SN-8) — a ranking aid, never an identity input.
 */
export function QueryPanel({ dataset }: { dataset: string | null }) {
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const hits = useDatasetQuery(dataset ?? undefined, query);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setQuery(draft.trim());
  };

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="dataset-query-panel">
      <h2>Search</h2>
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
          Search
        </button>
      </form>

      {hits.isFetching ? <EmptyState title="Searching…" /> : null}
      {hits.isError ? (
        isNoEmbedder(hits.error) ? (
          <EmbedderNotice />
        ) : (
          <EmptyState title="Search failed" detail={toUiError(hits.error).message} />
        )
      ) : null}
      {hits.data && hits.data.length === 0 && query ? (
        <EmptyState title="No matches" detail="Nothing in this corpus matched the query." />
      ) : null}
      {hits.data && hits.data.length > 0 ? (
        <ol className="dataset-hits" data-testid="dataset-hits">
          {hits.data.map((h, i) => (
            <m.li
              key={h.contentRef}
              className="dataset-hit"
              data-testid="dataset-hit"
              {...rowEntrance(i, 0)}
            >
              <span
                className="dataset-hit__score"
                title="Display-only similarity (never an identity input — SN-8)"
              >
                {h.score.toFixed(3)}
              </span>
              <span className="dataset-hit__text">{h.text}</span>
            </m.li>
          ))}
        </ol>
      ) : null}
    </GlowCard>
  );
}
