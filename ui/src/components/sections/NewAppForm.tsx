/**
 * The Apps chat surface — ONE prompt box, then review, then approve.
 *
 * This replaced a stacked form (name, goal, prompt, model, four capability rails, a guidance
 * rule, an optional "Propose steps" button buried mid-page) whose "Create & scaffold" SAVED the
 * App and only then scaffolded it. The author's first look at what the runtime had decided came
 * AFTER an App already existed in the catalog.
 *
 * The flow is now three states:
 *
 *  1. **compose** — one prompt box. The KIND selector (Hosted | Scheduled) sits at its top
 *     right; choosing Scheduled opens a second selector to its right (Contextual | Codified,
 *     defaulting to CODIFIED). One prompt is the whole input.
 *  2. **review** — `DeriveApp` returns a design and NOTHING has been persisted. A scheduled
 *     design lands on the builder canvas as an editable graph; a hosted design lands as its
 *     planned file list.
 *  3. **approve** — only now does `SaveApp` + `ScaffoldApp` run, and the browser routes to the
 *     App's own page, where the scaffold streams in.
 *
 * **The graph is the whole create surface.** Tools, skills, integrations and grounding all
 * attach to the NODE that uses them, in the step drawer — there are no capability rails
 * beside the canvas. That is not a layout preference: a rail is app-level, and an app-level
 * capability binds to the entry step, which on a fan-out is not the step that needed it. A
 * node is the unit that says what it does, what it may reach, and what it knows.
 *
 * The declarations the envelope needs (`references.skills` / `.connections` / `.datasets`)
 * are computed at approve as the UNION of what the nodes name, so the author never maintains
 * two lists that can disagree.
 *
 * The envelope still carries NO authority: the server re-resolves every warrant at run, and a
 * derived tool grant is a WISH the runtime intersects again at fire (SN-8).
 */

