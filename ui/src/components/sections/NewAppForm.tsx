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

import { Reach, type WorkflowProposal, app, defaultHandle, flow } from "@kortecx/sdk/web";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type FormEvent, Suspense, lazy, useCallback, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { queryKeys } from "../../kx/query-keys";
import { useApps } from "../../kx/use-apps";
import { useAttachments } from "../../kx/use-attachments";
import { useDatasets } from "../../kx/use-datasets";
import { useModels } from "../../kx/use-models";
import { useProposeWorkflow } from "../../kx/use-propose-workflow";
import { useScaffoldApp } from "../../kx/use-scaffold-app";
import { composeCapabilityPrompt, composeProposeGoal } from "../../lib/app-capability-prompt";
import type { ConnectionEntry } from "../../lib/app-envelope";
import { collidingHandle } from "../../lib/app-handle";
import {
  ConnectionsPicker,
  type PickedSkill,
  SkillsPicker,
  ToolsPicker,
} from "../apps/CapabilityPickers";
import { FRESH_UNMODELED, builderGraphToBlueprint } from "../builder/app-blueprint";
import { type BuilderGraph, proposalToBuilderGraph } from "../builder/builder-graph";
import { GlowCard } from "../ds/GlowCard";
import { ScaffoldProgress } from "./ScaffoldProgress";

/**
 * The visual builder canvas, MOUNTED HERE as the structure surface for a scheduled App.
 *
 * MUST stay `lazy`. This form is statically imported by AppsSection, so a static import
 * would pull the builder + @xyflow + dagre (~248 KB) onto the Apps route for everyone who
 * merely opens the catalog. Lazy, the canvas chunk loads when the form is opened.
 */
const BlueprintBuilderSection = lazy(() =>
  import("./BlueprintBuilderSection").then((m) => ({
    default: m.BlueprintBuilderSection,
  })),
);

/**
 * The step kinds an APP's canvas offers. Agent + Tool: an App is a governed automation,
 * and the pattern macros (swarm / supervisor / consensus) belong to the workflow builder,
 * where a one-shot run is the point. The standalone route keeps all three kinds and every
 * macro — `palette` is omitted there.
 */
const APP_PALETTE = ["model", "tool"] as const;

type ProposedPlan = Extract<WorkflowProposal, { proposed: true }>;

/**
 * Seed the canvas from a multi-step NL WorkflowProposal, via the SAME persona-framing fold
 * the visual builder applies (`proposalToBuilderGraph`).
 *
 * This used to lower the proposal straight to a `DagSpecJson` and discard the graph, so a
 * proposed plan could be previewed as a list and never edited. Producing the GRAPH instead
 * means the plan lands on the canvas the user can rearrange, and the lowering happens once
 * at save from whatever they ended up with.
 */
function proposalGraph(plan: ProposedPlan): BuilderGraph {
  const insert = proposalToBuilderGraph(plan.steps, plan.edges, 0);
  return { steps: insert.steps, edges: insert.edges };
}

/** Which lane the New App form authors (D213): a scheduled functional app or a hosted
 *  experience (web) app. */
export type NewAppKind = "scheduled" | "hosted";

/** The hosted-lane framework choices, in display order. `auto` lets the model pick; the
 *  others pin a concrete scaffold template. The `value` is the stable wire label the
 *  SDK's `.hosted(framework, …)` and the server template registry share. */
const HOSTED_FRAMEWORKS = [
  { value: "auto", label: "Auto" },
  { value: "vite_react", label: "React" },
  { value: "next_js", label: "Next.js" },
  { value: "svelte", label: "Svelte" },
] as const;
type HostedFrameworkChoice = (typeof HOSTED_FRAMEWORKS)[number]["value"];

/** How a scheduled App is authored (the second axis, orthogonal to {@link NewAppKind}):
 *  `contextual` = a text app steered by its own markdown; `codified` = the model authors the
 *  code and configuration the runtime orchestrates from. */
