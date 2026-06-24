/**
 * POC-5d: the single-App LINEAGE editor — the App's portable blueprint (a `DagSpec`)
 * rendered as an EDITABLE reactflow graph, reusing the BlueprintBuilder leaf
 * components ({@link BuilderNode}, {@link StepConfigDrawer}) + the pure
 * {@link appBlueprintToBuilderGraph}/{@link builderGraphToBlueprint} round-trip and
 * the dagre {@link layoutGraph}. Editing the agentic structure (reorder / config /
 * add / remove nodes + edges) and saving persists a NEW App envelope version via
 * `SaveApp` — the App's blueprint is the ONLY field replaced (`{...envelope,
 * blueprint}`), every other rail (references / steering / replay / input_schema) is
 * carried verbatim.
 *
 * GR15 / D142 honesty: a LOCKED App or an un-round-trippable blueprint (`refuseEdit`
 * — an exec/binary step) renders READ-ONLY with a clear notice and NO Save control
 * (the server also refuses a locked structure save — LOCKED_BRANCH — so the gate is
 * authoritative, the UI gate is advisory). Edge instructions are a run-only fold (no
 * `DagSpec` representation) so the editor never offers that field.
 */

import { useNavigate } from "@tanstack/react-router";
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
import type { Connection, Edge, NodeMouseHandler } from "@xyflow/react";
import { useCallback, useMemo, useRef, useState } from "react";
import { toUiError } from "../../kx/errors";
import { useApp, useRunApp, useSaveApp } from "../../kx/use-apps";
import { useModels } from "../../kx/use-models";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { BuilderNode } from "../builder/BuilderNode";
import type { BuilderFlowNode } from "../builder/BuilderNode";
import { StepConfigDrawer } from "../builder/StepConfigDrawer";
import {
  type UnmodeledReport,
  appBlueprintToBuilderGraph,
  builderGraphToBlueprint,
} from "../builder/app-blueprint";
import {
  type BuilderGraph,
  type BuilderStep,
  type BuilderStepKind,
  newStep,
  validationError,
} from "../builder/builder-graph";
import { NODE_H, NODE_W, layoutGraph } from "../dag/layout";

const nodeTypes = { builder: BuilderNode };

/** Lay a parsed lineage graph out via dagre (or an empty starter). */
function seedLineage(graph: BuilderGraph): {
  nodes: BuilderFlowNode[];
  edges: Edge[];
  nextId: number;
} {
  if (graph.steps.length === 0) {
    return { nodes: [], edges: [], nextId: 0 };
  }
  const positions = layoutGraph(
    graph.steps.map((s) => s.id),
    graph.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      edgeKind: e.edge === "control" ? "control" : "data",
      nonCascade: false,
    })),
  );
  const nodes: BuilderFlowNode[] = graph.steps.map((step) => ({
    id: step.id,
    type: "builder",
    position: positions.get(step.id) ?? { x: 80, y: 40 },
    data: { step },
  }));
  const edges: Edge[] = graph.edges.map((e) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    data: { edge: e.edge },
  }));
  const used = graph.steps
    .map((s) => Number.parseInt(s.id.replace(/^s/, ""), 10))
    .filter((n) => Number.isFinite(n));
  return { nodes, edges, nextId: (used.length ? Math.max(...used) : -1) + 1 };
}

