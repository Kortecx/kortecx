/**
 * The `/workflows/create` screen — author a DURABLE Workflow on one surface: a
 * header form (name · description · handle · save-as-draft) over the builder
 * canvas in `embedded` mode (the NewAppForm seam — the host owns the terminal
 * action, so none of the builder's own navigations can fire).
 *
 * Save lowers the live graph via `builderGraphToBlueprint` into a
 * `kortecx.workflow/v1` envelope and `SaveWorkflow`s it, then lands on the
 * definition page. `?handle=` seeds the form + canvas from the stored envelope
 * (`GetWorkflow` → `appBlueprintToBuilderGraph`) — the edit / finish-draft
 * entry; a re-save is LOSSLESS outside what this screen edits (references /
 * steering_config / replay / tags ride verbatim, and the parse's unmodeled
 * snapshot re-merges the blueprint-level fields BuilderGraph does not carry).
 *
 * Deliberately NO scaffold machinery: the save IS the authoring act — nothing
 * asynchronous exists to watch or resume (no CreateAppScreen state machine).
 */

import { useNavigate } from "@tanstack/react-router";
import { Suspense, lazy, useCallback, useState } from "react";
import { toUiError } from "../../kx/errors";
import { WORKFLOW_SCHEMA, useSaveWorkflow, useWorkflow } from "../../kx/use-workflows";
import { EmptyState } from "../EmptyState";
import {
  FRESH_UNMODELED,
  type UnmodeledReport,
  appBlueprintToBuilderGraph,
  builderGraphToBlueprint,
} from "../builder/app-blueprint";
import type { BuilderGraph } from "../builder/builder-graph";
import { validationError } from "../builder/builder-graph";

// The canvas (reactflow + dagre) loads only here — the route module stays eager-light.
const BlueprintBuilderSection = lazy(() =>
  import("./BlueprintBuilderSection").then((m) => ({
    default: m.BlueprintBuilderSection,
  })),
);

/** Derive the default 3-segment catalog handle `workflows/local/<sanitized>` from a
 *  Workflow name (the SDK `defaultHandle` rules under the workflows namespace). */
export function defaultWorkflowHandle(name: string): string {
  let san = "";
  for (const c of name) {
    if (/[a-z0-9._-]/.test(c)) {
      san += c;
    } else if (/[A-Z]/.test(c)) {
      san += c.toLowerCase();
    } else {
      san += "-";
    }
  }
  san = san
    .replace(/^[.-]+/, "")
    .replace(/[.-]+$/, "")
    .slice(0, 128);
  return `workflows/local/${san || "workflow"}`;
}

/** The seeded parse — the graph the canvas mounts with plus the unmodeled snapshot
 *  the save re-merges (the lossless rule). */
interface SeedState {
  readonly graph: BuilderGraph;
  readonly unmodeled: UnmodeledReport;
  readonly envelope: Record<string, unknown>;
}

