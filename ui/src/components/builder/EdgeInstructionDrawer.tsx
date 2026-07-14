/**
 * The edge instruction-file drawer (D141.5) — attach an inter-step instruction
 * between two authored steps. Surfaced ON the edge as a file chip; the resolved
 * text edits in Monaco. In Tier-1 the instruction folds into the downstream agent's
 * prompt at submit (its durable content-bundle backing arrives with PR-7 — stated
 * honestly here, don't-fake-gaps D142).
 */

import { m } from "framer-motion";
import { useEffect } from "react";
import { MonacoMount } from "../editor/MonacoMount";
import type { BuilderEdge, BuilderStep } from "./builder-graph";

const slideIn = {
  initial: { x: 24, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  transition: { type: "spring", stiffness: 420, damping: 34 },
} as const;

export function EdgeInstructionDrawer({
  edge,
  steps,
  hideInstruction = false,
  onChange,
  onDelete,
  onClose,
}: {
  edge: BuilderEdge;
  steps: readonly BuilderStep[];
  /** POC-5d: hide the instruction field in App modes — an edge instruction is a
   *  run-only fold with no DagSpec representation, so it can't persist to a saved
   *  App blueprint. The drawer stays useful for viewing the route + removing the edge. */
  hideInstruction?: boolean;
  onChange: (text: string) => void;
  onDelete: () => void;
  onClose: () => void;
}) {
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const from = steps.find((s) => s.id === edge.source)?.label ?? edge.source;
  const to = steps.find((s) => s.id === edge.target)?.label ?? edge.target;
  const target = steps.find((s) => s.id === edge.target);

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close instruction editor"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="edge-instruction-drawer"
        // biome-ignore lint/a11y/useSemanticElements: non-modal side panel riding framer-motion; dialog semantics via role+aria-label.
        role="dialog"
        aria-label={`Instruction from ${from} to ${to}`}
        initial={slideIn.initial}
        animate={slideIn.animate}
        transition={slideIn.transition}
      >
        <div className="node-drawer__head">
          <span className="builder-edge__route">
            {from} → {to}
          </span>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        {hideInstruction ? (
          <div className="builder-field">
            <span className="builder-field__hint" data-testid="edge-instruction-app-note">
              Edges in an App are pure control/data flow ({edge.edge}). Encode step-to-step guidance
              in the downstream agent's prompt.
            </span>
          </div>
        ) : (
          <div className="builder-field">
            <span className="builder-field__label">Instruction file</span>
            <MonacoMount
              value={edge.instruction}
              language="plaintext"
              onChange={onChange}
              height={200}
              testId="edge-instruction"
              ariaLabel="Edge instruction text"
              placeholder="Context / instructions passed from the upstream step to this one…"
            />
            <span className="builder-field__hint">
              {target?.kind === "model"
                ? "Prepended to the downstream agent's prompt at run time (Tier-1)."
                : "Carried as provenance on this edge (the downstream step is not an agent)."}
            </span>
          </div>
        )}

        <div className="node-drawer__foot">
          <button
            type="button"
            className="linkbtn danger"
            data-testid="edge-instruction-delete"
            onClick={onDelete}
          >
            Remove edge
          </button>
        </div>
      </m.aside>
    </>
  );
}
