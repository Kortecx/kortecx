import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useNodesInitialized,
  useReactFlow,
  useStore,
} from "@xyflow/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useResultMap } from "../../kx/use-content-batch";
import type { ProjectionVM } from "../../kx/use-projection";
import { useRunStepKinds } from "../../kx/use-run-step-kinds";
import { EmptyState } from "../EmptyState";
import { MoteTable } from "../MoteTable";
import { MoteNode } from "./MoteNode";
import { NodeDetailDrawer } from "./NodeDetailDrawer";
import { SwarmOverview } from "./SwarmOverview";
import { buildEdges, topologyHash } from "./dag-graph";
import { agenticTurnLabels, derivedChainEdges } from "./derived-lineage";
import { buildFlowEdges, buildFlowNodes, miniMapColor } from "./flow";
import type { MoteFlowNode } from "./flow";
import { type NodeBox, layoutForContainer } from "./layout";
import { branchEdgeIds, detectSwarm } from "./swarm-shape";

/** Stable empty roster — an inline `[]` would re-run every memo keyed on it. */
const NO_ROSTER: readonly string[] = [];

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

/** Stable identity for "nothing measured yet" — falls through to layout.ts's declared
 *  NODE_W/NODE_H. A fresh `{}` per render would re-run every memo keyed on the box. */
const DECLARED_BOX: NodeBox = {};

