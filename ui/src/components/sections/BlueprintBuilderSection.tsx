/**
 * The visual Blueprint builder (D141.3/.4/.5/.6) — author a Tier-1 DAG (a vetted
 * PURE / MODEL palette) on an interactive reactflow canvas, configure each step in
 * a Monaco drawer, then SUBMIT it through `SubmitWorkflow`. The client sends ONLY
 * topology + params; the SERVER compiles the DAG + builds every warrant (SN-8).
 *
 * Rich-graph counterpart of the read-only run viewer: nodes are draggable +
 * connectable. An edge carries an optional instruction-file (D141.5) folded into
 * the downstream agent's prompt at submit. "Clone to edit" (D141.4) seeds the
 * builder from a committed run's DAG; the submit is always a NEW workflow (new
 * identity by construction). The acyclicity precheck mirrors the server's compile.
 */

import { defaultHandle } from "@kortecx/sdk/web";
import type { ProposedWorkflowEdge, ProposedWorkflowStep } from "@kortecx/sdk/web";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  addEdge,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import type { Connection, Edge, EdgeMouseHandler, NodeMouseHandler } from "@xyflow/react";
import { m } from "framer-motion";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useApp, useApps, useSaveApp } from "../../kx/use-apps";
import { useModels } from "../../kx/use-models";
import { useSubmitWorkflow } from "../../kx/use-submit-workflow";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { BuilderNode } from "../builder/BuilderNode";
import type { BuilderFlowNode } from "../builder/BuilderNode";
import { EdgeInstructionDrawer } from "../builder/EdgeInstructionDrawer";
import { NlProposePanel } from "../builder/NlProposePanel";
import { StepConfigDrawer } from "../builder/StepConfigDrawer";
import {
  type UnmodeledReport,
  appBlueprintToBuilderGraph,
  newAppEnvelope,
  structureSaveEnvelope,
} from "../builder/app-blueprint";
import {
  type BuilderEdge,
  type BuilderGraph,
  type BuilderStep,
  type BuilderStepKind,
  type PatternInsert,
  type PatternKind,
  insertPattern,
  newStep,
  proposalToBuilderGraph,
  toRequest,
  validationError,
} from "../builder/builder-graph";
import { NODE_H, NODE_W, layoutGraph } from "../dag/layout";

/**
 * How the builder is being used (the PUBLIC prop):
 *  - `workflow` (default) — author a one-shot DAG, "Build & run" it via `SubmitWorkflow`
 *    (unchanged); ALSO "Save as App" (a durable, reusable `kortecx.app/v1` envelope).
 *  - `app-edit` — edit a saved App's structure (POC-5d): pass only the `handle`; this
 *    (lazy) builder FETCHES + parses the envelope itself, so the eager route imports
 *    neither the App hooks nor the blueprint parser. "Save to App" replaces ONLY
 *    `envelope.blueprint` (the lossless rule).
 */
export type BuilderMode =
  | { kind: "workflow" }
  | { kind: "app-edit"; handle: string }
  /**
   * EMBEDDED — the canvas hosted inside another form (the New App flow), which owns the
   * terminal action.
   *
   * A distinct mode rather than a set of flags, because the property that matters is
   * negative: in this mode the builder renders NO Build&run / Save-as-App / Save-to-App
   * control, so none of its three `useNavigate` side-effects can fire and navigate the
   * user away from a half-filled form. Making that unreachable by construction beats
   * hiding the buttons and hoping.
   *
   * The host receives the live graph through `onGraphChange` and lowers it itself.
   */
  | { kind: "embedded" };

/** The mode `BuilderInner` consumes — for app-edit it additionally carries the parsed
 *  `unmodeled` snapshot (preserved blueprint-level fields) the Save re-merges. */
type InnerMode =
  | { kind: "workflow" }
  | { kind: "embedded" }
  | {
      kind: "app-edit";
      handle: string;
      envelope: Record<string, unknown>;
      unmodeled: UnmodeledReport;
    };

// Module-level stable reference (an inline object re-registers node types every
// render — a reactflow perf footgun).
const nodeTypes = { builder: BuilderNode };

