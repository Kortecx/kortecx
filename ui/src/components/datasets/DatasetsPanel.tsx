import { useEffect } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDatasets } from "../../kx/use-datasets";
import { EmptyState } from "../EmptyState";
import { GlowCard } from "../ds/GlowCard";

/**
 * The corpora panel: a CHIP picker over the datasets the gateway holds (button
 * controls, never a controlled `<select>` — the Playwright `selectOption` gotcha),
 * driving the query panel for the selected dataset. Degrades to a not-enabled empty
 * state when the gateway lacks the `hnsw` feature (UNIMPLEMENTED).
 */
export function DatasetsPanel({
  selectedDataset,
  onSelect,
}: {
  selectedDataset: string | null;
  onSelect: (id: string) => void;
}) {
  const datasets = useDatasets();
  const list = datasets.data ?? [];
  const effective =
    selectedDataset && list.some((d) => d.datasetId === selectedDataset)
      ? selectedDataset
      : (list[0]?.datasetId ?? null);

  // Default the selection to the first dataset once they load.
  useEffect(() => {
    const first = list[0];
    if (!selectedDataset && first) {
      onSelect(first.datasetId);
    }
  }, [selectedDataset, list, onSelect]);

  const notWired = datasets.isError && toUiError(datasets.error).kind === "not-wired";

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="datasets-panel">
      <h2>Corpora</h2>
      {datasets.isLoading ? <EmptyState title="Loading datasets…" /> : null}
      {notWired ? (
        <EmptyState
          title="Datasets not enabled here"
          detail="This gateway was built without the `hnsw` feature. Start it with `kx serve --features hnsw` to enable RAG corpora."
        />
      ) : null}
      {datasets.isError && !notWired ? (
        <EmptyState title="Couldn't load datasets" detail={toUiError(datasets.error).message} />
      ) : null}
      {datasets.data && list.length === 0 ? (
        <EmptyState
          title="No datasets yet"
          detail="Ingest documents below to create your first corpus."
        />
      ) : null}

      {list.length > 0 ? (
        <div className="chip-row" role="radiogroup" aria-label="Dataset">
          {list.map((d) => (
            <button
              key={d.datasetId}
              type="button"
              data-testid={`dataset-pick-${d.datasetId}`}
              className={`chip${d.datasetId === effective ? " chip--active" : ""}`}
              aria-pressed={d.datasetId === effective}
              onClick={() => onSelect(d.datasetId)}
            >
              <span className="chip__label">{d.name || d.datasetId}</span>
              <span className="chip__meta">
                {d.docCount} doc{d.docCount === 1 ? "" : "s"}
                {d.chunked ? ` · ${d.chunkCount} chunks` : ""} · dim {d.dim}
              </span>
            </button>
          ))}
        </div>
      ) : null}
    </GlowCard>
  );
}
