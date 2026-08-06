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
  /**
   * True when the reader SYNTHESISED this edge rather than reading it off a Mote's
   * `parents[]`. An agent's turn order is durable but off-DAG, so the only honest way
   * to draw it is to mark it as derived — the surface must never present a synthesised
   * link as a recorded parent. See `derived-lineage.ts`.
   */
  readonly derived?: boolean;
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
 *
 * `extraEdges` folds in edges the reader synthesised (an agent's turn order, which the
 * runtime records off-DAG). They MUST be here: the layout memo is keyed on this hash
 * alone, so a synthesised edge outside it would never trigger the relayout that places
 * the turn it connects.
 */
export function topologyHash(
  motes: readonly MoteVM[],
  extraEdges: readonly GraphEdge[] = [],
): string {
  const ids = motes.map((m) => m.moteId).sort();
  const edges = [...buildEdges(motes), ...extraEdges]
    .map(
      (e) =>
        `${e.source}>${e.target}:${e.edgeKind}${e.nonCascade ? "!" : ""}${e.derived ? "~" : ""}`,
    )
    .sort();
  return `${ids.join(",")}|${edges.join(",")}`;
}

/**
 * The connected component of the projection containing `anchorMoteId` — one run's Motes,
 * pulled out of a journal that holds every run.
 *
 * WHY THIS IS NEEDED. `GetProjection` is not run-scoped by design: one `kx serve` is one
 * journal with ONE `instance_id` shared by every Invoke, chat turn, scaffold and cron
 * fire, and the fold reads the whole thing. So the run-detail view, opened for the App
 * you just ran, showed the entire workspace's Motes — and on a long-lived serve it
 * crossed MAX_DAG_NODES and silently degraded to a table for every run.
 *
 * The traversal is UNDIRECTED over `parents[]`: a run's Motes form one connected
 * component, and reaching only ancestors would drop everything downstream of the anchor
 * (the answer step, its artifacts) — which is most of what a user opened the page to see.
 *
 * Total and pure: an anchor that is absent from `motes` (a stale link, an old server
 * sending no salt) returns EMPTY, and callers must treat empty-with-an-anchor as "cannot
 * scope this" and say so, rather than silently falling back to the unscoped set.
 */
export function connectedComponent(
  motes: readonly MoteVM[],
  anchorMoteId: string,
): readonly MoteVM[] {
  return componentOfAny(motes, [anchorMoteId]);
}

/**
 * The union of the connected components containing ANY of `anchorMoteIds`, in the
 * projection's own order, without duplicates. Anchors absent from the fold are ignored;
 * if NONE is present the result is EMPTY, and a caller must treat that as "cannot scope
 * this" exactly as with a single anchor.
 *
 * WHY A UNION IS NEEDED. A react/agentic run is not one component. The coordinator
 * registers every turn Mote EDGE-FREE — declaring parents would move the canonical
 * digest — and only a tool observation carries a Data edge, back to its own turn. So the
 * run folds as N DISJOINT TWO-NODE STARS and a walk from one anchor reaches exactly one
 * of them. The chain's real order lives off-DAG in the durable ReactRound facts, so the
 * reader supplies each turn as its own anchor and unions what they reach.
 */
export function componentOfAny(
  motes: readonly MoteVM[],
  anchorMoteIds: readonly string[],
): readonly MoteVM[] {
  const byId = new Map(motes.map((m) => [m.moteId, m]));
  const anchors = anchorMoteIds.filter((id) => byId.has(id));
  if (anchors.length === 0) {
    return [];
  }
  // Adjacency in both directions, built once — `parents[]` only points upward.
  const neighbours = new Map<string, string[]>();
  const link = (a: string, b: string) => {
    const cur = neighbours.get(a);
    if (cur) {
      cur.push(b);
    } else {
      neighbours.set(a, [b]);
    }
  };
  for (const m of motes) {
    for (const p of m.parents) {
      if (!byId.has(p.parentId)) {
        continue; // dangling parent — not at this frontier yet
      }
      link(m.moteId, p.parentId);
      link(p.parentId, m.moteId);
    }
  }
  const seen = new Set(anchors);
  const stack = [...anchors];
  while (stack.length > 0) {
    const id = stack.pop();
    if (id === undefined) {
      break;
    }
    for (const n of neighbours.get(id) ?? []) {
      if (!seen.has(n)) {
        seen.add(n);
        stack.push(n);
      }
    }
  }
  // Preserve the projection's own order so downstream layout stays stable.
  return motes.filter((m) => seen.has(m.moteId));
}
