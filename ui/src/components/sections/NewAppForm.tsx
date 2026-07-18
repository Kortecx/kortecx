/**
 * POC-5a "New App" — an inline collapsible authoring panel (the NewBranchForm
 * precedent; NOT a dialog). It saves an App envelope (a single agentic step over
 * the goal) then launches the server-side agentic scaffold, handing off to the
 * honest {@link ScaffoldProgress} poller.
 *
 * The envelope carries NO authority (the server re-resolves warrants at run); the
 * optional model rides as a steering hint only. T-RUNAPP-CONTEXT-RAIL adds the
 * declarative knowledge rail: "Ground on dataset" chips (references.datasets → a
 * live `retrieve@1` grant at run, the App self-grounds) + a guidance rule
 * (references.rules → an entry-step context item). Both resolve server-side at
 * RunApp — the same rails the SDK `.dataset()` / `.rule()` authoring emits.
 *
 * By convention the App's project branch handle IS the saved App handle
 * (one-App-one-branch), so the scaffold + progress poll key on it.
 */

import { type WorkflowProposal, app, defaultHandle, flow } from "@kortecx/sdk/web";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { queryKeys } from "../../kx/query-keys";
import { useAttachments } from "../../kx/use-attachments";
import { useDatasets } from "../../kx/use-datasets";
import { useModels } from "../../kx/use-models";
import { useProposeWorkflow } from "../../kx/use-propose-workflow";
import { useScaffoldApp } from "../../kx/use-scaffold-app";
import { composeCapabilityPrompt } from "../../lib/app-capability-prompt";
import { FRESH_UNMODELED, builderGraphToBlueprint } from "../builder/app-blueprint";
import { proposalToBuilderGraph } from "../builder/builder-graph";
import { GlowCard } from "../ds/GlowCard";
import { ScaffoldProgress } from "./ScaffoldProgress";

type ProposedPlan = Extract<WorkflowProposal, { proposed: true }>;

/**
 * Bridge a multi-step NL WorkflowProposal → an App blueprint source. Reuses the SAME
 * persona-framing fold the visual builder applies (`proposalToBuilderGraph`), then lowers
 * it to the portable `DagSpecJson` the App envelope carries (`builderGraphToBlueprint`, the
 * same path the "Save as App" flow uses). The server still re-compiles + re-warrants the
 * DAG at RunApp (SN-8) — this only shapes what gets authored.
 */
function proposalBlueprint(plan: ProposedPlan) {
  const insert = proposalToBuilderGraph(plan.steps, plan.edges, 0);
  const dag = builderGraphToBlueprint(
    { steps: insert.steps, edges: insert.edges },
    FRESH_UNMODELED,
  );
  return { toBlueprint: () => dag };
}

/** Which lane the New App form authors (D213): a scheduled functional app or a hosted
 *  experience (web) app. */
export type NewAppKind = "scheduled" | "hosted";

