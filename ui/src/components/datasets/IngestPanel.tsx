import { type FormEvent, useState } from "react";
import { toUiError } from "../../kx/errors";
import { useIngestDocuments } from "../../kx/use-datasets";
import { EmptyState } from "../EmptyState";
import { EmbedderNotice, isNoEmbedder } from "./EmbedderNotice";

/**
 * Ingest text documents (one per line) into a named dataset. The gateway embeds +
 * indexes each — so this needs a server embedder (the `inference` feature); without
 * one it returns FAILED_PRECONDITION and the panel shows the {@link EmbedderNotice}
 * (or use the SDK's FFI-free client-vector path). Re-ingesting identical content is
 * a no-op (content-addressed dedup); the server derives each doc's id (SN-8).
 */
export function IngestPanel() {
  const [name, setName] = useState("");
  const [text, setText] = useState("");
  const ingest = useIngestDocuments();

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const docs = text
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
    if (name.trim() && docs.length > 0) {
      ingest.mutate({ dataset: name.trim(), docs });
    }
  };

  return (
    <div data-testid="dataset-ingest-panel">
      <h2>Ingest</h2>
      <p className="muted">One document per line — the gateway embeds + indexes each.</p>
      <form onSubmit={onSubmit} className="dataset-ingest-form">
        <input
          type="text"
          data-testid="dataset-ingest-name"
          placeholder="dataset name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label="Dataset name"
        />
        <textarea
          data-testid="dataset-ingest-text"
          rows={4}
          placeholder="One document per line…"
          value={text}
          onChange={(e) => setText(e.target.value)}
          aria-label="Documents"
        />
        <button
          type="submit"
          data-testid="dataset-ingest-submit"
          disabled={ingest.isPending || name.trim().length === 0 || text.trim().length === 0}
        >
          {ingest.isPending ? "Ingesting…" : "Ingest"}
        </button>
      </form>

      {ingest.isError ? (
        isNoEmbedder(ingest.error) ? (
          <EmbedderNotice />
        ) : (
          <EmptyState title="Ingest failed" detail={toUiError(ingest.error).message} />
        )
      ) : null}
      {ingest.isSuccess ? (
        <p className="dataset-ingest__result" data-testid="dataset-ingest-result">
          Ingested into <strong>{ingest.data.datasetId}</strong>: +{ingest.data.inserted} new (
          {ingest.data.docCount} total, dim {ingest.data.dim}).
        </p>
      ) : null}
    </div>
  );
}
