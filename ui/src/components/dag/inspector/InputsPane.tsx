/**
 * The inspector's Inputs pane (PR-2 "edge resolved text"): every inbound edge
 * with its parent's committed result RESOLVED to text — the data actually
 * flowing INTO this Mote — next to the digest chip (§4.11: resolved text is
 * the headline, the digest a secondary pointer). One preview-sized
 * `GetContentBatch` join over the ALREADY-FETCHED projection (parents[] +
 * sibling result refs) — NO new wire. Truncated previews link to the full
 * artifact in the run's Artifacts tab.
 */

import { Link } from "@tanstack/react-router";
import { toUiError } from "../../../kx/errors";
import { type BatchedContentVM, useContentBatch } from "../../../kx/use-content-batch";
import type { MoteVM } from "../../../kx/use-projection";
import { shortHex } from "../../../lib/format";
import { memberMoteSearch } from "../../../lib/run-anchor";
import { DigestChip } from "../../DigestChip";
import { EmptyState } from "../../EmptyState";
import { ErrorNotice } from "../../ErrorNotice";
import { CodeViewer } from "../../editor/CodeViewer";

export function InputsPane({
  mote,
  motes,
  instanceId,
}: {
  mote: MoteVM;
  motes: readonly MoteVM[];
  instanceId: string;
}) {
  // Join each inbound edge to its parent's committed result ref (the sibling
  // lookup over the projection we already hold — no extra projection RPC).
  const rows = mote.parents.map((edge) => ({
    edge,
    resultRef: motes.find((m) => m.moteId === edge.parentId)?.resultRef ?? null,
  }));
  const refs = rows.flatMap((r) => (r.resultRef ? [r.resultRef] : []));
  const batch = useContentBatch(instanceId, refs);

  if (rows.length === 0) {
    return <EmptyState title="No inputs" detail="This Mote is a root — no inbound edges." />;
  }
  if (batch.error) {
    return <ErrorNotice error={toUiError(batch.error)} onRetry={() => void batch.refetch()} />;
  }
  const byRef = new Map((batch.data ?? []).map((item) => [item.contentRef, item]));
  return (
    <ul className="inspector-inputs" data-testid="inspector-inputs">
      {rows.map(({ edge, resultRef }) => (
        <li key={edge.parentId} data-testid="inspector-input-row">
          <div className="inspector-inputs__head">
            <DigestChip hex={edge.parentId} label="parent" />
            <span className="badge" title="Edge kind">
              {edge.edgeKind}
            </span>
            {edge.nonCascade ? (
              <span className="badge" title="Failure does not cascade over this edge">
                non-cascade
              </span>
            ) : null}
          </div>
          <InputPreview
            instanceId={instanceId}
            // The inspected Mote anchors the artifact deep-link back to THIS run — the
            // Artifacts tab opens its own projection query and would otherwise widen to
            // the whole journal at the moment the user drills into one preview.
            anchorMoteId={mote.moteId}
            resultRef={resultRef}
            item={resultRef ? byRef.get(resultRef) : undefined}
            loading={batch.isLoading && refs.length > 0}
          />
        </li>
      ))}
    </ul>
  );
}

function InputPreview({
  instanceId,
  anchorMoteId,
  resultRef,
  item,
  loading,
}: {
  instanceId: string;
  anchorMoteId: string;
  resultRef: string | null;
  item: BatchedContentVM | undefined;
  loading: boolean;
}) {
  if (!resultRef) {
    return <p className="muted">No committed result yet — the parent has not committed.</p>;
  }
  if (loading && !item) {
    return <p className="muted">Resolving…</p>;
  }
  if (!item || item.missing) {
    return <p className="muted">Payload unavailable.</p>;
  }
  if (item.content.kind === "empty") {
    return <p className="muted">Empty result.</p>;
  }
  return (
    <>
      <CodeViewer
        value={item.content.text}
        language={item.content.kind === "json" ? "json" : "plaintext"}
        testId={`inspector-input-${shortHex(resultRef)}`}
        ariaLabel={`Resolved input ${shortHex(resultRef)}`}
        height={Math.min(180, Math.max(60, item.content.text.split("\n").length * 19 + 24))}
      />
      {item.truncated ? (
        <p className="muted">
          Preview (first 4 KiB of {item.fullSize} bytes) —{" "}
          <Link
            to="/workflows/$instanceId"
            params={{ instanceId }}
            search={{ ...memberMoteSearch(anchorMoteId), tab: "artifacts", ref: resultRef }}
          >
            open the full artifact
          </Link>
        </p>
      ) : null}
    </>
  );
}