export function CreateWorkflowScreen({ seedHandle = null }: { seedHandle?: string | null }) {
  const navigate = useNavigate();
  const save = useSaveWorkflow();
  const stored = useWorkflow(seedHandle);

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [handleDraft, setHandleDraft] = useState("");
  // The handle follows the name until the user edits it (then their word stands).
  const [handleTouched, setHandleTouched] = useState(false);
  const [draft, setDraft] = useState(false);
  // The LIVE canvas graph — what the author sees is what gets lowered at save.
  const [graph, setGraph] = useState<BuilderGraph | null>(null);
  const onGraphChange = useCallback((g: BuilderGraph) => setGraph(g), []);
  const [seed, setSeed] = useState<SeedState | null>(null);
  // The builder seeds its node state ONCE (`useNodesState` is `useState` underneath),
  // so a later seed needs a remount — keying the mount is the honest way to say
  // "this is a different starting graph" (the NewAppForm precedent).
  const [seedKey, setSeedKey] = useState(0);
  const [seedApplied, setSeedApplied] = useState(false);

  // Apply the stored envelope ONCE when it arrives (render-phase state seeding —
  // no effect needed; the query result is the only input).
  if (seedHandle !== null && !seedApplied && stored.data !== undefined) {
    setSeedApplied(true);
    if (stored.data !== null) {
      const env = stored.data.envelope as Record<string, unknown>;
      const parsed = appBlueprintToBuilderGraph((env.blueprint ?? { seed: 0, steps: [] }) as never);
      setName(typeof env.name === "string" ? env.name : "");
      setDescription(typeof env.description === "string" ? env.description : "");
      setHandleDraft(seedHandle);
      setHandleTouched(true);
      setDraft(stored.data.lifecycle === "draft");
      setSeed({ graph: parsed.graph, unmodeled: parsed.unmodeled, envelope: env });
      setGraph(parsed.graph);
      setSeedKey((k) => k + 1);
    }
  }

  const handle = handleTouched ? handleDraft : defaultWorkflowHandle(name);
  const refuseEdit = seed?.unmodeled.refuseEdit === true;
  // A Workflow (like a portable App) may leave a model step blank — the SERVER
  // binds the served model / the envelope's model_route at run.
  const invalid =
    graph === null
      ? "Add at least one step."
      : validationError(graph, {
          allowEmptyModel: true,
        });
  const canSave =
    name.trim() !== "" &&
    handle.trim() !== "" &&
    invalid === null &&
    !refuseEdit &&
    !save.isPending;

  function onSave(): void {
    if (!canSave || graph === null) {
      return;
    }
    const blueprint = builderGraphToBlueprint(graph, seed?.unmodeled ?? FRESH_UNMODELED);
    // Lossless outside this screen's edits: a seeded envelope's other regions
    // (references / steering_config / replay / tags / input_schema) ride verbatim.
    const { description: _drop, ...base } = seed?.envelope ?? {};
    const envelope: Record<string, unknown> = {
      ...base,
      schema: WORKFLOW_SCHEMA,
      name: name.trim(),
      version: typeof base.version === "string" && base.version !== "" ? base.version : "1",
      blueprint,
    };
    if (description.trim() !== "") {
      envelope.description = description.trim();
    }
    save.mutate(
      { handle, envelope, lifecycle: draft ? "draft" : "" },
      {
        onSuccess: ({ handle: saved }) =>
          void navigate({ to: "/workflows/def/$handle", params: { handle: saved } }),
      },
    );
  }

  const editing = seedHandle !== null;
  if (editing && !seedApplied) {
    return (
      <section className="screen" data-testid="workflows-create">
        <EmptyState title="Loading workflow…" />
      </section>
    );
  }

  return (
    <section className="screen" data-testid="workflows-create">
      <div className="section-head">
        <div>
          <h1>{editing ? "Edit workflow" : "New workflow"}</h1>
          <p className="muted">
            A durable, reusable workflow — saved to your catalog, runnable and schedulable by
            handle. The server compiles the DAG and builds every warrant at run; the envelope
            carries no authority.
          </p>
        </div>
      </div>

      {editing && stored.data === null ? (
        <p className="field-hint" data-testid="workflow-seed-missing" aria-live="polite">
          No workflow is stored at <code className="mono">{seedHandle}</code> — starting fresh.
          Saving records a new workflow.
        </p>
      ) : null}
      {refuseEdit ? (
        <p className="field-error" data-testid="workflow-refuse-edit" role="alert">
          {seed?.unmodeled.reason ??
            "This workflow's blueprint can't be safely edited here — edit it via the SDK/CLI."}
        </p>
      ) : null}

      <div className="register-tool-form" data-testid="workflow-create-form">
        <fieldset className="new-app-form__rail">
          <legend className="muted">Name</legend>
          <input
            type="text"
            data-testid="workflow-name"
            placeholder="e.g. Morning digest"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Workflow name"
            maxLength={80}
            disabled={save.isPending}
          />
        </fieldset>
        <fieldset className="new-app-form__rail">
          <legend className="muted">Description</legend>
          <input
            type="text"
            data-testid="workflow-description"
            placeholder="What one run of this workflow produces (optional)"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            aria-label="Workflow description"
            maxLength={200}
            disabled={save.isPending}
          />
        </fieldset>
        <fieldset className="new-app-form__rail">
          <legend className="muted">Handle</legend>
          <input
            type="text"
            data-testid="workflow-handle"
            value={handle}
            onChange={(e) => {
              setHandleTouched(true);
              setHandleDraft(e.target.value);
            }}
            aria-label="Workflow handle"
            spellCheck={false}
            autoComplete="off"
            maxLength={160}
            disabled={save.isPending}
          />
        </fieldset>
        <label className="muted" data-testid="workflow-draft-label">
          <input
            type="checkbox"
            data-testid="workflow-draft"
            checked={draft}
            onChange={(e) => setDraft(e.target.checked)}
            disabled={save.isPending}
          />{" "}
          Save as draft — not schedulable yet; the catalog offers “Finish draft” instead of Run
        </label>

        <fieldset className="new-app-form__rail" data-testid="workflow-structure">
          <legend className="muted">
            Structure
            {graph !== null
              ? ` (${graph.steps.length} step${graph.steps.length === 1 ? "" : "s"})`
              : ""}
          </legend>
          <Suspense fallback={<p className="muted">Loading the builder…</p>}>
            <BlueprintBuilderSection
              key={seedKey}
              mode={{ kind: "embedded" }}
              initialGraph={seed?.graph ?? undefined}
              onGraphChange={onGraphChange}
            />
          </Suspense>
        </fieldset>

        {save.isError ? (
          <p className="field-error" data-testid="workflow-save-error" role="alert">
            {toUiError(save.error).message}
          </p>
        ) : null}

        <div className="new-app-form__actions">
          <button
            type="button"
            className="btn-primary"
            data-testid="workflow-save"
            disabled={!canSave}
            title={
              name.trim() === ""
                ? "Name the workflow first"
                : (invalid ?? "Save this workflow to your catalog")
            }
            onClick={onSave}
          >
            {save.isPending ? "Saving…" : draft ? "Save draft" : "Save workflow"}
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="workflow-cancel"
            disabled={save.isPending}
            onClick={() => void navigate({ to: "/workflows" })}
          >
            Cancel
          </button>
        </div>
      </div>
    </section>
  );
}