function LineageEditor({
  handle,
  envelope,
  locked,
}: {
  handle: string;
  envelope: Record<string, unknown>;
  locked: boolean;
}) {
  const navigate = useNavigate();
  const { models, unsupported } = useModels();
  const saveApp = useSaveApp();
  const runApp = useRunApp();

  const parsed = useMemo(
    () => appBlueprintToBuilderGraph((envelope.blueprint ?? { seed: 0, steps: [] }) as never),
    [envelope.blueprint],
  );
  const unmodeled: UnmodeledReport = parsed.unmodeled;
  const readOnly = locked || unmodeled.refuseEdit;

  const seeded = useMemo(() => seedLineage(parsed.graph), [parsed.graph]);
  const [nodes, setNodes, onNodesChange] = useNodesState<BuilderFlowNode>(seeded.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>(seeded.edges);
  const [selNode, setSelNode] = useState<string | null>(null);
  const idc = useRef(seeded.nextId);

  const addStep = useCallback(
    (kind: BuilderStepKind) => {
      const id = `s${idc.current++}`;
      setNodes((ns) => [
        ...ns,
        {
          id,
          type: "builder",
          position: { x: 80 + (ns.length % 2) * (NODE_W + 60), y: 40 + ns.length * (NODE_H + 40) },
          data: { step: newStep(kind, id) },
        },
      ]);
      setSelNode(id);
    },
    [setNodes],
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
      if (readOnly || !c.source || !c.target || c.source === c.target) {
        return;
      }
      const id = `e-${c.source}-${c.target}`;
      setEdges((es) =>
        es.some((e) => e.id === id) ? es : addEdge({ ...c, id, data: { edge: "data" } }, es),
      );
    },
    [setEdges, readOnly],
  );

  const graph: BuilderGraph = useMemo(
    () => ({
      steps: nodes.map((n) => n.data.step),
      edges: edges.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
        edge: ((e.data as { edge?: string } | undefined)?.edge ?? "data") as "data" | "control",
        instruction: "",
      })),
    }),
    [nodes, edges],
  );
  const invalid = validationError(graph);

  const onNodeClick = useCallback<NodeMouseHandler>(
    (_e, node) => {
      if (!readOnly) {
        setSelNode(node.id);
      }
    },
    [readOnly],
  );

  const onSave = useCallback(() => {
    if (readOnly || invalid) {
      return;
    }
    const blueprint = builderGraphToBlueprint(graph, unmodeled);
    saveApp.mutate({ handle, envelope: { ...envelope, blueprint } });
  }, [readOnly, invalid, graph, unmodeled, saveApp, handle, envelope]);

  const onRun = useCallback(() => {
    runApp.mutate(
      { handle },
      {
        onSuccess: ({ instanceId }) =>
          void navigate({ to: "/workflows/$instanceId", params: { instanceId } }),
      },
    );
  }, [runApp, handle, navigate]);

  const selectedStep = selNode ? (nodes.find((n) => n.id === selNode)?.data.step ?? null) : null;

  return (
    <div className="app-lineage" data-testid="app-lineage">
      <div className="app-lineage__toolbar">
        {readOnly ? (
          <span
            className="muted"
            data-testid={locked ? "lineage-locked-notice" : "lineage-readonly-notice"}
            role="note"
          >
            {locked
              ? "This App is locked — structure edits are refused. Unlock it in Policies to edit."
              : (unmodeled.reason ?? "This blueprint is read-only.")}
          </span>
        ) : (
          <>
            <button
              type="button"
              className="btn-ghost"
              data-testid="lineage-add-agent"
              onClick={() => addStep("model")}
            >
              + Agent
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="lineage-add-pure"
              onClick={() => addStep("pure")}
            >
              + Pure
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="lineage-add-tool"
              onClick={() => addStep("tool")}
            >
              + Tool
            </button>
            <button
              type="button"
              className="btn-primary"
              data-testid="app-lineage-save"
              disabled={invalid !== null || saveApp.isPending}
              title={invalid ?? "Save the edited structure as a new App version"}
              onClick={onSave}
            >
              {saveApp.isPending ? "Saving…" : "Save to App"}
            </button>
          </>
        )}
        <button
          type="button"
          className="btn-ghost"
          data-testid="app-lineage-run"
          disabled={runApp.isPending}
          onClick={onRun}
        >
          {runApp.isPending ? "Running…" : "Run"}
        </button>
      </div>

      {invalid && !readOnly ? (
        <output className="builder-validation" data-testid="lineage-validation">
          {invalid}
        </output>
      ) : null}
      {saveApp.isError ? (
        <ErrorNotice error={toUiError(saveApp.error)} onRetry={() => saveApp.reset()} />
      ) : null}
      {runApp.isError ? (
        <ErrorNotice error={toUiError(runApp.error)} onRetry={() => runApp.reset()} />
      ) : null}

      {nodes.length === 0 ? (
        <EmptyState title="No steps" detail="This App's blueprint has no steps to show." />
      ) : (
        <div
          className="dag-canvas app-lineage__canvas"
          data-testid="app-lineage-canvas"
          role="application"
          aria-label="App lineage canvas"
        >
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            onNodesChange={readOnly ? undefined : onNodesChange}
            onEdgesChange={readOnly ? undefined : onEdgesChange}
            onConnect={onConnect}
            onNodeClick={onNodeClick}
            fitView
            proOptions={{ hideAttribution: true }}
            minZoom={0.1}
            maxZoom={1.5}
            nodesDraggable={!readOnly}
            nodesConnectable={!readOnly}
            elementsSelectable={!readOnly}
            deleteKeyCode={readOnly ? null : ["Backspace", "Delete"]}
          >
            <Background gap={20} />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeStrokeWidth={2} className="dag-minimap" />
          </ReactFlow>
        </div>
      )}

      {selectedStep && !readOnly ? (
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
    </div>
  );
}

/** The Lineage pane: fetch the App's full envelope, then render the editable graph. */
export function AppLineageSection({ handle, locked }: { handle: string; locked: boolean }) {
  const app = useApp(handle);

  if (app.isLoading) {
    return <EmptyState title="Loading structure…" />;
  }
  if (app.isError) {
    return <ErrorNotice error={toUiError(app.error)} onRetry={() => void app.refetch()} />;
  }
  if (!app.data) {
    return (
      <EmptyState title="App not found" detail="This App is not in your catalog (or not owned)." />
    );
  }
  return (
    <ReactFlowProvider>
      <LineageEditor handle={handle} envelope={app.data.envelope} locked={locked} />
    </ReactFlowProvider>
  );
}
