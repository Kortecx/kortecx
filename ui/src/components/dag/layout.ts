/**
 * Pure dagre directed-graph layout (no React). Isolated so it can be tested in
 * isolation and memoized by the caller on a topology hash (state-only polls must
 * never relayout).
 */

import dagre from "@dagrejs/dagre";
import type { GraphEdge } from "./dag-graph";

export interface XY {
  readonly x: number;
  readonly y: number;
}

/** Default node box used for layout (matches the `.dag-node` CSS footprint). */
export const NODE_W = 184;
export const NODE_H = 72;

/** An override for the laid-out node box. A surface whose cards are not the
 *  `.dag-node` footprint (the App Lineage diagram's granular per-step cards) passes
 *  its own size so dagre's rank/node separation — and the center→top-left conversion
 *  below — are computed against the box it actually renders. Omitted ⇒ the
 *  {@link NODE_W}/{@link NODE_H} defaults, i.e. byte-identical to before. */
export interface NodeBox {
  readonly nodeW?: number;
  readonly nodeH?: number;
}

/**
 * Lay the graph out top-to-bottom and return each node's TOP-LEFT position
 * (reactflow's coordinate origin; dagre reports centers). Tolerant of cycles
 * (dagre breaks back-edges via a greedy feedback-arc set) and of empty graphs.
 */
export function layoutGraph(
  nodeIds: readonly string[],
  edges: readonly GraphEdge[],
  box: NodeBox = {},
): Map<string, XY> {
  const nodeW = box.nodeW ?? NODE_W;
  const nodeH = box.nodeH ?? NODE_H;
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "TB", nodesep: 44, ranksep: 64, marginx: 16, marginy: 16 });
  g.setDefaultEdgeLabel(() => ({}));

  for (const id of nodeIds) {
    g.setNode(id, { width: nodeW, height: nodeH });
  }
  for (const e of edges) {
    // Defensive: buildEdges already drops dangling, but never edge to a missing node.
    if (g.hasNode(e.source) && g.hasNode(e.target)) {
      g.setEdge(e.source, e.target);
    }
  }

  dagre.layout(g);

  const positions = new Map<string, XY>();
  for (const id of nodeIds) {
    const n = g.node(id);
    positions.set(id, {
      x: (n?.x ?? 0) - nodeW / 2,
      y: (n?.y ?? 0) - nodeH / 2,
    });
  }
  return positions;
}