/** Edge data the builder carries (the D141.5 instruction + the edge kind). */
interface BuilderEdgeData extends Record<string, unknown> {
  instruction: string;
  edge: "data" | "control";
}

/** Seed reactflow nodes/edges from an optional clone-to-edit graph (laid out via
 *  the shared dagre layout), else a single starter agent node. */
function seed(initial: BuilderGraph | undefined): {
  nodes: BuilderFlowNode[];
  edges: Edge[];
  nextId: number;
} {
  if (!initial || initial.steps.length === 0) {
    const step = newStep("model", "s0");
    return {
      nodes: [{ id: "s0", type: "builder", position: { x: 80, y: 40 }, data: { step } }],
      edges: [],
      nextId: 1,
    };
  }
  const positions = layoutGraph(
    initial.steps.map((s) => s.id),
    initial.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      edgeKind: e.edge === "control" ? "control" : "data",
      nonCascade: false,
    })),
  );
  const nodes: BuilderFlowNode[] = initial.steps.map((step) => ({
    id: step.id,
    type: "builder",
    position: positions.get(step.id) ?? { x: 80, y: 40 },
    data: { step },
  }));
  const edges: Edge[] = initial.edges.map((e) => toRfEdge(e));
  // The next free numeric id past any `s<n>` already used.
  const used = initial.steps
    .map((s) => Number.parseInt(s.id.replace(/^s/, ""), 10))
    .filter((n) => Number.isFinite(n));
  return { nodes, edges, nextId: (used.length ? Math.max(...used) : -1) + 1 };
}

function toRfEdge(e: BuilderEdge): Edge {
  const data: BuilderEdgeData = { instruction: e.instruction, edge: e.edge };
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    data,
    label: e.instruction.trim() ? "📎 instruction" : undefined,
    animated: false,
  };
}

