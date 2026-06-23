import { useDatasets } from "../../kx/use-datasets";

/**
 * POC-1 CHAT-RAG: the composer dataset control — a dropdown over `ListDatasets`.
 * Picking a non-empty dataset routes the turn to `kx/recipes/chat-rag`, which
 * grounds the answer on that corpus (embed → top-k → fold the exact refs). The
 * default "None" is a plain, ungrounded chat. Only datasets with an indexed
 * document are selectable for grounding; an empty one is shown disabled (honest:
 * grounding turns on AFTER you ingest). On a gateway without the `hnsw` dataset
 * view (UNIMPLEMENTED) the control renders nothing — there is nothing to ground on.
 */
export function DatasetPicker({
  value,
  onChange,
}: {
  value: string | undefined;
  onChange: (dataset: string | undefined) => void;
}) {
  const { data, isLoading, isError } = useDatasets();
  // No dataset view wired (hnsw off / old gateway), still loading, or none yet
  // ingested ⇒ nothing to ground on — don't render a fake knob (don't-fake-gaps).
  if (isError || isLoading || data === undefined || data.length === 0) {
    return null;
  }
  return (
    <label className="modelpicker" data-testid="dataset-picker">
      <span className="modelpicker__label">Dataset</span>
      <select
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value === "" ? undefined : e.target.value)}
        aria-label="Grounding dataset"
        data-testid="dataset-picker-select"
      >
        <option value="">None — plain chat</option>
        {data.map((d) => (
          <option key={d.datasetId} value={d.name} disabled={d.docCount === 0}>
            {d.name}
            {d.docCount === 0 ? " (empty — ingest first)" : ` (${d.docCount} docs)`}
          </option>
        ))}
      </select>
    </label>
  );
}
