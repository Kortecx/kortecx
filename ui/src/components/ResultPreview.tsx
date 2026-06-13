import type { DecodedContent } from "../lib/content-decode";
import { DigestChip } from "./DigestChip";

/**
 * The D142.2 "resolved text is the HEADLINE, the digest a secondary pointer"
 * treatment for a committed result, in an inline/dense context (mote tables,
 * the artifact list, event feeds, the DAG node). PRESENTATIONAL: the container
 * batch-fetches all visible refs in ONE `getContentBatch` (the N+1 collapse —
 * see `use-content-batch`) and feeds each row its decoded `content`, so a wide
 * table/feed never fans out per row. The full payload is one click away (the
 * node-detail Result pane / artifact view render the complete text).
 *
 * States (all honest): uncommitted (no ref) · resolving · unavailable (the
 * uniform-empty item) · empty result · binary · truncated preview · text.
 */

/** Collapse whitespace + clip to a one-line preview (the dense-context headline). */
export function oneLine(text: string, max = 140): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max)}…` : flat;
}

export function ResultPreview({
  resultRef,
  content,
  loading = false,
  missing = false,
  max = 140,
  chip = true,
}: {
  /** The committed result's content ref (hex), or null when uncommitted. */
  resultRef: string | null;
  /** The decoded payload (from the container's batch fetch); undefined while loading. */
  content?: DecodedContent;
  loading?: boolean;
  /** True iff the batch returned the uniform-empty item for this ref. */
  missing?: boolean;
  /** One-line preview character cap. */
  max?: number;
  /** Render the trailing `DigestChip`. Set false when the container sits inside a
   *  `<button>` (nested buttons are invalid) and renders its own chip as a sibling. */
  chip?: boolean;
}) {
  // Uncommitted Mote — no result yet (honest, not a hash).
  if (!resultRef) {
    return <span className="muted">—</span>;
  }

  const chipEl = chip ? <DigestChip hex={resultRef} label="result" /> : null;

  if (loading && !content) {
    return (
      <span className="result-preview" data-testid="result-preview" data-state="loading">
        <span className="muted">resolving…</span> {chipEl}
      </span>
    );
  }
  if (missing) {
    return (
      <span className="result-preview" data-testid="result-preview" data-state="missing">
        <span className="muted">unavailable</span> {chipEl}
      </span>
    );
  }
  if (!content || content.kind === "empty") {
    return (
      <span className="result-preview" data-testid="result-preview" data-state="empty">
        <span className="muted">(empty)</span> {chipEl}
      </span>
    );
  }
  if (content.kind === "binary") {
    return (
      <span className="result-preview" data-testid="result-preview" data-state="binary">
        <span className="muted">binary · {content.byteLength} B</span> {chipEl}
      </span>
    );
  }
  // text | json: the resolved text IS the headline; the digest is the pointer.
  return (
    <span className="result-preview" data-testid="result-preview" data-state="text">
      <span className="result-preview__text" title={content.text}>
        {oneLine(content.text, max)}
      </span>{" "}
      {chipEl}
    </span>
  );
}