function BuilderInner({
  initialGraph,
  mode,
  palette,
  patterns = true,
  onGraphChange,
}: {
  initialGraph?: BuilderGraph;
  mode: InnerMode;
  /** Which step kinds the toolbar offers. Omitted ⇒ all three (the standalone route).
   *  There was no palette concept before; an App canvas wants a narrower one. */
  palette?: readonly BuilderStepKind[];
  /** Show the multi-agent pattern macros. */
  patterns?: boolean;
  /** Publish the live graph to a host that owns the terminal (embedded mode). */
  onGraphChange?: (graph: BuilderGraph) => void;
}) {
  const navigate = useNavigate();
  const { models, unsupported } = useModels();
  const submit = useSubmitWorkflow();
  const saveApp = useSaveApp();
  const { apps } = useApps();
  const seeded = useMemo(() => seed(initialGraph), [initialGraph]);
  const [nodes, setNodes, onNodesChange] = useNodesState<BuilderFlowNode>(seeded.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>(seeded.edges);
  const [selNode, setSelNode] = useState<string | null>(null);
  const [selEdge, setSelEdge] = useState<string | null>(null);
  const [savingAs, setSavingAs] = useState(false);
  const [saveAsError, setSaveAsError] = useState<string | null>(null);
  // NL authoring (D209.3): the "describe a workflow" propose-then-confirm panel.
  const [proposing, setProposing] = useState(false);
  const idc = useRef(seeded.nextId);

  const addStep = useCallback(
    (kind: BuilderStepKind) => {
      const id = `s${idc.current++}`;
      setNodes((ns) => {
        const y = 40 + ns.length * (NODE_H + 40);
        return [
          ...ns,
          {
            id,
            type: "builder",
            position: { x: 80 + (ns.length % 2) * (NODE_W + 60), y },
            data: { step: newStep(kind, id) },
          },
        ];
      });
      setSelNode(id);
    },
    [setNodes],
  );

  // Scaffold a multi-agent orchestration pattern — insert a pre-wired
  // cluster of the EXISTING model/pure vocabulary (the same nodes/edges/drawer as
  // `addStep`), laid out below the current canvas via the shared dagre layout. The
  // cluster lowers to the SAME DAG the SDK/CLI author (`insertPattern` mirrors
  // `flow().swarm/supervisor/consensus()`). The author then fills in each node's
  // model (validation gates submit until every agent has one).
  // Apply a laid-out cluster (a pattern macro OR an NL-proposed plan) to the canvas:
  // lay it out, APPEND it below any existing nodes, and select its first node (opening the
  // config drawer). Shared by `insertPatternMacro` and `applyProposal` so both land nodes
  // identically.
  const applyInsert = useCallback(
    ({ steps, edges: patEdges, firstId, nextId }: PatternInsert) => {
      idc.current = nextId;
      const layout = layoutGraph(
        steps.map((s) => s.id),
        patEdges.map((e) => ({
          id: e.id,
          source: e.source,
          target: e.target,
          edgeKind: "data" as const,
          nonCascade: false,
        })),
      );
      const minY = Math.min(...steps.map((s) => layout.get(s.id)?.y ?? 0));
      setNodes((ns) => {
        const yBase = ns.length ? Math.max(...ns.map((n) => n.position.y)) + NODE_H + 80 : 40;
        const added: BuilderFlowNode[] = steps.map((step) => {
          const p = layout.get(step.id) ?? { x: 80, y: 0 };
          return {
            id: step.id,
            type: "builder",
            position: { x: p.x, y: p.y - minY + yBase },
            data: { step },
          };
        });
        return [...ns, ...added];
      });
      setEdges((es) => [...es, ...patEdges.map(toRfEdge)]);
      setSelNode(firstId);
    },
    [setNodes, setEdges],
  );

  const insertPatternMacro = useCallback(
    (kind: PatternKind) => applyInsert(insertPattern(kind, idc.current)),
    [applyInsert],
  );

  // NL authoring (D209.3): apply a proposed multi-step plan (from `proposeWorkflow`) to the
  // canvas as editable model nodes. The server re-COMPILES + warrants the confirmed DAG at
  // save/submit (SN-8); this only shapes what is proposed.
  const applyProposal = useCallback(
    (steps: readonly ProposedWorkflowStep[], edges: readonly ProposedWorkflowEdge[]) =>
      applyInsert(proposalToBuilderGraph(steps, edges, idc.current)),
    [applyInsert],
  );

  const updateStep = useCallback(
    (id: string, step: BuilderStep) => {
      setNodes((ns) => ns.map((n) => (n.id === id ? { ...n, data: { step } } : n)));
    },
    [setNodes],
  );

  const deleteStep = useCallback(
    (id: string) => {
      setNodes((ns) => ns.filter((n) => n.id !== id));
      setEdges((es) => es.filter((e) => e.source !== id && e.target !== id));
      setSelNode(null);
    },
    [setNodes, setEdges],
  );

  const onConnect = useCallback(
    (c: Connection) => {
      if (!c.source || !c.target || c.source === c.target) {
        return;
      }
      const id = `e-${c.source}-${c.target}`;
      setEdges((es) =>
        es.some((e) => e.id === id)
          ? es
          : addEdge(
              {
                ...c,
                id,
                data: { instruction: "", edge: "data" } satisfies BuilderEdgeData,
              },
              es,
            ),
      );
    },
    [setEdges],
  );

  const updateEdgeInstruction = useCallback(
    (id: string, instruction: string) => {
      setEdges((es) =>
        es.map((e) =>
          e.id === id
            ? {
                ...e,
                data: { ...(e.data as BuilderEdgeData), instruction },
                label: instruction.trim() ? "📎 instruction" : undefined,
              }
            : e,
        ),
      );
    },
    [setEdges],
  );

  const deleteEdge = useCallback(
    (id: string) => {
      setEdges((es) => es.filter((e) => e.id !== id));
      setSelEdge(null);
    },
    [setEdges],
  );

  // The logical graph derived from the canvas — the single source the submit maps.
  const graph: BuilderGraph = useMemo(
    () => ({
      steps: nodes.map((n) => n.data.step),
      edges: edges.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
        edge: ((e.data as BuilderEdgeData | undefined)?.edge ?? "data") as "data" | "control",
        instruction: (e.data as BuilderEdgeData | undefined)?.instruction ?? "",
      })),
    }),
    [nodes, edges],
  );
  // A one-shot workflow run requires an explicit model per agent; an App may leave a
  // served-model step blank (the run binds the served model — the `allowEmptyModel`
  // convention). The banner shows the reason for this mode's primary terminal.
  const invalidRun = validationError(graph);
  const invalidApp = validationError(graph, { allowEmptyModel: true });
  // An App may leave a model blank (the run binds the served one), so an embedded canvas
  // authoring an App validates by the APP rule, not the one-shot-run rule.
  const invalid = mode.kind === "workflow" ? invalidRun : invalidApp;

  // Publish the live graph to a host (embedded mode). Effect, not a render-time call:
  // the host sets state from it.
  useEffect(() => {
    onGraphChange?.(graph);
  }, [graph, onGraphChange]);

  const onNodeClick = useCallback<NodeMouseHandler>((_e, node) => {
    setSelEdge(null);
    setSelNode(node.id);
  }, []);
  const onEdgeClick = useCallback<EdgeMouseHandler>((_e, edge) => {
    setSelNode(null);
    setSelEdge(edge.id);
  }, []);

  const onSubmit = useCallback(() => {
    let req: ReturnType<typeof toRequest>;
    try {
      req = toRequest(graph, 0);
    } catch {
      return; // `invalid` already surfaces the reason in the bar
    }
    submit.mutate(req, {
      onSuccess: ({ instanceId, reactChainSalt }) => {
        // See AppRunDrawer: the salt is what scopes the run view to THIS submission.
        void navigate({
          to: "/workflows/$instanceId",
          params: { instanceId },
          search: reactChainSalt ? { chain: reactChainSalt } : {},
        });
      },
    });
  }, [graph, submit, navigate]);

  // "Save to App" (app-edit): replace ONLY `envelope.blueprint` (the lossless rule,
  // app-blueprint.ts) — everything else (references / steering_config / input_schema /
  // tags) rides verbatim. The new structure drives the next run.
  const onSaveToApp = useCallback(() => {
    if (mode.kind !== "app-edit") {
      return;
    }
    saveApp.mutate(
      {
        handle: mode.handle,
        envelope: structureSaveEnvelope(mode.envelope, graph, mode.unmodeled),
      },
      {
        onSuccess: () =>
          void navigate({
            to: "/apps/$handle",
            params: { handle: mode.handle },
            search: { tab: "lineage" },
          }),
      },
    );
  }, [mode, graph, saveApp, navigate]);

  // "Save as App" (workflow): mint a fresh minimal `kortecx.app/v1` envelope from the
  // current structure (mirrors the SDK `AppBuilder.toEnvelope` minimal shape). Refuse a
  // handle collision (never silently clobber an existing App).
  const onSaveAsApp = useCallback(
    (name: string) => {
      const handle = defaultHandle(name);
      if (apps.some((a) => a.handle === handle)) {
        setSaveAsError(`An App named "${name}" already exists — choose a different name.`);
        return;
      }
      saveApp.mutate(
        { handle, envelope: newAppEnvelope(name, graph) },
        {
          onSuccess: () => {
            setSavingAs(false);
            void navigate({ to: "/apps/$handle", params: { handle } });
          },
        },
      );
    },
    [apps, graph, saveApp, navigate],
  );

  const selectedStep = selNode ? (nodes.find((n) => n.id === selNode)?.data.step ?? null) : null;
  const selectedEdge = selEdge ? (graph.edges.find((e) => e.id === selEdge) ?? null) : null;

  const embedded = mode.kind === "embedded";
  const Chrome = embedded ? EmbeddedChrome : PageChrome;
  // No palette given ⇒ every kind (the standalone route's behaviour, unchanged).
  const offers = (k: BuilderStepKind): boolean => palette === undefined || palette.includes(k);
  return (
    <Chrome
      mode={mode}
      toolbar={
        <div className="builder-toolbar">
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-propose"
            title="Describe a goal; the served model proposes a multi-step plan you review + apply"
            onClick={() => setProposing(true)}
          >
            ✨ Describe a workflow
          </button>
          <span className="builder-toolbar__divider" aria-hidden="true" />
          {offers("model") ? (
            <button
              type="button"
              className="btn-ghost"
              data-testid="builder-add-agent"
              onClick={() => addStep("model")}
            >
              + Agent
            </button>
          ) : null}
          {offers("pure") ? (
            <button
              type="button"
              className="btn-ghost"
              data-testid="builder-add-pure"
              onClick={() => addStep("pure")}
            >
              + Pure step
            </button>
          ) : null}
          {offers("tool") ? (
            <button
              type="button"
              className="btn-ghost"
              data-testid="builder-add-tool"
              onClick={() => addStep("tool")}
            >
              + Tool
            </button>
          ) : null}
          {patterns ? (
            <>
              <span className="builder-toolbar__divider" aria-hidden="true" />
              <button
                type="button"
                className="btn-ghost"
                data-testid="builder-add-swarm"
                title="Fan out to parallel agents, then gather"
                onClick={() => insertPatternMacro("swarm")}
              >
                + Swarm
              </button>
              <button
                type="button"
                className="btn-ghost"
                data-testid="builder-add-supervisor"
                title="A planner decomposes the task; workers run in parallel; a lead integrates"
                onClick={() => insertPatternMacro("supervisor")}
              >
                + Supervisor
              </button>
              <button
                type="button"
                className="btn-ghost"
                data-testid="builder-add-consensus-judge"
                title="N voters, then a judge selects the single best answer"
                onClick={() => insertPatternMacro("consensusJudge")}
              >
                + Consensus · judge
              </button>
              <button
                type="button"
                className="btn-ghost"
                data-testid="builder-add-consensus-majority"
                title="N voters, then an exact-equality majority vote (server-reduced)"
                onClick={() => insertPatternMacro("consensusMajority")}
              >
                + Consensus · majority
              </button>
            </>
          ) : null}
          {mode.kind === "embedded" ? null : mode.kind === "app-edit" ? (
            <button
              type="button"
              className="builder-submit"
              data-testid="builder-save-app"
              disabled={invalidApp !== null || saveApp.isPending}
              title={invalidApp ?? "Save this structure to the App"}
              onClick={onSaveToApp}
            >
              {saveApp.isPending ? "Saving…" : "Save to App"}
            </button>
          ) : (
            <>
              <button
                type="button"
                className="btn-ghost"
                data-testid="builder-save-as-app"
                disabled={invalidApp !== null || saveApp.isPending}
                title={invalidApp ?? "Save this structure as a reusable App"}
                onClick={() => {
                  setSaveAsError(null);
                  setSavingAs(true);
                }}
              >
                Save as App
              </button>
              <button
                type="button"
                className="builder-submit"
                data-testid="builder-submit"
                disabled={invalidRun !== null || submit.isPending}
                title={invalidRun ?? "Compile + run on the server"}
                onClick={onSubmit}
              >
                {submit.isPending ? "Submitting…" : "Build & run"}
              </button>
            </>
          )}
        </div>
      }
    >
      {invalid ? (
        <output className="builder-validation" data-testid="builder-validation">
          {invalid}
        </output>
      ) : null}
      {submit.isError ? <ErrorNotice error={toUiError(submit.error)} /> : null}
      {saveApp.isError ? <ErrorNotice error={toUiError(saveApp.error)} /> : null}

      <div
        className="dag-canvas builder-canvas"
        data-testid="builder-canvas"
        role="application"
        aria-label="Blueprint builder canvas"
      >
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onNodeClick={onNodeClick}
          onEdgeClick={onEdgeClick}
          fitView
          proOptions={{ hideAttribution: true }}
          minZoom={0.1}
          maxZoom={1.5}
          deleteKeyCode={["Backspace", "Delete"]}
        >
          <Background gap={20} />
          <Controls showInteractive={false} />
          <MiniMap pannable zoomable nodeStrokeWidth={2} className="dag-minimap" />
        </ReactFlow>
      </div>

      {selectedStep ? (
        <StepConfigDrawer
          key={selectedStep.id}
          step={selectedStep}
          models={models}
          modelsUnsupported={unsupported}
          onChange={(s) => updateStep(selectedStep.id, s)}
          onDelete={() => deleteStep(selectedStep.id)}
          onClose={() => setSelNode(null)}
        />
      ) : null}
      {selectedEdge ? (
        <EdgeInstructionDrawer
          key={selectedEdge.id}
          edge={selectedEdge}
          steps={graph.steps}
          // In App modes edges are pure control/data flow: an edge instruction is a
          // run-only fold with no DagSpec representation (app-blueprint.ts), so it would
          // silently vanish on Save — hide the field rather than fake persistence.
          hideInstruction={mode.kind !== "workflow"}
          onChange={(text) => updateEdgeInstruction(selectedEdge.id, text)}
          onDelete={() => deleteEdge(selectedEdge.id)}
          onClose={() => setSelEdge(null)}
        />
      ) : null}

      {savingAs ? (
        <SaveAsAppDialog
          pending={saveApp.isPending}
          error={saveAsError}
          onSubmit={onSaveAsApp}
          onClose={() => {
            setSavingAs(false);
            setSaveAsError(null);
            saveApp.reset();
          }}
        />
      ) : null}

      {proposing ? (
        <NlProposePanel onApply={applyProposal} onClose={() => setProposing(false)} />
      ) : null}
    </Chrome>
  );
}

