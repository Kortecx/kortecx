/**
 * Reader-SYNTHESISED chain lineage: the edges that draw an agent's turn order.
 *
 * The runtime records NO parent between one turn and the next. That is deliberate and
 * load-bearing — a turn Mote is registered edge-free because declaring parents would
 * move the canonical state digest, so the trajectory travels out-of-band instead. The
 * order is still durable; it lives in the ReactRound facts that `ListReactTurns` serves.
 *
 * So this module draws a link the projection does not contain, and every edge it emits
 * is flagged `derived` — the canvas renders those differently and says so in a legend.
 * A synthesised link must never be presentable as a recorded parent.
 *
 * Pure: no React, no reactflow, so every frontier can be unit-tested without rendering
 * (the same discipline as `dag-graph.ts`).
 */

import type { MoteVM } from "../../kx/use-projection";
import type { GraphEdge } from "./dag-graph";

/**
 * Consecutive edges along `roster`, restricted to the entries actually present at this
 * frontier.
 *
 * The roster is COMPRESSED before pairing, not filtered after: a turn whose Mote has not
 * landed yet is skipped and its neighbours are joined directly, so the chain stays
 * connected and no edge ever dangles. That matters twice over — `buildEdges` drops a
 * dangling durable edge for the same reason, and a dangling edge here would also hide
 * every node downstream of the gap from the layout.
 */
export function derivedChainEdges(
  roster: readonly string[],
  motes: readonly MoteVM[],
): GraphEdge[] {
  const present = new Set(motes.map((m) => m.moteId));
  const chain = roster.filter((id) => present.has(id));
  const edges: GraphEdge[] = [];
  for (let i = 1; i < chain.length; i += 1) {
    const source = chain[i - 1];
    const target = chain[i];
    if (source === undefined || target === undefined || source === target) {
      continue; // a repeated roster entry must never become a self-edge
    }
    edges.push({
      // `~>` not `->`: a derived id can never collide with a durable parent->child id,
      // so nothing keyed on edge ids (the swarm branch highlight) can mistake one.
      id: `${source}~>${target}`,
      source,
      target,
      // CONTROL is what it means — sequencing, not data flow. The turn's context is
      // assembled out-of-band, so there is genuinely no data dependency to draw.
      edgeKind: "control",
      nonCascade: false,
      derived: true,
    });
  }
  return edges;
}
