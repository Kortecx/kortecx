/**
 * The builder's step-config drawer (D141.6 n8n-level node config). A right-side
 * panel — the editable counterpart of the read-only `NodeDetailDrawer` (reuses
 * the `.node-drawer` design language) — to configure one authored step: its label,
 * and for an AGENT (MODEL) step the served model, the prompt (Monaco), the opt-in
 * reasoning-mode, plus typed free-params (Monaco JSON). No fake knobs: a control
 * appears only where the wire enforces it (don't-fake-gaps, D142).
 */

import { type ModelSummary, PERSONAS, personaNames } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useListMcpServers } from "../../kx/use-connections";
import { useDatasets } from "../../kx/use-datasets";
import { useListSkills } from "../../kx/use-skills";
import { useDiscoverTools } from "../../kx/use-tool-registry";
import { JsonEditor } from "../editor/JsonEditor";
import { MonacoMount } from "../editor/MonacoMount";
import type { BuilderStep } from "./builder-graph";
import { isJsonObject } from "./builder-graph";

/** The per-node capability axes, in display order. `field` is the {@link BuilderStep} key
 *  each one toggles; `empty` is what to say when the account has none of that thing —
 *  naming where to add one, since an empty group with no explanation reads as broken. */
const CAPABILITY_AXES = [
  {
    field: "skills",
    label: "Skills",
    hint: "Instructions plus a tool wish. Attached here, they reach THIS step — its prompt and its loop — not the whole app.",
    empty: (
      <>
        No skills in the catalog. Add one in <strong>Skills</strong>.
      </>
    ),
  },
  {
    field: "connections",
    label: "Integrations",
    hint: "Only this step may dial the connector, and only this step's warrant carries its credential scope.",
    empty: (
      <>
        No integrations connected. Dial one in <strong>Tools → Connections</strong>.
      </>
    ),
  },
  {
    field: "datasets",
    label: "Grounding",
    hint: "This step gets `retrieve` over the dataset and is steered to search it before answering.",
    empty: (
      <>
        No dataset holds an indexed document yet. Ingest one in <strong>Datasets</strong>.
      </>
    ),
  },
] as const satisfies ReadonlyArray<{
  field: "skills" | "connections" | "datasets";
  label: string;
  hint: string;
  empty: React.ReactNode;
}>;

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
  appCapabilities = false,
  onChange,
  onDelete,
  onClose,
}: {
  step: BuilderStep;
  models: readonly ModelSummary[] | undefined;
  modelsUnsupported: boolean;
  /** Offer the per-node App capability axes (skills / integrations / grounding).
   *
   *  OFF by default, because this drawer is shared with the standalone workflow builder and
   *  a plain `SubmitWorkflow` has no `references` rail for a name to point at — the lowering
   *  refuses one outright. Showing the controls there would be a rail the runtime cannot
   *  honour. The App canvas turns them on. */
  appCapabilities?: boolean;
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
  // The live catalogs the per-node capability chips are drawn from. Hooks are
  // unconditional (rules of hooks); each query is already mounted elsewhere on the Apps
  // route, so these resolve from cache rather than costing a round trip.
  const skillCatalog = useListSkills();
  const serverRegistry = useListMcpServers();
  const datasets = useDatasets();
  /** The available NAMES per axis. Grounding offers only datasets that hold an indexed
   *  document — an empty one would ground the step on nothing, which is the same honesty
   *  rule the App dataset rail already applies. */
  const available: Record<(typeof CAPABILITY_AXES)[number]["field"], string[]> = {
    skills: skillCatalog.skills.map((s) => s.name),
    connections: serverRegistry.servers.map((s) => s.endpoint),
    datasets: (datasets.data ?? []).filter((d) => d.docCount > 0).map((d) => d.name),
  };
  /** A connector's endpoint is what the envelope binds and what the runtime dials, but it
   *  is not what a person recognises — show the registered name and bind the endpoint. */
  const chipLabel = (field: string, value: string): string =>
    field === "connections"
      ? (serverRegistry.servers.find((s) => s.endpoint === value)?.serverName ?? value)
      : value;

  // POC-C5: portal to <body> with the `--overlay` variants (the #330 pattern, per
  // `SaveAsAppDialog`/`BlueprintFormDrawer`). The drawer is a SIBLING of the builder
  // canvas inside a non-positioned `.screen`/`.shell__main`, so the plain canvas-scoped
  // `.node-drawer` (absolute, z5) fell to the document origin and the sticky navbar (z10)
  // clipped it. `--overlay` is `position:fixed` z49/z50 — it clears the navbar. Base
  // `.node-drawer__scrim` (shared by the canvas-scoped builder/DAG drawers) is untouched.
  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close step config"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
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
            {/* NL-primary (D209.3): the agent is authored FIRST in natural language — the
                instruction leads; the model + runtime steer how it runs. Model / persona /
                reasoning / tools are secondary knobs below. */}
            <div className="builder-field">
              <span className="builder-field__label">Instruction</span>
              <MonacoMount
                value={step.prompt}
                language="plaintext"
                onChange={(v) => onChange({ ...step, prompt: v })}
                height={160}
                testId="step-config-prompt"
                ariaLabel="Agent instruction"
                placeholder="Describe what this agent should do — the runtime + model decide how."
              />
              <span className="builder-field__hint">
                The primary control: author the agent in natural language. Pick a persona to prepend
                a curated role framing; the knobs below refine model, reasoning, and tools.
              </span>
            </div>

            <div className="builder-field">
              <span className="builder-field__label">Persona</span>
              <div className="builder-chips" data-testid="step-config-persona">
                {personaNames().map((name) => (
                  <button
                    key={name}
                    type="button"
                    className="chip"
                    data-testid={`step-config-persona-${name}`}
                    onClick={() => {
                      const role = PERSONAS[name] ?? "";
                      // Strip a leading persona (any known role) first, so re-picking a
                      // persona SWAPS the role rather than stacking it.
                      let body = step.prompt;
                      for (const known of Object.values(PERSONAS)) {
                        if (body === known) {
                          body = "";
                          break;
                        }
                        if (body.startsWith(`${known}\n\n`)) {
                          body = body.slice(known.length + 2);
                          break;
                        }
                      }
                      body = body.trim();
                      onChange({ ...step, prompt: body ? `${role}\n\n${body}` : role });
                    }}
                  >
                    {name}
                  </button>
                ))}
              </div>
              <span className="builder-field__hint">
                A curated role. Clicking prepends its instructions to the instruction (an editable
                template); the same as the SDK <code>kx.persona("…")</code>.
              </span>
            </div>

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

            <div className="builder-field">
              <span className="builder-field__label">Tools (agentic loop)</span>
              {toolsNotWired || tools.length === 0 ? (
                <p className="muted" data-testid="step-config-no-agent-tools">
                  No tools are registered. Register one in <strong>Tools → Registry</strong> or dial
                  an external MCP server in <strong>Tools → Connections</strong> to grant a set
                  here.
                </p>
              ) : (
                <div className="builder-chips" data-testid="step-config-agent-tools">
                  {tools.map((t) => {
                    const granted = step.toolContract[t.toolName] === t.toolVersion;
                    return (
                      <button
                        key={`grant-${t.toolName}@${t.toolVersion}`}
                        type="button"
                        className={`chip${granted ? " chip--active" : ""}`}
                        title={t.description}
                        aria-pressed={granted}
                        onClick={() => {
                          const next = { ...step.toolContract };
                          if (next[t.toolName] === t.toolVersion) {
                            delete next[t.toolName];
                          } else {
                            next[t.toolName] = t.toolVersion;
                          }
                          onChange({ ...step, toolContract: next });
                        }}
                      >
                        {t.toolName}@{t.toolVersion}
                      </button>
                    );
                  })}
                </div>
              )}
              <span className="builder-field__hint">
                Grant a FIXED tool set to run a bounded reason→tool→observe loop (the set is part of
                the step's identity). The SERVER builds the union warrant + drives the loop (SN-8).
              </span>
            </div>

            {/* THE PER-NODE CAPABILITY AXES. A node says what it does, what it may reach,
                and what it knows — so the graph is the whole authoring surface and there
                is no rail beside it that has to be kept in sync. */}
            {appCapabilities
              ? CAPABILITY_AXES.map((axis) => {
                  const picked = step[axis.field];
                  const options = available[axis.field];
                  return (
                    <div className="builder-field" key={axis.field}>
                      <span className="builder-field__label">{axis.label}</span>
                      {options.length === 0 ? (
                        <p className="muted" data-testid={`step-config-no-${axis.field}`}>
                          {axis.empty}
                        </p>
                      ) : (
                        <div className="builder-chips" data-testid={`step-config-${axis.field}`}>
                          {options.map((name) => {
                            const on = picked.includes(name);
                            return (
                              <button
                                key={name}
                                type="button"
                                className={`chip${on ? " chip--active" : ""}`}
                                aria-pressed={on}
                                data-testid={`step-config-${axis.field}-${name}`}
                                onClick={() =>
                                  onChange({
                                    ...step,
                                    [axis.field]: on
                                      ? picked.filter((x) => x !== name)
                                      : [...picked, name],
                                  })
                                }
                              >
                                {chipLabel(axis.field, name)}
                              </button>
                            );
                          })}
                        </div>
                      )}
                      <span className="builder-field__hint">{axis.hint}</span>
                    </div>
                  );
                })
              : null}

            {Object.keys(step.toolContract).length > 0 ? (
              <>
                <label className="builder-field">
                  <span className="builder-field__label">Max turns</span>
                  <input
                    className="builder-input"
                    type="number"
                    min={2}
                    max={8}
                    data-testid="step-config-max-turns"
                    value={step.maxTurns ?? 8}
                    onChange={(e) =>
                      onChange({
                        ...step,
                        maxTurns: Number.parseInt(e.target.value, 10) || undefined,
                      })
                    }
                  />
                </label>
                <label className="builder-field">
                  <span className="builder-field__label">Max tool calls</span>
                  <input
                    className="builder-input"
                    type="number"
                    min={1}
                    max={7}
                    data-testid="step-config-max-tool-calls"
                    value={step.maxToolCalls ?? 6}
                    onChange={(e) =>
                      onChange({
                        ...step,
                        maxToolCalls: Number.parseInt(e.target.value, 10) || undefined,
                      })
                    }
                  />
                  <span className="builder-field__hint">
                    Bounded loop: <code>0 &lt; tool-calls &lt; turns ≤ 8</code> (a turn is left to
                    read the last observation and answer). Defaults 8 / 6.
                  </span>
                </label>
              </>
            ) : null}
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
    </>,
    document.body,
  );
}
