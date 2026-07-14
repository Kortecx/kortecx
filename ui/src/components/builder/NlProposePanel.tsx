/**
 * NL workflow authoring — the propose-then-confirm panel (D209.3 / SN-8). The author
 * describes a goal; the served model proposes a multi-step DAG (`proposeWorkflow`), which is
 * previewed here and, on confirm, applied to the builder canvas. VALIDATE-ONLY server-side:
 * nothing runs until the applied steps are saved / submitted. An honest rejection (no served
 * model, an inadmissible plan) is surfaced verbatim (don't-fake-gaps, D142).
 *
 * Portaled to <body> with the `--overlay` variants (the section-drawer pattern) so it clears
 * the sticky navbar. It NEVER mutates the base `.node-drawer__scrim` (the canvas-scoped
 * builder/DAG drawers share it).
 */

import type { ProposedWorkflowEdge, ProposedWorkflowStep } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useProposeWorkflow } from "../../kx/use-propose-workflow";
import { ErrorNotice } from "../ErrorNotice";

export function NlProposePanel({
  onApply,
  onClose,
}: {
  onApply: (steps: readonly ProposedWorkflowStep[], edges: readonly ProposedWorkflowEdge[]) => void;
  onClose: () => void;
}) {
  const [goal, setGoal] = useState("");
  const propose = useProposeWorkflow();
  const proposal = propose.data;

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canPropose = goal.trim().length > 0 && !propose.isPending;

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close describe-a-workflow"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
        data-testid="builder-propose-panel"
        // biome-ignore lint/a11y/useSemanticElements: a non-modal side panel riding framer-motion; dialog semantics via role+aria-label (the section-drawer precedent).
        role="dialog"
        aria-label="Describe your workflow"
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>Describe your workflow</strong>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="builder-field">
          <span className="builder-field__label">Goal</span>
          <textarea
            className="builder-input"
            data-testid="builder-propose-goal"
            rows={3}
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            placeholder="e.g. Research the top 3 durable-execution engines and write a comparison"
          />
          <span className="builder-field__hint">
            The served model proposes a multi-step plan of agent roles. You review it before it runs
            — nothing executes until you apply the steps and save or submit.
          </span>
        </div>

        <div className="builder-field">
          <button
            type="button"
            className="btn-primary"
            data-testid="builder-propose-submit"
            disabled={!canPropose}
            onClick={() => propose.mutate(goal)}
          >
            {propose.isPending ? "Planning…" : "Propose a plan"}
          </button>
        </div>

        {propose.error ? (
          <ErrorNotice error={toUiError(propose.error)} onRetry={() => propose.mutate(goal)} />
        ) : null}

        {proposal && !proposal.proposed ? (
          <p className="muted" data-testid="builder-propose-reject">
            {proposal.reason}
          </p>
        ) : null}

        {proposal?.proposed ? (
          <>
            <div className="builder-field">
              <span className="builder-field__label">
                Proposed plan — {proposal.steps.length} step
                {proposal.steps.length === 1 ? "" : "s"}
              </span>
              <ol className="propose-steps" data-testid="builder-propose-steps">
                {proposal.steps.map((s, i) => (
                  <li
                    // biome-ignore lint/suspicious/noArrayIndexKey: proposal steps are positional (edges reference them by index) and never reordered here.
                    key={i}
                    className="propose-step"
                    data-testid="builder-propose-step"
                  >
                    <span className="chip">{s.role}</span>
                    <span className="propose-step__intent">{s.intent}</span>
                  </li>
                ))}
              </ol>
              <span className="builder-field__hint">
                Each step becomes an editable agent on the canvas (its role's persona framing is
                applied). Refine, wire, or delete steps, then save or submit.
              </span>
            </div>
            <div className="node-drawer__foot">
              <button
                type="button"
                className="btn-primary"
                data-testid="builder-propose-apply"
                onClick={() => {
                  onApply(proposal.steps, proposal.edges);
                  onClose();
                }}
              >
                Apply to canvas
              </button>
            </div>
          </>
        ) : null}
      </m.aside>
    </>,
    document.body,
  );
}
