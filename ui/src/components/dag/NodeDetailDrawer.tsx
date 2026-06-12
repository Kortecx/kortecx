/**
 * The DAG node-detail drawer (PR-2: the run INSPECTOR). Clicking a Mote in the
 * live graph opens a right-side panel with that Mote's identity plus five
 * read-only panes: the committed Result, and — via `GetMoteDetail` (Batch B) —
 * the admitted Prompt, Params, Tool contract, and the resolved Inputs (each
 * inbound edge's parent result text, the "edge resolved text" join).
 *
 * The detail query is COMMIT-GATED: `mote_def_hash` only exists on a Committed
 * fact, so a pending node renders "available after commit" with NO RPC. It
 * renders as a sibling OVERLAY of the ReactFlow canvas, so opening/closing —
 * and switching panes — never mutates the graph's nodes/edges/positions (the
 * no-thrash layout invariant holds). Pure read surface (D141.3, SN-8).
 */

import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import { useMoteDetail } from "../../kx/use-mote-detail";
import type { MoteVM } from "../../kx/use-projection";
import { promotionIsNotable, promotionLabel } from "../../lib/colors";
import { formatSeq, shortHex } from "../../lib/format";
import { AnomalyBadge } from "../AnomalyBadge";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { NdClassBadge } from "../NdClassBadge";
import { StatePill } from "../StatePill";
import { CodeViewer } from "../editor/CodeViewer";
import { InputsPane } from "./inspector/InputsPane";
import { ParamsPane } from "./inspector/ParamsPane";
import { PromptPane } from "./inspector/PromptPane";
import { ToolContractPane } from "./inspector/ToolContractPane";

const slideIn = {
  initial: { x: 24, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  transition: { type: "spring", stiffness: 420, damping: 34 },
} as const;

const PANES = ["result", "prompt", "params", "tools", "inputs"] as const;
type Pane = (typeof PANES)[number];

const PANE_LABEL: Record<Pane, string> = {
  result: "Result",
  prompt: "Prompt",
  params: "Params",
  tools: "Tools",
  inputs: "Inputs",
};

export function NodeDetailDrawer({
  mote,
  motes,
  instanceId,
  onClose,
}: {
  mote: MoteVM;
  /** The run's full Mote set — the Inputs pane's parent-result join. */
  motes: readonly MoteVM[];
  instanceId: string;
  onClose: () => void;
}) {
  const [pane, setPane] = useState<Pane>("result");
  const detail = useMoteDetail(instanceId, mote.moteId, mote.moteDefHash);

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
          {detail.data?.defFound ? (
            <span className="badge" title="Step kind (display classification)">
              {detail.data.stepKind}
            </span>
          ) : null}
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

        <fieldset className="view-toggle" aria-label="Inspector pane" data-testid="inspector-panes">
          {PANES.map((p) => (
            <button
              key={p}
              type="button"
              aria-pressed={pane === p}
              data-testid={`inspector-pane-${p}`}
              onClick={() => setPane(p)}
            >
              {PANE_LABEL[p]}
            </button>
          ))}
        </fieldset>

        <div className="node-drawer__result">
          {pane === "result" ? (
            <>
              <span className="section-label">Committed result</span>
              <NodeResult instanceId={instanceId} resultRef={mote.resultRef} />
            </>
          ) : pane === "inputs" ? (
            <InputsPane mote={mote} motes={motes} instanceId={instanceId} />
          ) : (
            <DefPane pane={pane} mote={mote} detail={detail} />
          )}
        </div>
      </m.aside>
    </>
  );
}

/** The def-backed panes (Prompt / Params / Tools), with the honest empties:
 *  commit-gated (no RPC before the def hash exists) and the pre-Batch-B
 *  "definition not retained" shape (`def_found = false`). */
function DefPane({
  pane,
  mote,
  detail,
}: {
  pane: "prompt" | "params" | "tools";
  mote: MoteVM;
  detail: ReturnType<typeof useMoteDetail>;
}) {
  if (mote.moteDefHash === "") {
    return (
      <EmptyState
        title="Available after commit"
        detail="The definition resolves once this Mote commits (the def hash lives on the Committed fact)."
      />
    );
  }
  if (detail.isLoading) {
    return <EmptyState title="Loading definition…" />;
  }
  if (detail.error) {
    return <ErrorNotice error={toUiError(detail.error)} onRetry={() => void detail.refetch()} />;
  }
  if (!detail.data) {
    return null;
  }
  if (!detail.data.defFound) {
    return (
      <EmptyState
        title="Definition not retained"
        detail="This Mote was admitted by a binary predating definition persistence (PR-2); only its hash survives."
      />
    );
  }
  if (pane === "prompt") {
    return <PromptPane detail={detail.data} />;
  }
  if (pane === "params") {
    return <ParamsPane detail={detail.data} />;
  }
  return <ToolContractPane detail={detail.data} />;
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