export function NewAppForm({
  onClose,
  initialKind = "scheduled",
}: {
  onClose: () => void;
  initialKind?: NewAppKind;
}) {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  const scaffold = useScaffoldApp();
  const datasets = useDatasets();
  const models = useModels();
  const attach = useAttachments();
  const propose = useProposeWorkflow();
  const [kind, setKind] = useState<NewAppKind>(initialKind);
  const [proposal, setProposal] = useState<WorkflowProposal | null>(null);
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [prompt, setPrompt] = useState("");
  const [model, setModel] = useState("");
  const [handle, setHandle] = useState("");
  // T-RUNAPP-CONTEXT-RAIL authoring state (the declarative rail).
  const [grounding, setGrounding] = useState<string[]>([]);
  const [rule, setRule] = useState("");
  // Set once the scaffold has launched — switches the panel to the progress view.
  const [scaffolding, setScaffolding] = useState<{
    appHandle: string;
    branchHandle: string;
  } | null>(null);

  const save = useMutation<string, unknown, void>({
    mutationFn: async (): Promise<string> => {
      if (!client) {
        throw new Error("not connected");
      }
      // The App's agent step runs the PROMPT (the instruction); the GOAL is the
      // description. A comprehensive capability rule teaches the model what a Kortecx
      // App is + how to drive the runtime (tools/connections/datasets/skills/files).
      // Attachments already uploaded (PutContent) ride as by-reference context files.
      const readyFiles = attach.attachments.filter((a) => a.status === "ready" && a.ref);
      const trimmedModel = model.trim();
      const trimmedHandle = handle.trim();

      // D213 HOSTED (experience) app: no blueprint — the runtime scaffolds a real web
      // project into the app's branch and serves it on a local port. The branch handle IS
      // the app handle (one-app-one-branch), so it must be resolved up front.
      if (kind === "hosted") {
        const h = trimmedHandle !== "" ? trimmedHandle : defaultHandle(name.trim());
        let hb = app(name.trim())
          .describe(goal.trim())
          .hosted("auto", h)
          .rule("capabilities", {
            body: composeCapabilityPrompt(
              goal.trim(),
              readyFiles.map((a) => a.filename),
              "hosted",
            ),
          });
        if (trimmedModel !== "") {
          hb = hb.steer({ model: trimmedModel });
        }
        for (const a of readyFiles) {
          hb = hb.context(a.filename, a.ref as string, { mediaType: a.mediaType });
        }
        const hosted = await hb.save({ client, handle: h });
        return hosted.handle;
      }

      // SCHEDULED (functional) app: an agentic blueprint over the goal/prompt.
      const promptText = prompt.trim() || goal.trim();
      // 5b: if the author proposed + previewed a multi-step plan, author the App as that
      // MULTI-STEP blueprint (each step keeps its role/intent/model); otherwise fall back to
      // today's single agent step over the prompt. The capability rule co-lands on both.
      const proposed = proposal?.proposed === true && proposal.steps.length > 0 ? proposal : null;
      let builder = app(name.trim())
        .describe(goal.trim())
        .blueprint(proposed ? proposalBlueprint(proposed) : flow().agent(promptText))
        .rule("capabilities", {
          body: composeCapabilityPrompt(
            goal.trim(),
            readyFiles.map((a) => a.filename),
            "scheduled",
          ),
        });
      if (trimmedModel !== "") {
        builder = builder.steer({ model: trimmedModel });
      }
      for (const a of readyFiles) {
        builder = builder.context(a.filename, a.ref as string, { mediaType: a.mediaType });
      }
      for (const ds of grounding) {
        builder = builder.dataset(ds);
      }
      const trimmedRule = rule.trim();
      if (trimmedRule !== "") {
        builder = builder.rule("guidance", { body: trimmedRule });
      }
      const result = await builder.save({
        client,
        ...(trimmedHandle !== "" ? { handle: trimmedHandle } : {}),
      });
      return result.handle;
    },
    // Surface the new App in the catalog immediately (the scaffold, when it runs, also
    // invalidates on completion; a hosted app has no blueprint scaffold to wait for).
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    },
  });

  const nameOk = name.trim().length > 0;
  const goalOk = goal.trim().length > 0;
  const busy = save.isPending || scaffold.isPending;
  const canSubmit = nameOk && goalOk && !busy && !attach.uploading && scaffolding === null;

  // Only datasets with an indexed document can ground (honest: grounding turns on
  // AFTER you ingest). No dataset view (hnsw off) / none ingested ⇒ render nothing
  // (don't-fake-gaps, the DatasetPicker precedent).
  const groundable = (datasets.data ?? []).filter((d) => d.docCount > 0);

  function toggleDataset(nameOfDataset: string): void {
    setGrounding((cur) =>
      cur.includes(nameOfDataset)
        ? cur.filter((d) => d !== nameOfDataset)
        : [...cur, nameOfDataset],
    );
  }

  function onSubmit(e: FormEvent): void {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    save.mutate(undefined, {
      onSuccess: (appHandle) => {
        scaffold.mutate(
          { handle: appHandle, goal: goal.trim() },
          {
            onSuccess: ({ branchHandle }) => {
              setScaffolding({ appHandle, branchHandle });
            },
            onError: () => {
              // A hosted app is CREATED by SaveApp; scaffolding its page needs a served
              // model. Without one, the app still runs with the framework's default page —
              // close the form (the app is in the Hosted catalog). A scheduled app instead
              // surfaces the scaffold error (its blueprint is the whole app).
              if (kind === "hosted") {
                onClose();
              }
            },
          },
        );
      },
    });
  }

  const saveErr = save.error ? toUiError(save.error) : null;
  const scaffoldErr = scaffold.error ? toUiError(scaffold.error) : null;
  const proposeErr = propose.error ? toUiError(propose.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="new-app-form">
      <div className="new-app-form__head">
        <h2>New App</h2>
        <button
          type="button"
          className="linkbtn"
          data-testid="new-app-close"
          aria-label="Close New App"
          onClick={onClose}
        >
          ✕
        </button>
      </div>
      <p className="muted">
        Describe the App and its goal. We save a durable App envelope, then the agent scaffolds a
        starter project tree into the App's own content-addressed branch (the host is never
        written). Browse and edit it after.
      </p>

      {scaffolding === null ? (
        <form onSubmit={onSubmit} className="register-tool-form">
          <fieldset
            className="view-toggle view-toggle--compact"
            aria-label="App kind"
            data-testid="new-app-kind"
          >
            <button
              type="button"
              data-testid="new-app-kind-scheduled"
              aria-pressed={kind === "scheduled"}
              onClick={() => setKind("scheduled")}
              disabled={busy}
              title="An automation app — runs on a trigger / in workflows"
            >
              Scheduled
            </button>
            <button
              type="button"
              data-testid="new-app-kind-hosted"
              aria-pressed={kind === "hosted"}
              onClick={() => setKind("hosted")}
              disabled={busy}
              title="A hosted web app — scaffolded and served on a local port"
            >
              Hosted
            </button>
          </fieldset>
          {kind === "hosted" ? (
            <p className="muted" data-testid="new-app-hosted-note">
              A hosted web app: the runtime scaffolds a React / Next.js project from your
              description and serves it on a local port. The framework is chosen automatically.
            </p>
          ) : null}
          <input
            type="text"
            data-testid="new-app-name"
            placeholder="App name (e.g. Release Notes Writer)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="App name"
            disabled={busy}
          />
          <textarea
            className="input"
            data-testid="new-app-goal"
            placeholder="Goal — what should this App do? (e.g. 'Summarize a changelog into release notes')"
            rows={2}
            value={goal}
            onChange={(e) => {
              setGoal(e.target.value);
              setProposal(null); // a new goal invalidates any previewed plan
            }}
            aria-label="App goal"
            disabled={busy}
          />

          {/* 5b: NL multi-step authoring — ask the served model to plan a workflow for the
              goal, preview it, then author the App as that multi-step blueprint. Falls back
              to a single agent step when there is no plan (e.g. no served model). */}
          {kind === "scheduled" ? (
            <div className="register-tool-form__row">
              <button
                type="button"
                className="btn-ghost"
                data-testid="new-app-propose"
                onClick={() => propose.mutate(goal.trim(), { onSuccess: setProposal })}
                disabled={!goalOk || busy || propose.isPending}
                title="Plan a multi-step workflow for this goal. You preview it before authoring."
              >
                {propose.isPending ? "Proposing…" : "Propose steps"}
              </button>
              {proposal?.proposed === true ? (
                <button
                  type="button"
                  className="linkbtn"
                  data-testid="new-app-proposal-clear"
                  onClick={() => setProposal(null)}
                  disabled={busy}
                >
                  Clear plan (author a single step)
                </button>
              ) : null}
            </div>
          ) : null}
          {proposal?.proposed === true ? (
            <fieldset className="new-app-form__rail" data-testid="new-app-proposal">
              <legend className="muted">Proposed plan ({proposal.steps.length} steps)</legend>
              <ol>
                {proposal.steps.map((s, i) => (
                  <li key={`${s.role}-${i}`} data-testid={`new-app-proposal-step-${i}`}>
                    <strong>{s.role}</strong>
                    {s.modelId ? <span className="mono"> · {s.modelId}</span> : null}
                    <div className="muted">{s.intent}</div>
                  </li>
                ))}
              </ol>
            </fieldset>
          ) : proposal?.proposed === false ? (
            <output className="muted" data-testid="new-app-proposal-rejected">
              No multi-step plan: {proposal.reason} — authoring a single agent step over the goal.
            </output>
          ) : null}
          {proposeErr ? (
            <p className="field-error" data-testid="new-app-propose-error" role="alert">
              {proposeErr.message}
            </p>
          ) : null}

          {kind === "scheduled" ? (
            <textarea
              className="input"
              data-testid="new-app-prompt"
              placeholder="Prompt (optional) — the instruction the model runs each time. Defaults to the goal. A comprehensive capability prompt (how to use the runtime's tools, connections, datasets & files) is added automatically."
              rows={3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              aria-label="App prompt"
              disabled={busy}
            />
          ) : null}
          <div className="register-tool-form__row">
            <select
              className="mono"
              data-testid="new-app-model"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              aria-label="Model"
              disabled={busy}
              title="Pick a local model, or leave on the served default. External providers connect via Integrations."
            >
              <option value="">Served default</option>
              {(models.models ?? []).map((m) => (
                <option key={m.modelId} value={m.modelId}>
                  {m.modelId}
                  {m.modalities.includes("image") ? " (vision)" : ""}
                </option>
              ))}
            </select>
            <input
              type="text"
              className="mono"
              data-testid="new-app-handle"
              placeholder="handle (optional — derived from the name)"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
              aria-label="App handle (optional)"
              disabled={busy}
            />
          </div>

          {/* T-RUNAPP-CONTEXT-RAIL: "Ground on dataset" — chip toggles (a controlled
              <select> can't be Playwright-driven; chips are the standing pattern). Only
              shown when a non-empty dataset exists (don't-fake-gaps). */}
          {kind === "scheduled" && groundable.length > 0 ? (
            <fieldset className="new-app-form__rail" data-testid="new-app-datasets">
              <legend className="muted">Ground on dataset (RAG)</legend>
              <div className="chips">
                {groundable.map((d) => {
                  const on = grounding.includes(d.name);
                  return (
                    <button
                      key={d.datasetId}
                      type="button"
                      className={on ? "chip chip--active" : "chip"}
                      aria-pressed={on}
                      data-testid={`new-app-dataset-${d.name}`}
                      onClick={() => toggleDataset(d.name)}
                      disabled={busy}
                    >
                      {d.name} ({d.docCount})
                    </button>
                  );
                })}
              </div>
            </fieldset>
          ) : null}

          {kind === "scheduled" ? (
            <textarea
              className="input"
              data-testid="new-app-rule"
              placeholder="Guidance rule (optional) — a behavior note the agent must follow (e.g. 'Always cite sources.')"
              rows={2}
              value={rule}
              onChange={(e) => setRule(e.target.value)}
              aria-label="Guidance rule (optional)"
              disabled={busy}
            />
          ) : null}

          {/* Attachments — uploaded (PutContent) and attached to the App as
              by-reference context files the model reads at run. */}
          <fieldset className="new-app-form__rail" data-testid="new-app-attachments">
            <legend className="muted">Attachments (context files)</legend>
            <input
              type="file"
              multiple
              data-testid="new-app-attach-input"
              onChange={(e) => {
                if (e.target.files && e.target.files.length > 0) {
                  attach.addFiles(e.target.files);
                  e.target.value = "";
                }
              }}
              aria-label="Attach context files"
              disabled={busy}
            />
            {attach.attachments.length > 0 ? (
              <div className="chips">
                {attach.attachments.map((a) => (
                  <span
                    key={a.id}
                    className="chip"
                    data-testid={`new-app-attachment-${a.filename}`}
                  >
                    {a.filename}
                    {a.status !== "ready" ? " · uploading…" : ""}
                    <button
                      type="button"
                      className="context-strip__remove"
                      aria-label={`Remove ${a.filename}`}
                      onClick={() => attach.remove(a.id)}
                      disabled={busy}
                    >
                      ✕
                    </button>
                  </span>
                ))}
              </div>
            ) : null}
          </fieldset>

          <div className="register-tool-form__row">
            <button type="submit" data-testid="new-app-submit" disabled={!canSubmit}>
              {busy ? "Scaffolding…" : "Create & scaffold"}
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="new-app-cancel"
              onClick={onClose}
              disabled={busy}
            >
              Cancel
            </button>
          </div>
          {kind === "scheduled" ? (
            <p className="muted">
              Prefer to compose the structure yourself?{" "}
              <Link to="/blueprints/new" className="linkbtn" data-testid="new-app-build-visual">
                Build in the visual builder →
              </Link>
            </p>
          ) : null}
        </form>
      ) : (
        <ScaffoldProgress
          branchHandle={scaffolding.branchHandle}
          appHandle={scaffolding.appHandle}
        />
      )}

      {saveErr ? (
        <p className="field-error" data-testid="new-app-save-error" role="alert">
          {saveErr.message}
        </p>
      ) : null}
      {scaffoldErr ? (
        <p className="field-error" data-testid="new-app-scaffold-error" role="alert">
          {scaffoldErr.message}
        </p>
      ) : null}
    </GlowCard>
  );
}
