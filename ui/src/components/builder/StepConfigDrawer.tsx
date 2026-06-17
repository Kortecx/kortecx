/**
 * The builder's step-config drawer (D141.6 n8n-level node config). A right-side
 * panel — the editable counterpart of the read-only `NodeDetailDrawer` (reuses
 * the `.node-drawer` design language) — to configure one authored step: its label,
 * and for an AGENT (MODEL) step the served model, the prompt (Monaco), the opt-in
 * reasoning-mode, plus typed free-params (Monaco JSON). No fake knobs: a control
 * appears only where the wire enforces it (don't-fake-gaps, D142).
 */

import type { ModelSummary } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { useEffect } from "react";
import { useDiscoverTools } from "../../kx/use-tool-registry";
import { JsonEditor } from "../editor/JsonEditor";
import { MonacoMount } from "../editor/MonacoMount";
import type { BuilderStep } from "./builder-graph";
import { isJsonObject } from "./builder-graph";

/** The drawer's kind badge label + modifier (PURE / MODEL / TOOL). */
function kindBadge(kind: BuilderStep["kind"]): { label: string; mod: string } {
  if (kind === "model") return { label: "Agent", mod: "model" };
  if (kind === "tool") return { label: "Tool", mod: "tool" };
  return { label: "Pure", mod: "pure" };
}

const slideIn = {
  initial: { x: 24, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  transition: { type: "spring", stiffness: 420, damping: 34 },
} as const;

/** The opt-in reasoning-mode chips (PR-4 Phase F). "" = default (the model's own). */
const REASONING: ReadonlyArray<{ value: BuilderStep["reasoning"]; label: string }> = [
  { value: "", label: "Default" },
  { value: "full", label: "Full" },
  { value: "minimal", label: "Minimal" },
  { value: "off", label: "Off" },
];

export function StepConfigDrawer({
  step,
  models,
  modelsUnsupported,
  onChange,
  onDelete,
  onClose,
}: {
  step: BuilderStep;
  models: readonly ModelSummary[] | undefined;
  modelsUnsupported: boolean;
  onChange: (next: BuilderStep) => void;
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

  const paramsValid = isJsonObject(step.paramsText);
  const served = (models ?? []).filter((mm) => mm.serving);
  const badge = kindBadge(step.kind);
  // PR-6b-2: the LIVE registered-tool set for a TOOL step's picker (DiscoverTools).
  const { tools, notWired: toolsNotWired } = useDiscoverTools();

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close step config"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="step-config-drawer"
        data-node={step.id}
        // biome-ignore lint/a11y/useSemanticElements: non-modal side panel riding framer-motion; dialog semantics via role+aria-label (mirrors NodeDetailDrawer).
        role="dialog"
        aria-label={`Configure ${step.label}`}
        initial={slideIn.initial}
        animate={slideIn.animate}
        transition={slideIn.transition}
      >
        <div className="node-drawer__head">
          <span className={`builder-node__kind builder-node__kind--${badge.mod}`}>
            {badge.label}
          </span>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <label className="builder-field">
          <span className="builder-field__label">Name</span>
          <input
            className="builder-input"
            data-testid="step-config-label"
            value={step.label}
            onChange={(e) => onChange({ ...step, label: e.target.value })}
          />
        </label>

        {step.kind === "model" ? (
          <>
            <div className="builder-field">
              <span className="builder-field__label">Model</span>
              {modelsUnsupported || served.length === 0 ? (
                <p className="muted" data-testid="step-config-no-models">
                  No model is being served. Start <code>kx serve --features inference</code> with
                  <code>KX_SERVE_MODEL_GGUF</code> to run agent steps.
                </p>
              ) : (
                <div className="builder-chips" data-testid="step-config-model">
                  {served.map((mm) => (
                    <button
                      key={mm.modelId}
                      type="button"
                      className={`chip${step.modelId === mm.modelId ? " chip--active" : ""}`}
                      onClick={() => onChange({ ...step, modelId: mm.modelId })}
                    >
                      {mm.modelId}
                    </button>
                  ))}
                </div>
              )}
            </div>

            <div className="builder-field">
              <span className="builder-field__label">Prompt</span>
              <MonacoMount
                value={step.prompt}
                language="plaintext"
                onChange={(v) => onChange({ ...step, prompt: v })}
                height={160}
                testId="step-config-prompt"
                ariaLabel="Agent prompt"
                placeholder="What should this agent do?"
              />
            </div>

            <div className="builder-field">
              <span className="builder-field__label">Reasoning</span>
              <div className="builder-chips" data-testid="step-config-reasoning">
                {REASONING.map((r) => (
                  <button
                    key={r.value || "default"}
                    type="button"
                    className={`chip${step.reasoning === r.value ? " chip--active" : ""}`}
                    onClick={() => onChange({ ...step, reasoning: r.value })}
                  >
                    {r.label}
                  </button>
                ))}
              </div>
              <span className="builder-field__hint">
                Opt-in: maps to the model's native think / no-think. Default leaves the model's own
                behavior (and the step's identity) unchanged.
              </span>
            </div>
          </>
        ) : null}

        {step.kind === "tool" ? (
          <div className="builder-field">
            <span className="builder-field__label">Tool</span>
            {toolsNotWired || tools.length === 0 ? (
              <p className="muted" data-testid="step-config-no-tools">
                No tools are registered. Register one in <strong>Tools → Registry</strong>, or dial
                an external MCP server in <strong>Tools → Connections</strong>, then pick it here.
              </p>
            ) : (
              <div className="builder-chips" data-testid="step-config-tool">
                {tools.map((t) => {
                  const active = step.toolId === t.toolName && step.toolVersion === t.toolVersion;
                  return (
                    <button
                      key={`${t.toolName}@${t.toolVersion}`}
                      type="button"
                      className={`chip${active ? " chip--active" : ""}`}
                      title={t.description}
                      onClick={() =>
                        onChange({ ...step, toolId: t.toolName, toolVersion: t.toolVersion })
                      }
                    >
                      {t.toolName}@{t.toolVersion}
                    </button>
                  );
                })}
              </div>
            )}
            <span className="builder-field__hint">
              The SERVER resolves the tool in its live registry + builds the per-step warrant from
              the tool's declared scope (you never supply a warrant — SN-8).
            </span>
          </div>
        ) : null}

        <div className="builder-field">
          <span className="builder-field__label">
            {step.kind === "tool" ? "Args (JSON)" : "Params (JSON)"}
          </span>
          <JsonEditor
            value={step.paramsText}
            onChange={(v) => onChange({ ...step, paramsText: v })}
            testId="step-config-params"
            ariaLabel={
              step.kind === "tool" ? "Tool args (JSON object)" : "Step params (JSON object)"
            }
            height={120}
          />
          {!paramsValid ? (
            <span className="builder-field__error" data-testid="step-config-params-error">
              {step.kind === "tool" ? "Args" : "Params"} must be a JSON object.
            </span>
          ) : null}
        </div>

        <div className="node-drawer__foot">
          <button
            type="button"
            className="linkbtn danger"
            data-testid="step-config-delete"
            onClick={onDelete}
          >
            Delete step
          </button>
        </div>
      </m.aside>
    </>
  );
}