interface ChromeProps {
  mode: InnerMode;
  /** The builder toolbar — inside `.section-head` on a page, standalone when embedded. */
  toolbar: React.ReactNode;
  children: React.ReactNode;
}

/** The standalone route's page chrome. Byte-identical DOM to before the embed split:
 *  `.screen.builder` > `.section-head` > (heading + lede, toolbar), then the canvas. */
function PageChrome({ mode, toolbar, children }: ChromeProps) {
  const appEdit = mode.kind === "app-edit";
  return (
    <section className="screen builder" data-testid="blueprint-builder">
      <header className="section-head">
        <div>
          <h2>{appEdit ? "Edit App structure" : "New blueprint"}</h2>
          <p className="muted">
            {appEdit
              ? `Editing ${String(mode.envelope.name ?? mode.handle)} — arrange steps + edges, then Save to App. The saved structure drives the next run.`
              : "Drag to arrange, drag handle-to-handle to connect. The server compiles the DAG and builds every warrant — you author topology + params only."}
          </p>
        </div>
        {toolbar}
      </header>
      {children}
    </section>
  );
}

/**
 * Embedded chrome — no `.screen` wrapper, no heading, no lede. The host form has already
 * said what this is; a second page-level heading inside a card reads as a nested page.
 * Keeps `data-testid="blueprint-builder"` so the canvas is addressable identically on
 * both surfaces.
 */
