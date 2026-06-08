/**
 * Pure projection → DAG-graph transform (no React, no reactflow). The single
 * source of the live graph's structure; kept isolated so every topology can be
 * exhaustively unit-tested without rendering. Mirrors the Rust core's
 * pure/total/testable discipline.
 */

import type { MoteVM, ParentEdgeVM } from "../../kx/use-projection";

/** A directed edge in the projection DAG (parent → child). */
export interface GraphEdge {
  readonly id: string;
  readonly source: string; // parent mote id (hex)
  readonly target: string; // child mote id (hex)
  readonly edgeKind: ParentEdgeVM["edgeKind"];
  readonly nonCascade: boolean;
}

/**
 * Build the DAG edges from each Mote's `parents[]`. An edge whose parent is NOT
 * present at the current frontier is DROPPED — a child can surface a poll before
 * its parent; never render a dangling edge, it materializes on the next poll.
 */
export function buildEdges(motes: readonly MoteVM[]): GraphEdge[] {
  const present = new Set(motes.map((m) => m.moteId));
  const edges: GraphEdge[] = [];
  for (const m of motes) {
    for (const p of m.parents) {
      if (!present.has(p.parentId)) {
        continue; // drop dangling — parent not at this frontier yet
      }
      edges.push({
        id: `${p.parentId}->${m.moteId}`,
        source: p.parentId,
        target: m.moteId,
        edgeKind: p.edgeKind,
        nonCascade: p.nonCascade,
      });
    }
  }
  return edges;
}

/**
 * A stable hash of the DAG *topology* — the sorted node-id set + sorted edge set.
 * Deliberately EXCLUDES mote state/anomaly so a poll that only flips a state
 * yields an identical hash (→ no dagre relayout; cached positions are reused).
 * This is the load-bearing no-thrash invariant.
 */
export function topologyHash(motes: readonly MoteVM[]): string {
  const ids = motes.map((m) => m.moteId).sort();
  const edges = buildEdges(motes)
    .map((e) => `${e.source}>${e.target}:${e.edgeKind}${e.nonCascade ? "!" : ""}`)
    .sort();
  return `${ids.join(",")}|${edges.join(",")}`;
}
