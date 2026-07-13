import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { type FileDoc, useIngestDocuments } from "../../kx/use-datasets";
import { EmptyState } from "../EmptyState";
import { GlowCard } from "../ds/GlowCard";
import { EmbedderNotice, isNoEmbedder } from "./EmbedderNotice";

/**
 * Ingest documents into a named dataset — TEXT (one per line) and/or uploaded FILES
 * (multimodal: images, PDFs, any bytes). Both cross the wire as raw
 * `IngestDoc.content` bytes over the shipped `IngestDocuments` RPC. The gateway embeds
 * + indexes each — so this needs a server embedder (the `inference` feature); without
 * one it returns FAILED_PRECONDITION and the panel shows the {@link EmbedderNotice}
 * (or use the SDK's FFI-free client-vector path). Re-ingesting identical content is a
 * no-op (content-addressed dedup); the server derives each doc's id (SN-8).
 */
export function IngestPanel() {
  const [name, setName] = useState("");
  const [text, setText] = useState("");
  const [files, setFiles] = useState<File[]>([]);
  const ingest = useIngestDocuments();

  const addFiles = (list: FileList | null) => {
    if (list && list.length > 0) {
      // Snapshot the FileList SYNCHRONOUSLY — the caller resets the input's value right
      // after (to allow re-picking the same file), which empties the live FileList
      // before a deferred state-updater would read it.
      const picked = Array.from(list);
      setFiles((prev) => [...prev, ...picked]);
    }
  };
  const removeFile = (idx: number) => setFiles((prev) => prev.filter((_, i) => i !== idx));

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const docs = text
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
    // Read each file to raw bytes; carry the name/media type as advisory metadata.
    const fileDocs: FileDoc[] = await Promise.all(
      files.map(async (f) => {
        const metadata: Record<string, string> = { name: f.name };
        if (f.type) {
          metadata.mediaType = f.type;
        }
        return { content: new Uint8Array(await f.arrayBuffer()), metadata };
      }),
    );
    if (name.trim() && (docs.length > 0 || fileDocs.length > 0)) {
      ingest.mutate({ dataset: name.trim(), docs, fileDocs }, { onSuccess: () => setFiles([]) });
    }
  };

  const nothingToIngest = text.trim().length === 0 && files.length === 0;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="dataset-ingest-panel">
      <h2>Ingest</h2>
      <p className="muted">
        Text (one document per line) and/or uploaded files — the gateway embeds + indexes each.
      </p>
      <form onSubmit={(e) => void handleSubmit(e)} className="dataset-ingest-form">
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
        <div className="dataset-ingest__files">
          <label className="linkbtn dataset-ingest__addfiles">
            <input
              type="file"
              multiple
              hidden
              data-testid="dataset-ingest-file-input"
              onChange={(e) => {
                addFiles(e.target.files);
                e.target.value = ""; // allow re-picking the same file
              }}
              aria-label="Add files"
            />
            + Add files
          </label>
          {files.length > 0 ? (
            <ul className="dataset-ingest__filelist" data-testid="dataset-ingest-filelist">
              {files.map((f, i) => (
                <li
                  key={`${f.name}:${f.size}:${f.lastModified}`}
                  className="dataset-ingest__file"
                  data-testid={`dataset-ingest-file-${f.name}`}
                >
                  <span className="mono">{f.name}</span>
                  <span className="muted">{formatBytes(f.size)}</span>
                  <button
                    type="button"
                    className="dataset-ingest__file-remove"
                    aria-label={`Remove ${f.name}`}
                    data-testid={`dataset-ingest-file-remove-${f.name}`}
                    onClick={() => removeFile(i)}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        <button
          type="submit"
          data-testid="dataset-ingest-submit"
          disabled={ingest.isPending || name.trim().length === 0 || nothingToIngest}
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
    </GlowCard>
  );
}

/** Compact human byte size for the staged-file list (display-only). */
function formatBytes(n: number): string {
  if (n < 1024) {
    return `${n} B`;
  }
  if (n < 1024 * 1024) {
    return `${(n / 1024).toFixed(1)} KB`;
  }
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