function EmbeddedChrome({ toolbar, children }: ChromeProps) {
  return (
    <div className="builder builder--embedded" data-testid="blueprint-builder">
      {toolbar}
      {children}
    </div>
  );
}

/** Name + confirm a brand-new App minted from the current builder structure (a
 *  portaled `--overlay` dialog — the `DuplicateDialog` pattern, above the navbar). */
function SaveAsAppDialog({
  pending,
  error,
  onSubmit,
  onClose,
}: {
  pending: boolean;
  error: string | null;
  onSubmit: (name: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const trimmed = name.trim();
  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Cancel save as App"
        onClick={onClose}
      />
      <div className="dialog-center dialog-center--overlay">
        <m.div
          className="dialog-card"
          data-testid="builder-save-as-dialog"
          // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; modal semantics via role+aria-label (the DuplicateDialog precedent).
          role="dialog"
          aria-label="Save as new App"
          initial={{ y: 12, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
        >
          <h2 className="dialog-card__title">Save as new App</h2>
          <p className="muted">
            Save this structure as a durable, reusable App. The server compiles the blueprint and
            re-resolves every warrant at run (SN-8); grant tools + connections on the App page.
          </p>
          <label className="dialog-card__label" htmlFor="save-as-name">
            App name
          </label>
          <input
            ref={inputRef}
            id="save-as-name"
            className="input"
            data-testid="builder-save-as-name"
            value={name}
            placeholder="My App"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && trimmed !== "") {
                onSubmit(trimmed);
              }
            }}
          />
          {error ? (
            <span className="builder-field__error" data-testid="builder-save-as-error">
              {error}
            </span>
          ) : null}
          <div className="dialog-card__actions">
            <button type="button" className="btn-ghost" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="btn-primary"
              data-testid="builder-save-as-submit"
              disabled={pending || trimmed === ""}
              onClick={() => onSubmit(trimmed)}
            >
              {pending ? "Saving…" : "Save App"}
            </button>
          </div>
        </m.div>
      </div>
    </>,
    document.body,
  );
}

