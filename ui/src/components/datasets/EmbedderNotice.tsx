import { ErrorCode } from "@kortecx/sdk/web";
import { toUiError } from "../../kx/errors";
import { EmptyState } from "../EmptyState";

/**
 * `true` iff the error is the gateway's "no embedding model wired" precondition
 * (FAILED_PRECONDITION) — a text ingest/query needs a server embedder the gateway
 * lacks. Distinct from a generic bad-input so the panels can guide the operator.
 */
export function isNoEmbedder(err: unknown): boolean {
  return toUiError(err).code === ErrorCode.FailedPrecondition;
}

/** Actionable guidance when a text ingest/search needs a server embedder. */
export function EmbedderNotice() {
  return (
    <EmptyState
      title="No embedding model on this gateway"
      detail="Text ingest & search need an embedder. Run `kx serve --features inference` with a model (KX_SERVE_MODEL_GGUF), or supply vectors directly via the SDK (the FFI-free client-vector path)."
    />
  );
}
