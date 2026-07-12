import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useNodesInitialized,
  useReactFlow,
} from "@xyflow/react";
import { useEffect, useMemo, useState } from "react";
import { useResultMap } from "../../kx/use-content-batch";
import type { ProjectionVM } from "../../kx/use-projection";
import { useRunStepKinds } from "../../kx/use-run-step-kinds";
import { EmptyState } from "../EmptyState";
import { MoteTable } from "../MoteTable";
import { MoteNode } from "./MoteNode";
import { NodeDetailDrawer } from "./NodeDetailDrawer";
import { SwarmOverview } from "./SwarmOverview";
import { buildEdges, topologyHash } from "./dag-graph";
import { buildFlowEdges, buildFlowNodes, miniMapColor } from "./flow";
import type { MoteFlowNode } from "./flow";
import { layoutGraph } from "./layout";
import { branchEdgeIds, detectSwarm } from "./swarm-shape";

/**
 * Above this many Motes the DAG falls back to the table. All nodes within the cap
 * render (no viewport culling — that mis-culls un-measured nodes and is needless
 * below the cap); the cap itself bounds the dagre layout + reactflow DOM/SVG cost.
 * The 25k-Mote M2.1 ceiling is the table's domain (the scale surface); the DAG is
 * the human-scale legibility surface.
 */
export const MAX_DAG_NODES = 500;

// Module-level (stable reference) — an inline object re-registers node types every
// render, a known reactflow performance footgun.
const nodeTypes = { mote: MoteNode };

function DagFlow({ projection }: { projection: ProjectionVM }) {
  const motes = projection.motes;
  const topoHash = useMemo(() => topologyHash(motes), [motes]);
  // The clicked Mote (drawer). Selection is for the DETAIL overlay only — reactflow's
  // own `elementsSelectable` stays OFF, so this never perturbs nodes/edges/layout.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = selectedId ? (motes.find((mm) => mm.moteId === selectedId) ?? null) : null;

  // Relayout ONLY when the topology hash changes; a state-only poll reuses the
  // cached positions (the no-thrash invariant — see dag-graph.topologyHash).
  // biome-ignore lint/correctness/useExhaustiveDependencies: relayout is intentionally keyed on the topology hash only — a state-only poll must NOT relayout.
  const positions = useMemo(
    () =>
      layoutGraph(
        motes.map((m) => m.moteId),
        buildEdges(motes),
      ),
    [topoHash],
  );
  // The swarm shape (gather + branch fan-in) is topology-derived — recompute only on
  // a topology change; the branch/gather STRUCTURE is stable across a state-only poll.
  // biome-ignore lint/correctness/useExhaustiveDependencies: structure depends on topology only (same justification as positions/edges).
  const swarmStructure = useMemo(() => detectSwarm(motes), [topoHash]);
  const gatherId = swarmStructure?.gatherId;
  // Edges are topology — recompute only on a topology change; branch fan-in edges are highlighted.
  // biome-ignore lint/correctness/useExhaustiveDependencies: edges depend on topology only (same justification as positions).
  const edges = useMemo(() => buildFlowEdges(motes, branchEdgeIds(swarmStructure)), [topoHash]);
  // Batch-resolve every committed result (one RPC, shared with the table). `byRef`
  // is reference-stable across an unchanged poll (memoized in useResultMap), so it
  // doesn't re-create nodes — node DATA only re-merges when results actually land.
  const refs = useMemo(() => motes.flatMap((m) => (m.resultRef ? [m.resultRef] : [])), [motes]);
  const { byRef, isLoading } = useResultMap(projection.instanceId, refs);
  // PR-D: each committed Mote's high-level step type (model/MCP/connector/tool/action)
  // for the read-only review labels — shares the inspector's `moteDetail` cache.
  const stepKinds = useRunStepKinds(projection.instanceId, motes);
  // Node DATA (state/anomaly + resolved result + step type) re-merges each poll WITHOUT relayout.
  const nodes = useMemo(
    () => buildFlowNodes(motes, positions, { byRef, loading: isLoading }, gatherId, stepKinds),
    [motes, positions, byRef, isLoading, gatherId, stepKinds],
  );

  // Refit the viewport when the topology grows (dynamic children appear) AND once
  // every node has been measured. `useNodesInitialized()` flips true only after
  // reactflow measures node sizes, so the fit never runs against unmeasured
  // (zero-size) nodes — that race is what produced the stretched/narrow first
  // paint in the chat (T-FIX1). Guarded so a headless/jsdom flow is a no-op.
  const { fitView } = useReactFlow();
  const nodesInitialized = useNodesInitialized();
  // biome-ignore lint/correctness/useExhaustiveDependencies: topoHash is an intentional re-fit trigger (refit when the graph grows), not read in the body.
  useEffect(() => {
    if (!nodesInitialized) {
      return;
    }
    try {
      void fitView({ padding: 0.2, duration: 200 });
    } catch {
      /* no measured viewport (headless) — ignore */
    }
  }, [topoHash, nodesInitialized, fitView]);

  return (
    <>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        onNodeClick={(_e, node: MoteFlowNode) => setSelectedId(node.id)}
        proOptions={{ hideAttribution: true }}
        minZoom={0.1}
        maxZoom={1.5}
      >
        <Background gap={20} />
        <Controls showInteractive={false} />
        <MiniMap
          pannable
          zoomable
          nodeColor={(n) => miniMapColor((n.data as MoteFlowNode["data"]).mote.stateCode)}
          nodeStrokeWidth={2}
          className="dag-minimap"
        />
      </ReactFlow>
      {selected ? (
        <NodeDetailDrawer
          // Keyed by the Mote so switching nodes REMOUNTS the drawer — the
          // pane selection resets to Result instead of leaking across motes.
          key={selected.moteId}
          mote={selected}
          motes={motes}
          instanceId={projection.instanceId}
          onClose={() => setSelectedId(null)}
        />
      ) : null}
    </>
  );
}

/**
 * The run's Motes as a live execution DAG (nodes = Motes colored by state/nd_class,
 * edges = `parents[]`). Consumes the same `ProjectionVM` as the table, so the poll
 * seam, `?atSeq` time-travel, and Refresh are all view-agnostic. Replaces the table
 * as the default run view (T3.3); the table stays as a toggle + the >MAX fallback.
 */
export function MoteDag({ projection }: { projection: ProjectionVM }) {
  if (projection.motes.length === 0) {
    return (
      <EmptyState
        title="No Motes yet"
        detail="This run has no Motes at the current frontier — they appear as the run executes."
      />
    );
  }
  if (projection.motes.length > MAX_DAG_NODES) {
    return (
      <div data-testid="dag-fallback">
        <SwarmOverview projection={projection} />
        <p className="muted">
          Graph hidden for {projection.motes.length} Motes — showing the table (the DAG renders for
          runs up to {MAX_DAG_NODES} Motes).
        </p>
        <MoteTable projection={projection} />
      </div>
    );
  }
  return (
    <>
      <SwarmOverview projection={projection} />
      <div
        className="dag-canvas"
        data-testid="mote-dag"
        role="img"
        aria-label={`Execution DAG of ${projection.motes.length} Motes`}
      >
        <ReactFlowProvider>
          <DagFlow projection={projection} />
        </ReactFlowProvider>
      </div>
    </>
  );
}