/** app-edit: fetch the App's envelope, parse its blueprint, then open the editor. An
 *  un-round-trippable (exec/binary) blueprint refuses the editor + links back to the
 *  read-only Lineage. All of this lives in the lazy builder chunk (never eager). */
function AppEditLoader({ handle }: { handle: string }) {
  const app = useApp(handle);
  if (app.isLoading) {
    return <EmptyState title="Loading App structure…" detail="Fetching the blueprint to edit." />;
  }
  if (app.isError) {
    return <ErrorNotice error={toUiError(app.error)} onRetry={() => void app.refetch()} />;
  }
  if (!app.data) {
    return (
      <EmptyState title="App not found" detail="This App is not in your catalog (or not owned)." />
    );
  }
  const envelope = app.data.envelope;
  const parsed = appBlueprintToBuilderGraph(
    (envelope.blueprint ?? { seed: 0, steps: [] }) as never,
  );
  if (parsed.unmodeled.refuseEdit) {
    return (
      <EmptyState
        title="Structure can't be edited here"
        detail={
          parsed.unmodeled.reason ??
          "This App's blueprint has an exec/binary step the visual editor can't safely edit."
        }
        action={
          <Link
            to="/apps/$handle"
            params={{ handle }}
            search={{ tab: "lineage" }}
            className="btn-ghost"
            data-testid="app-edit-refuse-back"
          >
            View in Lineage
          </Link>
        }
      />
    );
  }
  return (
    <ReactFlowProvider>
      <BuilderInner
        initialGraph={parsed.graph}
        mode={{ kind: "app-edit", handle, envelope, unmodeled: parsed.unmodeled }}
      />
    </ReactFlowProvider>
  );
}

