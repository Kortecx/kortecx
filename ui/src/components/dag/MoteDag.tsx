import { Background, Controls, ReactFlow, ReactFlowProvider, useReactFlow } from "@xyflow/react";
import { useEffect, useMemo } from "react";
import type { ProjectionVM } from "../../kx/use-projection";
import { EmptyState } from "../EmptyState";
import { MoteTable } from "../MoteTable";
import { MoteNode } from "./MoteNode";
import { buildEdges, topologyHash } from "./dag-graph";
import { buildFlowEdges, buildFlowNodes } from "./flow";
import { layoutGraph } from "./layout";

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
  // Edges are topology — recompute only on a topology change.
  // biome-ignore lint/correctness/useExhaustiveDependencies: edges depend on topology only (same justification as positions).
  const edges = useMemo(() => buildFlowEdges(motes), [topoHash]);
  // Node DATA (state/anomaly) re-merges every poll WITHOUT relayout.
  const nodes = useMemo(() => buildFlowNodes(motes, positions), [motes, positions]);

  // Refit the viewport when the topology grows (dynamic children appear). Guarded
  // so a headless/jsdom flow (no measured viewport) is a harmless no-op.
  const { fitView } = useReactFlow();
  // biome-ignore lint/correctness/useExhaustiveDependencies: topoHash is an intentional re-fit trigger (refit when the graph grows), not read in the body.
  useEffect(() => {
    const t = setTimeout(() => {
      try {
        void fitView({ padding: 0.2, duration: 200 });
      } catch {
        /* no measured viewport (headless) — ignore */
      }
    }, 0);
    return () => clearTimeout(t);
  }, [topoHash, fitView]);

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      fitView
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
      proOptions={{ hideAttribution: true }}
      minZoom={0.1}
      maxZoom={1.5}
    >
      <Background gap={20} />
      <Controls showInteractive={false} />
    </ReactFlow>
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
        <p className="muted">
          Graph hidden for {projection.motes.length} Motes — showing the table (the DAG renders for
          runs up to {MAX_DAG_NODES} Motes).
        </p>
        <MoteTable projection={projection} />
      </div>
    );
  }
  return (
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
  );
}
