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
  /** Per-node height overrides (falls back to `nodeH`). A surface whose cards
   *  grow row-by-row with their content (the App Lineage diagram) passes each
   *  card's real height, so dagre ranks against the boxes it actually renders
   *  instead of reserving the worst case for every node. */
  readonly nodeHeights?: ReadonlyMap<string, number>;
  /** Rank direction. Omitted ⇒ `TB`, i.e. byte-identical to before for every
   *  existing caller. `LR` exists because the SHAPE of a run and the SHAPE of the
   *  viewport are independent: a long agent chain is many ranks deep and one node
   *  wide, and stacking that top-to-bottom in a short, wide canvas renders it tiny
   *  while most of the canvas stays empty. See {@link fitScale}. */
  readonly direction?: "TB" | "LR";
}

/** The laid-out graph's extent, in flow units, INCLUDING each node's own box. */
export function layoutExtent(
  positions: ReadonlyMap<string, XY>,
  box: NodeBox = {},
): { readonly width: number; readonly height: number } {
  const nodeW = box.nodeW ?? NODE_W;
  const nodeH = box.nodeH ?? NODE_H;
  let width = 0;
  let height = 0;
  for (const [id, p] of positions) {
    width = Math.max(width, p.x + nodeW);
    height = Math.max(height, p.y + (box.nodeHeights?.get(id) ?? nodeH));
  }
  return { width, height };
}

/**
 * How large the graph can be drawn inside a container — the scale at which it just
 * fits. Higher is better: it means the same cards render bigger, so the run is more
 * legible. Used to CHOOSE a rank direction from the container's real dimensions
 * rather than assuming one.
 */
export function fitScale(
  extent: { readonly width: number; readonly height: number },
  container: { readonly width: number; readonly height: number },
): number {
  if (extent.width <= 0 || extent.height <= 0) {
    return 0;
  }
  if (container.width <= 0 || container.height <= 0) {
    return 0;
  }
  return Math.min(container.width / extent.width, container.height / extent.height);
}

/**
 * Lay the graph out in whichever rank direction FITS THE CONTAINER BETTER, and say
 * which was chosen.
 *
 * Both directions are laid out and measured, because the better one depends on the
 * run's shape and the viewport's shape together and neither is known in advance: a
 * six-turn agent chain wants `LR` in a wide canvas and `TB` in a tall one, and a wide
 * fan-out wants the opposite. `TB` wins ties and near-ties — the margin below keeps a
 * marginal difference from flipping the whole picture on a few pixels of resize, which
 * would be far more disorienting than the space it would reclaim.
 */
export function layoutForContainer(
  nodeIds: readonly string[],
  edges: readonly GraphEdge[],
  box: NodeBox,
  container: { readonly width: number; readonly height: number },
): { positions: Map<string, XY>; direction: "TB" | "LR" } {
  const tb = layoutGraph(nodeIds, edges, { ...box, direction: "TB" });
  // No measured container (first paint, headless) — nothing to fit against, so keep
  // the established direction rather than guess from a zero.
  if (container.width <= 0 || container.height <= 0) {
    return { positions: tb, direction: "TB" };
  }
  const lr = layoutGraph(nodeIds, edges, { ...box, direction: "LR" });
  const tbScale = fitScale(layoutExtent(tb, box), container);
  const lrScale = fitScale(layoutExtent(lr, box), container);
  /** How much better `LR` must fit before the picture is allowed to turn. */
  const MARGIN = 1.25;
  return lrScale > tbScale * MARGIN
    ? { positions: lr, direction: "LR" }
    : { positions: tb, direction: "TB" };
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
  g.setGraph({
    rankdir: box.direction ?? "TB",
    nodesep: 44,
    ranksep: 64,
    marginx: 16,
    marginy: 16,
  });
  g.setDefaultEdgeLabel(() => ({}));

  const heightOf = (id: string): number => box.nodeHeights?.get(id) ?? nodeH;
  for (const id of nodeIds) {
    g.setNode(id, { width: nodeW, height: heightOf(id) });
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
      y: (n?.y ?? 0) - heightOf(id) / 2,
    });
  }
  return positions;
}