/**
 * The builder section. `initialGraph` (optional) seeds clone-to-edit (D141.4) from a
 * committed run's reconstructed DAG; absent ⇒ a fresh single-agent blueprint. `mode`
 * selects the terminal: a one-shot workflow run (default) with a Save-as-App option, or
 * editing a saved App's structure (app-edit fetches + parses the App HERE, in this lazy
 * chunk, so the eager route imports neither the App hooks nor the parser).
 */
export function BlueprintBuilderSection({
  initialGraph,
  mode = { kind: "workflow" },
  palette,
  patterns,
  onGraphChange,
}: {
  initialGraph?: BuilderGraph;
  mode?: BuilderMode;
  /** Which step kinds the toolbar offers (default: all three). */
  palette?: readonly BuilderStepKind[];
  /** Show the multi-agent pattern macros (default: true). */
  patterns?: boolean;
  /** Publish the live graph to a host that owns the terminal (embedded mode). */
  onGraphChange?: (graph: BuilderGraph) => void;
}) {
  if (mode.kind === "app-edit") {
    return <AppEditLoader handle={mode.handle} />;
  }
  return (
    <ReactFlowProvider>
      <BuilderInner
        initialGraph={initialGraph}
        mode={mode}
        palette={palette}
        patterns={patterns}
        onGraphChange={onGraphChange}
      />
    </ReactFlowProvider>
  );
}
