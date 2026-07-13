/**
 * POC-5d / redesign: the single-App LINEAGE pane — a READ-ONLY view of the App's
 * portable blueprint (a `DagSpec`) rendered as a static reactflow graph: the step order,
 * parallel-vs-sequential shape, and one node per step. Structure AUTHORING (add / remove
 * / reorder / config nodes + edges, save-to-App) is relocated to the Workflows builder;
 * this pane only VIEWS (and offers a Run). Reuses the BuilderNode leaf + the pure
 * `appBlueprintToBuilderGraph` parse + the dagre `layoutGraph`.
 */

import { useNavigate } from "@tanstack/react-router";
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import type { Edge } from "@xyflow/react";
import { useCallback, useMemo } from "react";
import { toUiError } from "../../kx/errors";
import { useApp, useRunApp } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { BuilderNode } from "../builder/BuilderNode";
import type { BuilderFlowNode } from "../builder/BuilderNode";
import { appBlueprintToBuilderGraph } from "../builder/app-blueprint";
import type { BuilderGraph } from "../builder/builder-graph";
import { layoutGraph } from "../dag/layout";

const nodeTypes = { builder: BuilderNode };

/** Lay a parsed lineage graph out via dagre (or an empty starter). */
function seedLineage(graph: BuilderGraph): { nodes: BuilderFlowNode[]; edges: Edge[] } {
  if (graph.steps.length === 0) {
    return { nodes: [], edges: [] };
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
  return { nodes, edges };
}

function LineageView({ handle, envelope }: { handle: string; envelope: Record<string, unknown> }) {
  const navigate = useNavigate();
  const runApp = useRunApp();

  const parsed = useMemo(
    () => appBlueprintToBuilderGraph((envelope.blueprint ?? { seed: 0, steps: [] }) as never),
    [envelope.blueprint],
  );
  const seeded = useMemo(() => seedLineage(parsed.graph), [parsed.graph]);
  const [nodes] = useNodesState<BuilderFlowNode>(seeded.nodes);
  const [edges] = useEdgesState<Edge>(seeded.edges);

  const onRun = useCallback(() => {
    runApp.mutate(
      { handle },
      {
        onSuccess: ({ instanceId }) =>
          void navigate({ to: "/workflows/$instanceId", params: { instanceId } }),
      },
    );
  }, [runApp, handle, navigate]);

  return (
    <div className="app-lineage" data-testid="app-lineage">
      <div className="app-lineage__toolbar">
        <span className="muted" data-testid="lineage-readonly-notice" role="note">
          A read-only view of this App's structure (order, parallel vs. sequential, one node per
          step). Compose or edit structure in the Workflows builder.
        </span>
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
            fitView
            proOptions={{ hideAttribution: true }}
            minZoom={0.1}
            maxZoom={1.5}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable={false}
            deleteKeyCode={null}
          >
            <Background gap={20} />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeStrokeWidth={2} className="dag-minimap" />
          </ReactFlow>
        </div>
      )}
    </div>
  );
}

/** The Lineage pane: fetch the App's full envelope, then render the read-only graph. */
export function AppLineageSection({ handle }: { handle: string }) {
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
      <LineageView handle={handle} envelope={app.data.envelope} />
    </ReactFlowProvider>
  );
}
