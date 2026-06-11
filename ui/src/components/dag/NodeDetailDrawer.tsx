/**
 * The DAG node-detail drawer. Clicking a Mote in the live graph opens a right-side
 * panel with that Mote's identity (id / state / nd_class / promotion / committed seq)
 * and its committed result rendered READ-ONLY in the Monaco code viewer. Pure read
 * surface — it reuses {@link useContent} (immutable, content-addressed) and the run's
 * own `instanceId` (the ownership ticket), so it adds NO new RPC and NO new wire.
 *
 * It renders as a sibling OVERLAY of the ReactFlow canvas, so opening/closing never
 * mutates the graph's nodes/edges/positions — the no-thrash layout invariant holds.
 */

import { m } from "framer-motion";
import { useEffect } from "react";
import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import type { MoteVM } from "../../kx/use-projection";
import { promotionIsNotable, promotionLabel } from "../../lib/colors";
import { formatSeq, shortHex } from "../../lib/format";
import { AnomalyBadge } from "../AnomalyBadge";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { NdClassBadge } from "../NdClassBadge";
import { StatePill } from "../StatePill";
import { CodeViewer } from "../editor/CodeViewer";

const slideIn = {
  initial: { x: 24, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  transition: { type: "spring", stiffness: 420, damping: 34 },
} as const;

export function NodeDetailDrawer({
  mote,
  instanceId,
  onClose,
}: {
  mote: MoteVM;
  instanceId: string;
  onClose: () => void;
}) {
  // Close on Escape (a11y); re-bound per selected Mote.
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close detail"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="node-detail-drawer"
        data-mote={mote.moteId}
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion animations and would impose modal semantics; this is a non-modal side panel, dialog semantics declared via role+aria-label
        role="dialog"
        aria-label={`Mote ${shortHex(mote.moteId)} detail`}
        initial={slideIn.initial}
        animate={slideIn.animate}
        transition={slideIn.transition}
      >
        <div className="node-drawer__head">
          <code className="mono node-drawer__id" title={mote.moteId}>
            {shortHex(mote.moteId)}
          </code>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="node-drawer__badges">
          <StatePill stateCode={mote.stateCode} />
          <NdClassBadge ndClass={mote.ndClass} />
          {promotionIsNotable(mote.promotion) ? (
            <span className="badge" title="Promotion state">
              {promotionLabel(mote.promotion)}
            </span>
          ) : null}
        </div>
        <AnomalyBadge anomaly={mote.anomaly} />

        <dl className="node-drawer__meta">
          <div>
            <dt>Committed seq</dt>
            <dd className="mono">{formatSeq(mote.committedSeq)}</dd>
          </div>
          <div>
            <dt>Result ref</dt>
            <dd className="mono" title={mote.resultRef ?? undefined}>
              {mote.resultRef ? shortHex(mote.resultRef) : "—"}
            </dd>
          </div>
          <div>
            <dt>Parents</dt>
            <dd className="mono">{mote.parents.length}</dd>
          </div>
        </dl>

        <div className="node-drawer__result">
          <span className="section-label">Committed result</span>
          <NodeResult instanceId={instanceId} resultRef={mote.resultRef} />
        </div>
      </m.aside>
    </>
  );
}

/** The committed payload (via GetContent), or an honest "no result yet" for an
 *  uncommitted Mote (the `useContent` query is disabled until a ref exists). */
function NodeResult({ instanceId, resultRef }: { instanceId: string; resultRef: string | null }) {
  const content = useContent(instanceId, resultRef ?? undefined);
  if (!resultRef) {
    return <EmptyState title="No committed result yet" detail="This Mote has not committed." />;
  }
  if (content.isLoading) {
    return <EmptyState title="Loading result…" />;
  }
  if (content.error) {
    return <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />;
  }
  if (!content.data) {
    return null;
  }
  if (content.data.kind === "empty") {
    return <EmptyState title="Empty result" detail="This Mote committed no output." />;
  }
  return (
    <CodeViewer
      value={content.data.text}
      language={content.data.kind === "json" ? "json" : "plaintext"}
      testId="node-detail-result"
      ariaLabel="Committed result"
      height={Math.min(320, Math.max(96, content.data.text.split("\n").length * 19 + 24))}
    />
  );
}