export type NewAppMode = "contextual" | "codified";

export function NewAppForm({
  onClose,
  initialKind = "scheduled",
  onKindAuthored,
}: {
  onClose: () => void;
  initialKind?: NewAppKind;
  /** Called with the kind the App was actually SAVED as, so the catalog can follow it. The
   *  form's kind toggle is local state, so without this a hosted app authored from the
   *  Scheduled tab lands in a section the user is not looking at. */
  onKindAuthored?: (kind: NewAppKind) => void;
}) {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  const scaffold = useScaffoldApp();
  const datasets = useDatasets();
  const models = useModels();
  // The live catalog, for the handle-collision check. `AppsSection` renders this form and
  // already holds this query, so it resolves from cache — no extra round trip.
  const { apps } = useApps();
  const attach = useAttachments();
  const propose = useProposeWorkflow();
  const [kind, setKind] = useState<NewAppKind>(initialKind);
  const [mode, setMode] = useState<NewAppMode>("contextual");
  const [proposal, setProposal] = useState<WorkflowProposal | null>(null);
  // The LIVE canvas graph. Previously a proposal was lowered once, inline, and the
  // editable graph thrown away — so the plan could be previewed but never adjusted.
  const [graph, setGraph] = useState<BuilderGraph | null>(null);
  // A stable callback: `onGraphChange` fires from an effect in the builder, so an inline
  // arrow would re-run it every render.
  const onGraphChange = useCallback((g: BuilderGraph) => setGraph(g), []);
  // Bumped whenever a NEW proposal must land on the canvas. The builder seeds its node
  // state ONCE (`useNodesState` is `useState` underneath), so a changed `initialGraph`
  // prop alone would be silently ignored and the proposed plan would never appear.
  // Keying the mount is the honest way to say "this is a different starting graph".
  const [seedNonce, setSeedNonce] = useState(0);
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [prompt, setPrompt] = useState("");
  const [model, setModel] = useState("");
  // The hosted-lane framework selector (ignored for the scheduled lane).
  const [framework, setFramework] = useState<HostedFrameworkChoice>("auto");
  // T-RUNAPP-CONTEXT-RAIL authoring state (the declarative rail).
  const [grounding, setGrounding] = useState<string[]>([]);
  const [rule, setRule] = useState("");
  // The CAPABILITY rails. This form had no capability call sites at all, so every
  // console-authored App saved with `references.tools == []` / `references.skills == []`
  // and no connection — it ran with nothing plugged in and the only way to fix it was to
  // create the App, open it, and attach afterwards. These three pieces of state are the
  // same values the detail page's rails hold; they fold into the SDK builder at save
  // (`.useTool()` / `.skill()` / `.withConnection()` / `.steer({ reach })`), which is
  // exactly what `app_run.rs` reads back at RunApp.
  const [toolGrants, setToolGrants] = useState<Record<string, string>>({});
  const [reachInherit, setReachInherit] = useState(false);
  const [skills, setSkills] = useState<PickedSkill[]>([]);
  const [connections, setConnections] = useState<ConnectionEntry[]>([]);
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

      // D213 HOSTED (experience) app: no blueprint — the runtime scaffolds a real web
      // project into the app's branch and serves it on a local port. The branch handle IS
      // the app handle (one-App-one-branch), derived from the name (no handle field).
      if (kind === "hosted") {
        const h = defaultHandle(name.trim());
        let hb = app(name.trim())
          .describe(goal.trim())
          .hosted(framework, h)
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
      // Author from the LIVE canvas when it holds steps — what the user sees is what is
      // saved, including anything they rearranged after proposing. Falls back to the
      // single agent step over the prompt when the canvas is empty.
      const lowered =
        graph !== null && graph.steps.length > 0
          ? { toBlueprint: () => builderGraphToBlueprint(graph, FRESH_UNMODELED) }
          : null;
      let builder = app(name.trim())
        .describe(goal.trim())
        .blueprint(lowered ?? flow().agent(promptText))
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
      // The capability rails, in the shape RunApp consumes: `useTool` writes the wish to
      // `steering_config.tools.requested_grants` (mirrored to `references.tools`), `skill`
      // writes `references.skills`, and `withConnection` writes `references.connections`
      // PLUS the credential NAME into `guards.secret_scope` (never the secret, D81). Every
      // one is a wish — the server still resolves `wish ∩ grants ∩ fireable` at run (SN-8).
      // Scheduled-lane only: the hosted supervisor reads the framework + commands and
      // nothing else, so a hosted App carrying these would be authoring dead weight.
      for (const [toolId, toolVersion] of Object.entries(toolGrants)) {
        builder = builder.useTool(toolId, toolVersion);
      }
      for (const s of skills) {
        builder = builder.skill(s);
      }
      for (const c of connections) {
        builder = builder.withConnection(c.descriptor, c.credential_ref);
      }
      if (reachInherit) {
        builder = builder.steer({ reach: Reach.InheritPrincipal });
      }
      // Only a non-default mode is written: leaving it off emits no `mode` key at all, so a
      // contextual App's envelope bytes — and its app_ref — are what they always were.
      if (mode === "codified") {
        builder = builder.mode("codified");
      }
      const trimmedRule = rule.trim();
      if (trimmedRule !== "") {
        builder = builder.rule("guidance", { body: trimmedRule });
      }
      const result = await builder.save({ client });
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
  // The App's catalog key is DERIVED from its name and `SaveApp` is an upsert on that
  // key, so saving over an existing handle replaces that App's envelope, its capability
  // rails, and whatever a trigger already points at — with no warning. Block instead of
  // auto-suffixing: `my-agent-2` leaves the user unable to tell which App is theirs.
  // Read from the catalog `AppsSection` has already loaded, so this costs no request.
  const collision = collidingHandle(apps, name);
  const canSubmit =
    nameOk && goalOk && collision === null && !busy && !attach.uploading && scaffolding === null;

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
    // Capture the kind the save was issued under — the catalog follows it, so a hosted app
    // authored from the Scheduled tab lands in the section the user ends up looking at.
    const authoredKind = kind;
    save.mutate(undefined, {
      onSuccess: (appHandle) => {
        onKindAuthored?.(authoredKind);
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
      <p className="muted" data-testid="new-app-lede">
        {kind === "hosted"
          ? "Describe the web app and its goal. We save a durable App envelope, then scaffold a real framework project into the App's own content-addressed branch — browse, edit, and serve it on a local port."
          : "Describe the App and its goal. We save a durable App envelope, then the agent plans a complete project tailored to your goal and scaffolds it — streaming each file in live — into the App's own content-addressed branch. Browse and edit it after."}
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
          {/* The authoring-mode axis. Codified is honest-DISABLED: the envelope carries the
              field and the catalog reads it, but the scheduled scaffold rail still authors
              markdown only — so saving `codified` today would put a "Codified" chip on an app
              that is prose. Offered now (the `new-app-packaging-docker` precedent) so the axis
              is discoverable, enabled when the rail lands. */}
          {kind === "scheduled" ? (
            <fieldset
              className="view-toggle view-toggle--compact"
              aria-label="App mode"
              data-testid="new-app-mode"
            >
              <button
                type="button"
                data-testid="new-app-mode-contextual"
                aria-pressed={mode === "contextual"}
                onClick={() => setMode("contextual")}
                disabled={busy}
                title="A text app: its own prompt, rules and reference notes steer the model"
              >
                Contextual
              </button>
              <button
                type="button"
                data-testid="new-app-mode-codified"
                aria-pressed={false}
                disabled
                title="Codified — the model writes the code and configuration this app is orchestrated from. Ships with the codified scaffold rail."
              >
                Codified · soon
              </button>
            </fieldset>
          ) : null}
          {kind === "hosted" ? (
            <>
              <p className="muted" data-testid="new-app-hosted-note">
                A hosted web app: the runtime scaffolds a real framework project from your
                description and serves it on a local port. Pick a framework, or leave it on Auto.
              </p>
              <fieldset
                className="new-app-form__rail"
                aria-label="Framework"
                data-testid="new-app-framework"
              >
                <legend className="muted">Framework</legend>
                <div className="chips">
                  {HOSTED_FRAMEWORKS.map((fw) => {
                    const on = framework === fw.value;
                    return (
                      <button
                        key={fw.value}
                        type="button"
                        className={on ? "chip chip--active" : "chip"}
                        aria-pressed={on}
                        data-testid={`new-app-framework-${fw.value}`}
                        onClick={() => setFramework(fw.value)}
                        disabled={busy}
                      >
                        {fw.label}
                      </button>
                    );
                  })}
                </div>
              </fieldset>
            </>
          ) : null}
          <input
            type="text"
            data-testid="new-app-name"
            placeholder="App name (e.g. Release Notes Writer)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="App name"
            maxLength={80}
            disabled={busy}
            aria-invalid={collision !== null}
            aria-describedby={collision !== null ? "new-app-name-collision" : undefined}
          />
          {collision !== null ? (
            <p
              id="new-app-name-collision"
              className="field-error"
              data-testid="new-app-name-collision"
              role="alert"
            >
              An App already exists at <code>{collision}</code>. Saving would replace it — pick a
              different name.
            </p>
          ) : null}
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

          {kind === "scheduled" ? (
            <textarea
              className="input"
              data-testid="new-app-prompt"
              placeholder="Prompt (optional) — the instruction the model runs each time. Defaults to the goal. A comprehensive capability prompt (how to use the runtime's tools, connections, datasets & files) is added automatically."
              rows={3}
              value={prompt}
              onChange={(e) => {
                setPrompt(e.target.value);
                // The prompt now FEEDS propose, so an edit invalidates the plan it produced —
                // the goal handler has always done this; the other inputs never did.
                setProposal(null);
              }}
              aria-label="App prompt"
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

          {/* 5b: NL multi-step authoring — ask the served model to plan a workflow for the
              goal, preview it, then author the App as that multi-step blueprint. Falls back
              to a single agent step when there is no plan (e.g. no served model). */}
          {kind === "scheduled" ? (
            <div className="register-tool-form__row">
              <button
                type="button"
                className="btn-ghost"
                data-testid="new-app-propose"
                onClick={() =>
                  propose.mutate(
                    // The planner sees the whole brief, not just the goal: the name, the
                    // instruction the App runs, what files it can read, and — since the
                    // rails below now exist — which tools, skills and connectors it was
                    // given. Names only: the planner has no grant to dereference a content
                    // ref, and a credential value has no business in a prompt.
                    composeProposeGoal({
                      name,
                      goal,
                      prompt,
                      attachments: attach.attachments
                        .filter((a) => a.status === "ready")
                        .map((a) => a.filename),
                      tools: Object.keys(toolGrants),
                      skills: skills.map((s) => s.name),
                      connections: connections.map((c) => c.descriptor),
                    }),
                    {
                      onSuccess: (p) => {
                        setProposal(p);
                        // Seed the CANVAS, not a read-only list — the plan is a starting
                        // point the author can rearrange before saving.
                        if (p.proposed === true && p.steps.length > 0) {
                          setGraph(proposalGraph(p));
                          setSeedNonce((n) => n + 1);
                        }
                      },
                    },
                  )
                }
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
                  onClick={() => {
                    setProposal(null);
                    setGraph(null);
                    setSeedNonce((n) => n + 1);
                  }}
                  disabled={busy}
                >
                  Clear plan (author a single step)
                </button>
              ) : null}
            </div>
          ) : null}
          {proposal?.proposed === false ? (
            <output className="muted" data-testid="new-app-proposal-rejected">
              No multi-step plan: {proposal.reason} — authoring a single agent step over the goal.
            </output>
          ) : null}
          {proposeErr ? (
            <p className="field-error" data-testid="new-app-propose-error" role="alert">
              {proposeErr.message}
            </p>
          ) : null}

          {/* THE STRUCTURE SURFACE. A proposed plan used to render as a read-only <ol> —
              you could see the steps and change nothing. The canvas is the same plan,
              editable, and it is what gets lowered at save. Scheduled only: a hosted App
              authors no blueprint at all. */}
          {kind === "scheduled" ? (
            <fieldset className="new-app-form__rail" data-testid="new-app-structure">
              <legend className="muted">
                Structure{graph !== null ? ` (${graph.steps.length} steps)` : ""}
              </legend>
              <Suspense fallback={<p className="muted">Loading the builder…</p>}>
                <BlueprintBuilderSection
                  key={seedNonce}
                  mode={{ kind: "embedded" }}
                  palette={APP_PALETTE}
                  patterns={false}
                  initialGraph={graph ?? undefined}
                  onGraphChange={onGraphChange}
                />
              </Suspense>
            </fieldset>
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
          </div>

          {/* Container packaging — the Docker app lane ships next. Surfaced now as an
              honest-DISABLED radio so the affordance is discoverable without faking a
              control the runtime can't yet fulfil (GR15). The App handle is derived from
              the name (no handle field). */}
          <fieldset
            className="new-app-form__rail"
            aria-label="Packaging"
            data-testid="new-app-packaging"
          >
            <legend className="muted">Packaging</legend>
            <label>
              <input type="radio" name="packaging" checked readOnly disabled={busy} /> Standard
              runtime
            </label>{" "}
            <label className="muted" title="Container packaging — ships with the Docker app lane">
              <input
                type="radio"
                name="packaging"
                disabled
                data-testid="new-app-packaging-docker"
              />{" "}
              Docker container · soon
            </label>
          </fieldset>

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

          {/* THE CAPABILITY RAILS — tools / skills / integrations, authored HERE so the
              App is plugged in the moment it exists. Each mounts the same picker the App
              detail page's rails mount, so what you attach at create and what you attach
              afterwards are literally the same control (they cannot drift). Scheduled
              only: `hostsupervisor.rs` launches a hosted App from its framework +
              install/dev/build commands and never reads a tool, skill or connection, so
              offering these on the hosted lane would be a control that cannot fire. */}
          {kind === "scheduled" ? (
            <>
              <fieldset className="new-app-form__rail" data-testid="new-app-tools">
                <legend className="muted">MCP tools (a wish, granted at run)</legend>
                <ToolsPicker
                  grants={toolGrants}
                  reachInherit={reachInherit}
                  onChange={(g, r) => {
                    setToolGrants(g);
                    setReachInherit(r);
                  }}
                  disabled={busy}
                  disabledTitle="Creating…"
                  groupTestId="new-app-tools"
                  itemTestId="new-app-tool"
                  reachTestId="new-app-tools-reach"
                />
              </fieldset>
              <fieldset className="new-app-form__rail" data-testid="new-app-skills">
                <legend className="muted">Skills (instructions + tool wishes)</legend>
                <SkillsPicker
                  skills={skills}
                  onChange={setSkills}
                  disabled={busy}
                  disabledTitle="Creating…"
                  groupTestId="new-app-skills"
                  itemTestId="new-app-skill"
                />
              </fieldset>
              <fieldset className="new-app-form__rail" data-testid="new-app-connections">
                <legend className="muted">Integrations (connectors)</legend>
                <ConnectionsPicker
                  connections={connections}
                  onChange={setConnections}
                  disabled={busy}
                  disabledTitle="Creating…"
                  groupTestId="new-app-connections"
                  itemTestId="new-app-connection"
                />
              </fieldset>
            </>
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