import { type AppDerivation, app, defaultHandle, flow } from "@kortecx/sdk/web";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { type FormEvent, Suspense, lazy, useCallback, useMemo, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { queryKeys } from "../../kx/query-keys";
import { useApps } from "../../kx/use-apps";
import { useAttachments } from "../../kx/use-attachments";
import { useListMcpServers } from "../../kx/use-connections";
import { useDatasets } from "../../kx/use-datasets";
import { useDeriveApp } from "../../kx/use-derive-app";
import { useScaffoldApp } from "../../kx/use-scaffold-app";
import { useListSkills } from "../../kx/use-skills";
import { composeCapabilityPrompt } from "../../lib/app-capability-prompt";
import { collidingHandle } from "../../lib/app-handle";
import { pickedSkill } from "../apps/CapabilityPickers";
import { FRESH_UNMODELED, builderGraphToBlueprint } from "../builder/app-blueprint";
import {
  type BuilderGraph,
  type BuilderStep,
  proposalToBuilderGraph,
} from "../builder/builder-graph";
import { GlowCard } from "../ds/GlowCard";

/**
 * The visual builder canvas — the review surface for a scheduled design.
 *
 * MUST stay `lazy`. This component is statically imported by AppsSection, so a static import
 * would pull the builder + @xyflow + dagre (~248 KB) onto the Apps route for everyone who
 * merely opens the catalog.
 */
const BlueprintBuilderSection = lazy(() =>
  import("./BlueprintBuilderSection").then((m) => ({
    default: m.BlueprintBuilderSection,
  })),
);

/**
 * The step kinds an APP's canvas offers. Agent + Tool: an App is a governed automation, and the
 * pattern macros (swarm / supervisor / consensus) belong to the workflow builder, where a
 * one-shot run is the point.
 */
const APP_PALETTE = ["model", "tool"] as const;

/** Which lane the surface authors (D213): a scheduled functional app or a hosted web app. */
export type NewAppKind = "scheduled" | "hosted";

/** How a scheduled App is authored — the second axis, orthogonal to {@link NewAppKind}. */
export type NewAppMode = "contextual" | "codified";

/** The hosted-lane framework choices, in display order. `auto` takes the runtime's default. */
const HOSTED_FRAMEWORKS = [
  { value: "auto", label: "Auto" },
  { value: "vite_react", label: "React" },
  { value: "next_js", label: "Next.js" },
  { value: "svelte", label: "Svelte" },
] as const;
type HostedFrameworkChoice = (typeof HOSTED_FRAMEWORKS)[number]["value"];

/**
 * Narrow the framework the SERVER resolved back onto the template set this console knows.
 *
 * The server already vets its answer against its own template registry, so this normally just
 * re-types the same string. It falls back to the author's own pick rather than forwarding an
 * unrecognised name, because `hosted()` writes the template the supervisor will try to launch —
 * a name neither side has is a project that scaffolds and then cannot start.
 */
function resolvedFramework(
  fromDesign: string | undefined,
  picked: HostedFrameworkChoice,
): HostedFrameworkChoice {
  const known = HOSTED_FRAMEWORKS.find((f) => f.value === fromDesign);
  return known ? known.value : picked;
}

/** A successfully derived design (the `derived: true` arm), for brevity below. */
type Design = Extract<AppDerivation, { derived: true }>;

export function NewAppForm({
  onClose,
  initialKind = "scheduled",
  onKindAuthored,
}: {
  onClose: () => void;
  initialKind?: NewAppKind;
  /** Called with the kind the App was actually SAVED as, so the catalog can follow it. */
  onKindAuthored?: (kind: NewAppKind) => void;
}) {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scaffold = useScaffoldApp();
  const derive = useDeriveApp();
  const datasets = useDatasets();
  const skillCatalog = useListSkills();
  const serverRegistry = useListMcpServers();
  // The live catalog, for the handle-collision check. `AppsSection` renders this form and
  // already holds this query, so it resolves from cache — no extra round trip.
  const { apps } = useApps();
  const attach = useAttachments();

  // ---- compose state: the whole input ----
  const [kind, setKind] = useState<NewAppKind>(initialKind);
  // CODIFIED is the default for a scheduled App: what you get is a real project the runtime is
  // orchestrated from, which is what an automation should be. Contextual stays one click away.
  // This is a UI default only — the envelope still omits `mode` unless codified, so a
  // contextual App's bytes, and its `app_ref`, are exactly what they always were.
  const [mode, setMode] = useState<NewAppMode>("codified");
  const [prompt, setPrompt] = useState("");
  const [framework, setFramework] = useState<HostedFrameworkChoice>("auto");

  // ---- review state: seeded by the derive, then editable ----
  // Only the App's IDENTITY lives here. Every capability lives on a node — see the module
  // note; the declarations are derived from the graph at approve.
  const [design, setDesign] = useState<Design | null>(null);
  const [name, setName] = useState("");
  const [files, setFiles] = useState<{ path: string; role: string }[]>([]);
  // The LIVE canvas graph — what the author sees is what gets lowered at approve.
  const [graph, setGraph] = useState<BuilderGraph | null>(null);
  const onGraphChange = useCallback((g: BuilderGraph) => setGraph(g), []);
  // The builder seeds its node state ONCE (`useNodesState` is `useState` underneath), so a
  // changed `initialGraph` prop alone would be silently ignored. Keying the mount is the honest
  // way to say "this is a different starting graph".
  const [seedNonce, setSeedNonce] = useState(0);

  const busy = derive.isPending || scaffold.isPending;
  const collision = collidingHandle(apps, name);
  const groundable = (datasets.data ?? []).filter((d) => d.docCount > 0);
  const reviewing = design !== null;

  /**
   * Seed the review from the design: the App's identity, and the graph whose NODES carry
   * every capability the design chose.
   *
   * The design names an integration by its registered NAME (short and readable, which is what
   * a byte-bounded prompt menu can afford); the envelope binds by ENDPOINT, because the
   * endpoint is what the runtime actually dials. Mapping name → endpoint HERE is what makes a
   * derived integration a real binding rather than a label. A name the registry no longer
   * holds is dropped rather than allowed to block the review — the server already intersected
   * it against this caller's ceiling, so a miss here is a catalog that moved underneath us.
   */
  function seedReview(d: Design): void {
    setDesign(d);
    setName(d.name);
    setFiles(d.files.map((f) => ({ path: f.path, role: f.role })));
    if (d.steps.length > 0) {
      const bound = d.steps.map((s) => ({
        ...s,
        integrations: s.integrations
          .map((n) => serverRegistry.servers.find((sv) => sv.serverName === n)?.endpoint)
          .filter((e): e is string => e !== undefined),
        datasets: s.datasets.filter((n) => groundable.some((g) => g.name === n)),
      }));
      const insert = proposalToBuilderGraph(bound, d.edges, 0);
      setGraph({ steps: insert.steps, edges: insert.edges });
    } else {
      setGraph(null);
    }
    setSeedNonce((n) => n + 1);
  }

  function onDerive(e: FormEvent): void {
    e.preventDefault();
    if (prompt.trim() === "" || busy) {
      return;
    }
    derive.mutate(
      {
        kind,
        mode: kind === "scheduled" ? mode : undefined,
        prompt,
        framework: kind === "hosted" ? framework : undefined,
        attachments: attach.attachments.filter((a) => a.status === "ready").map((a) => a.filename),
      },
      {
        onSuccess: (d) => {
          if (d.derived) {
            seedReview(d);
          }
        },
      },
    );
  }

  /** Back to the prompt box, discarding the design. Nothing was persisted, so this costs
   *  nothing but the derive — which is exactly the point of reviewing before creating. */
  function startOver(): void {
    setDesign(null);
    setGraph(null);
    setFiles([]);
    derive.reset();
  }

  const create = useMutation<string, unknown, void>({
    mutationFn: async (): Promise<string> => {
      if (!client) {
        throw new Error("not connected");
      }
      const readyFiles = attach.attachments.filter((a) => a.status === "ready" && a.ref);
      const description = design?.description.trim() ?? "";

      // HOSTED: no blueprint — the runtime scaffolds a real web project into the App's branch
      // and serves it on a local port. The branch handle IS the app handle (one-App-one-branch).
      if (kind === "hosted") {
        const h = defaultHandle(name.trim());
        let hb = app(name.trim())
          .describe(description)
          .hosted(resolvedFramework(design?.framework, framework), h)
          .rule("capabilities", {
            body: composeCapabilityPrompt(
              description || prompt.trim(),
              readyFiles.map((a) => a.filename),
              "hosted",
            ),
          });
        for (const a of readyFiles) {
          hb = hb.context(a.filename, a.ref as string, { mediaType: a.mediaType });
        }
        const hosted = await hb.save({ client, handle: h });
        return hosted.handle;
      }

      // SCHEDULED: the reviewed canvas IS the blueprint. It falls back to a single agent step
      // over the prompt only when the canvas ended up empty (a design the author cleared out).
      const lowered =
        graph !== null && graph.steps.length > 0
          ? { toBlueprint: () => builderGraphToBlueprint(graph, FRESH_UNMODELED) }
          : null;
      let builder = app(name.trim())
        .describe(description)
        .blueprint(lowered ?? flow().agent(prompt.trim()))
        .rule("capabilities", {
          body: composeCapabilityPrompt(
            description || prompt.trim(),
            readyFiles.map((a) => a.filename),
            "scheduled",
          ),
        });
      for (const a of readyFiles) {
        builder = builder.context(a.filename, a.ref as string, { mediaType: a.mediaType });
      }
      // THE DECLARATIONS, DERIVED FROM THE NODES. `references.*` says what this App needs
      // registered; the blueprint's per-step lists say which node uses each one. Computing
      // the declaration set from the graph is what keeps them from disagreeing — there is no
      // second list for the author to maintain, and nothing can be declared that no step
      // asked for. The bindings themselves already rode into the blueprint above.
      //
      // Tools are NOT written app-level at all: a step's `tool_contract` is a real grant on
      // that step, so `steering_config.tools.requested_grants` would only duplicate it onto
      // the entry step. Every one is still a wish — the server resolves
      // `wish ∩ grants ∩ fireable` at run (SN-8).
      const named = (pick: (s: BuilderStep) => readonly string[]): string[] => [
        ...new Set((graph?.steps ?? []).flatMap((s) => pick(s))),
      ];
      for (const n of named((s) => s.skills)) {
        const s = skillCatalog.skills.find((c) => c.name === n);
        if (s) {
          builder = builder.skill(pickedSkill(s));
        }
      }
      for (const endpoint of named((s) => s.connections)) {
        builder = builder.withConnection(endpoint, "");
      }
      for (const ds of named((s) => s.datasets)) {
        builder = builder.dataset(ds);
      }
      if (mode === "codified") {
        builder = builder.mode("codified");
      }
      const result = await builder.save({ client });
      return result.handle;
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    },
  });

  const canCreate =
    reviewing && name.trim() !== "" && collision === null && !busy && !create.isPending;

  function onApprove(): void {
    if (!canCreate) {
      return;
    }
    const authoredKind = kind;
    create.mutate(undefined, {
      onSuccess: (appHandle) => {
        onKindAuthored?.(authoredKind);
        // Route to the App's OWN page and let the scaffold stream in there. The scaffold runs
        // server-side and outlives this panel, so watching it inside a form that is about to
        // close was always the wrong place for it. `onSettled`, not `onSuccess`: a scaffold that
        // could not START still leaves a real, saved App, and stranding the author on a closed
        // form with no way to reach it would be the worse failure.
        scaffold.mutate(
          { handle: appHandle, goal: prompt.trim() },
          {
            onSettled: () => {
              onClose();
              void navigate({ to: "/apps/$handle", params: { handle: appHandle } });
            },
          },
        );
      },
    });
  }

  const deriveErr = derive.error ? toUiError(derive.error) : null;
  const createErr = create.error ? toUiError(create.error) : null;
  const scaffoldErr = scaffold.error ? toUiError(scaffold.error) : null;
  const refusal = derive.data && !derive.data.derived ? derive.data.reason : null;

  const lede = useMemo(() => {
    if (reviewing) {
      return kind === "hosted"
        ? "Review the project before it exists. Nothing has been created yet — edit the file plan and the app's details, then create it."
        : "Review the workflow before it exists. Nothing has been created yet — open any step to change what it does and what it may use: its tools, skills, integrations and grounding all live on the step. Then create the app.";
    }
    if (kind === "hosted") {
      return "Describe the web app you want. The runtime designs the project and shows you the file plan before anything is created.";
    }
    return mode === "codified"
      ? "Describe what the app should do. The runtime designs the workflow — deciding what runs in parallel — and writes the configuration and code it is orchestrated from. You review all of it before the app exists."
      : "Describe what the app should do. The runtime designs the workflow — deciding what runs in parallel — and steers it with its own prompt, rules and reference notes. You review all of it before the app exists.";
  }, [reviewing, kind, mode]);

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
        {lede}
      </p>

      {!reviewing ? (
        <form onSubmit={onDerive} className="register-tool-form" data-testid="new-app-compose">
          {/* THE PROMPT BOX. The selectors ride at its top right: kind first, and — only for a
              scheduled app — the authoring mode immediately to its right. */}
          <div className="new-app-prompt">
            <div className="new-app-prompt__selectors">
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
              {kind === "scheduled" ? (
                <fieldset
                  className="view-toggle view-toggle--compact"
                  aria-label="App mode"
                  data-testid="new-app-mode"
                >
                  <button
                    type="button"
                    data-testid="new-app-mode-codified"
                    aria-pressed={mode === "codified"}
                    onClick={() => setMode("codified")}
                    disabled={busy}
                    title="The model writes the workflow, tool list and supporting config this app is orchestrated from"
                  >
                    Codified
                  </button>
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
                </fieldset>
              ) : null}
            </div>
            <textarea
              className="input new-app-prompt__input"
              data-testid="new-app-prompt"
              placeholder={
                kind === "hosted"
                  ? "Describe the web app — e.g. 'A pomodoro timer with a task list I can reorder'"
                  : "Describe the app — e.g. 'Every morning, read the overnight support email, triage it by urgency, and draft replies to the routine ones'"
              }
              rows={4}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              aria-label="Describe the app"
              disabled={busy}
            />
            <div className="new-app-prompt__foot">
              <label
                className="new-app-prompt__attach"
                title="Attach context files the app can read"
              >
                <span className="muted">Attach files</span>
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
              </label>
              <button
                type="submit"
                data-testid="new-app-derive"
                disabled={prompt.trim() === "" || busy || attach.uploading}
              >
                {derive.isPending ? "Designing…" : "Design the app"}
              </button>
            </div>
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
          </div>

          {kind === "hosted" ? (
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
          ) : null}

          {refusal !== null ? (
            <p className="field-error" data-testid="new-app-derive-rejected" role="alert">
              {refusal}
            </p>
          ) : null}
          {deriveErr ? (
            <p className="field-error" data-testid="new-app-derive-error" role="alert">
              {deriveErr.message}
            </p>
          ) : null}
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
        <div className="register-tool-form" data-testid="new-app-review">
          {/* WHAT THE DESIGN DID NOT GET. Shown first and always: a design that quietly asked
              for a tool it did not receive produces an App that quietly cannot do part of its
              job, and the author is the only one who can fix that — while it still costs
              nothing, because the App does not exist yet. */}
          {design.notices.length > 0 ? (
            <ul className="muted" data-testid="new-app-notices">
              {design.notices.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          ) : null}

          <input
            type="text"
            data-testid="new-app-name"
            placeholder="App name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="App name"
            maxLength={80}
            disabled={busy || create.isPending}
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
          {design.description !== "" ? (
            <p className="muted" data-testid="new-app-description">
              {design.description}
            </p>
          ) : null}

          {kind === "scheduled" ? (
            <fieldset className="new-app-form__rail" data-testid="new-app-structure">
              <legend className="muted">
                Workflow{graph !== null ? ` (${graph.steps.length} steps)` : ""}
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
          ) : (
            <fieldset className="new-app-form__rail" data-testid="new-app-files">
              <legend className="muted">Project files ({files.length})</legend>
              {files.length === 0 ? (
                <p className="muted" data-testid="new-app-files-empty">
                  No file plan — the scaffold will plan the project when you create the app.
                </p>
              ) : (
                <ul className="new-app-files">
                  {files.map((f) => (
                    <li key={f.path} data-testid={`new-app-file-${f.path}`}>
                      <code>{f.path}</code> <span className="muted">{f.role}</span>{" "}
                      <button
                        type="button"
                        className="linkbtn"
                        aria-label={`Remove ${f.path}`}
                        data-testid={`new-app-file-remove-${f.path}`}
                        onClick={() => setFiles((cur) => cur.filter((x) => x.path !== f.path))}
                        disabled={busy || create.isPending}
                      >
                        ✕
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </fieldset>
          )}

          <div className="register-tool-form__row">
            <button
              type="button"
              data-testid="new-app-approve"
              onClick={onApprove}
              disabled={!canCreate}
            >
              {create.isPending || scaffold.isPending ? "Creating…" : "Create app"}
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="new-app-start-over"
              onClick={startOver}
              disabled={busy || create.isPending}
            >
              Start over
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="new-app-cancel"
              onClick={onClose}
              disabled={busy || create.isPending}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {createErr ? (
        <p className="field-error" data-testid="new-app-save-error" role="alert">
          {createErr.message}
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
