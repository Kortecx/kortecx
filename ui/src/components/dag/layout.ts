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

/** Fixed node box used for layout (matches the `.dag-node` CSS footprint). */
export const NODE_W = 184;
export const NODE_H = 72;

/**
 * Lay the graph out top-to-bottom and return each node's TOP-LEFT position
 * (reactflow's coordinate origin; dagre reports centers). Tolerant of cycles
 * (dagre breaks back-edges via a greedy feedback-arc set) and of empty graphs.
 */
export function layoutGraph(
  nodeIds: readonly string[],
  edges: readonly GraphEdge[],
): Map<string, XY> {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "TB", nodesep: 44, ranksep: 64, marginx: 16, marginy: 16 });
  g.setDefaultEdgeLabel(() => ({}));

  for (const id of nodeIds) {
    g.setNode(id, { width: NODE_W, height: NODE_H });
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
      x: (n?.x ?? 0) - NODE_W / 2,
      y: (n?.y ?? 0) - NODE_H / 2,
    });
  }
  return positions;
}