function DagFlow({ projection }: { projection: ProjectionVM }) {
  const motes = projection.motes;
  // The agent's turn order, which the runtime records OFF-DAG (turn Motes are edge-free
  // by design). These edges are synthesised, flagged `derived`, and drawn differently.
  const roster = projection.agenticTurnIds ?? NO_ROSTER;
  const derived = useMemo(() => derivedChainEdges(roster, motes), [roster, motes]);
  // The turn facts each node names itself with, off the rows the roster came from —
  // no extra request, and the same words the Timeline prints for that turn.
  const chainRows = projection.agenticTurnRows;
  const turnLabels = useMemo(
    () => (chainRows ? agenticTurnLabels(chainRows) : undefined),
    [chainRows],
  );
  const topoHash = useMemo(() => topologyHash(motes, derived), [motes, derived]);
  // The clicked Mote (drawer). Selection is for the DETAIL overlay only — reactflow's
  // own `elementsSelectable` stays OFF, so this never perturbs nodes/edges/layout.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = selectedId ? (motes.find((mm) => mm.moteId === selectedId) ?? null) : null;

  // The box dagre reserves per node, READ BACK from what the browser laid out rather
  // than declared as a constant. `.dag-node` is a flex column — an added row, a longer
  // result preview or a larger root font all change its height, and a constant cannot
  // track any of them. reactflow measures every node and republishes the size on
  // `measured`, so the sizes below are the rendered card's, by construction.
  //
  // Two passes, and that is inherent: nothing can be measured before it is rendered.
  // The first pass lays out against the CSS defaults, the measurement arrives, and the
  // second pass corrects it. It converges because a node's measured size depends on its
  // CONTENT and not on where it was placed — moving a card cannot resize it.
  //
  // ⚠ SUBSCRIBE TO A STRING, NEVER TO THE NODE ARRAY. `useNodes()` re-renders this
  // component on EVERY store change, and reactflow's store is a round trip for our own
  // `nodes` prop — so any value feeding that prop which is not reference-stable closes a
  // cycle: render → new `nodes` → setNodes → store change → render. `useRunStepKinds`
  // returns a fresh Map per render and is exactly such a value, so subscribing to the
  // array tore the tree down with React #185 ("maximum update depth exceeded"), which
  // surfaces as the run view's error boundary and says nothing about layout.
  //
  // Selecting a STRING fixes it by construction: reactflow compares the selected value,
  // so this component re-renders only when a measured SIZE actually changes — which is
  // also precisely when the layout may move. Positions changing cannot resize a card,
  // so the corrective second pass has nothing left with which to trigger a third.
  const rawSizeKey = useStore((s) => {
    let key = "";
    for (const [id, n] of s.nodeLookup) {
      key += `${id}:${n.measured?.width ?? 0}x${n.measured?.height ?? 0};`;
    }
    return key;
  });
  // ⚠ LATCH the last size a node was actually measured at. Handing reactflow a new
  // `nodes` array makes it re-adopt every node, and re-adoption drops `measured` until
  // the next measuring pass — so the raw key above dips through zero after every single
  // relayout. Read raw, that dip reverts the box to the declared default, which moves
  // the nodes, which triggers another re-adoption: the graph oscillates and its nodes
  // stay `visibility: hidden` (measured-less) for good. Keeping the last known size
  // makes the dip invisible, so a relayout settles in one pass.
  const latch = useRef(new Map<string, string>());
  const sizeKey = useMemo(() => {
    const live = new Set<string>();
    for (const entry of rawSizeKey.split(";")) {
      if (!entry) {
        continue;
      }
      const [id, dims] = entry.split(":");
      if (!id) {
        continue;
      }
      live.add(id);
      const [w, h] = (dims ?? "").split("x").map(Number);
      if (w && h) {
        latch.current.set(id, `${w}x${h}`);
      }
    }
    for (const id of [...latch.current.keys()]) {
      if (!live.has(id)) {
        latch.current.delete(id); // the Mote left the graph — forget its size
      }
    }
    return [...latch.current]
      .map(([id, dims]) => `${id}:${dims}`)
      .sort()
      .join(";");
  }, [rawSizeKey]);
  // The canvas's own measured size, as a primitive for the same reason as `sizeKey`.
  // reactflow tracks its container and updates this on every resize — a window resize,
  // a drawer opening, a laptop docked to an external display — so the layout below is
  // recomputed against the space actually available rather than a fixed assumption.
  const viewport = useStore((s) => `${Math.round(s.width)}x${Math.round(s.height)}`);
  const box = useMemo<NodeBox>(() => {
    const nodeHeights = new Map<string, number>();
    let widest = 0;
    for (const entry of sizeKey.split(";")) {
      if (!entry) {
        continue;
      }
      const [id, dims] = entry.split(":");
      const [w, h] = (dims ?? "").split("x").map(Number);
      if (!id || !w || !h) {
        continue; // never latched a real size for this node yet
      }
      nodeHeights.set(id, h);
      widest = Math.max(widest, w);
    }
    if (nodeHeights.size === 0) {
      return DECLARED_BOX;
    }
    return { nodeW: widest, nodeH: Math.max(...nodeHeights.values()), nodeHeights };
  }, [sizeKey]);

  // Relayout when the topology hash changes, or when a MEASURED SIZE changes; a
  // state-only poll moves neither and reuses the cached positions (the no-thrash
  // invariant — see dag-graph.topologyHash).
  // The DERIVED edges go to dagre too: without them an agent run's turns are N
  // parentless roots and lay out as a wide row of disconnected pairs.
  // biome-ignore lint/correctness/useExhaustiveDependencies: relayout is intentionally keyed on the topology hash + the measured box + the container size — a state-only poll moves none of them.
  const laidOut = useMemo(() => {
    const [w, h] = viewport.split("x").map(Number);
    return layoutForContainer(
      motes.map((m) => m.moteId),
      [...buildEdges(motes), ...derived],
      box,
      { width: w ?? 0, height: h ?? 0 },
    );
  }, [topoHash, box, viewport]);
  const positions = laidOut.positions;
  // The swarm shape (gather + branch fan-in) is topology-derived — recompute only on
  // a topology change; the branch/gather STRUCTURE is stable across a state-only poll.
  // biome-ignore lint/correctness/useExhaustiveDependencies: structure depends on topology only (same justification as positions/edges).
  const swarmStructure = useMemo(() => detectSwarm(motes), [topoHash]);
  const gatherId = swarmStructure?.gatherId;
  // Edges are topology — recompute only on a topology change; branch fan-in edges are highlighted.
  // biome-ignore lint/correctness/useExhaustiveDependencies: edges depend on topology only (which now folds in the derived edges — same justification as positions).
  const edges = useMemo(
    () => buildFlowEdges(motes, branchEdgeIds(swarmStructure), derived),
    [topoHash],
  );
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
    () =>
      buildFlowNodes(
        motes,
        positions,
        { byRef, loading: isLoading },
        gatherId,
        stepKinds,
        turnLabels,
      ),
    [motes, positions, byRef, isLoading, gatherId, stepKinds, turnLabels],
  );

  // Refit the viewport when the topology grows (dynamic children appear) AND once
  // every node has been measured. `useNodesInitialized()` flips true only after
  // reactflow measures node sizes, so the fit never runs against unmeasured
  // (zero-size) nodes — that race is what produced the stretched/narrow first
  // paint in the chat (T-FIX1). Guarded so a headless/jsdom flow is a no-op.
  const { fitView } = useReactFlow();
  const nodesInitialized = useNodesInitialized();
  // biome-ignore lint/correctness/useExhaustiveDependencies: topoHash, box and viewport are intentional re-fit triggers (refit when the graph grows, when a measured card size moves the layout, or when the canvas is resized), not read in the body.
  useEffect(() => {
    if (!nodesInitialized) {
      return;
    }
    try {
      void fitView({ padding: 0.2, duration: 200 });
    } catch {
      /* no measured viewport (headless) — ignore */
    }
  }, [topoHash, box, viewport, nodesInitialized, fitView]);

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
      {derived.length > 0 ? (
        <p className="dag-legend muted" data-testid="dag-derived-legend">
          Dotted links are the agent's turn order, read from this run's turn record — the runtime
          records no parent between turns.
        </p>
      ) : null}
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
