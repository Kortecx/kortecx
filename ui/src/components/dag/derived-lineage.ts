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

import type { AgenticTurnRow, MoteVM } from "../../kx/use-projection";
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

/** What a graph node says about the turn it draws — the same facts, from the same row,
 *  that the Timeline's card reads. */
export interface AgenticTurnLabel {
  readonly turn: number;
  readonly branch: string;
  readonly toolId: string;
  readonly toolVersion: string;
}

/**
 * Each chain node's turn facts, keyed by the Mote that draws it.
 *
 * FIRST-WINS over the newest-first wire — the same dedupe rule `agenticLineage` applies,
 * so a settled row supersedes the same turn's earlier pending one. No new request: these
 * are the rows the roster was already built from, carried through the projection.
 *
 * Lives HERE, in the lazily-loaded graph chunk, rather than beside the roster in
 * `use-projection`: that module is in the eager entry bundle, which is at a hard budget,
 * and only the graph needs these labels.
 */
export function agenticTurnLabels(
  rows: readonly AgenticTurnRow[],
): ReadonlyMap<string, AgenticTurnLabel> {
  const out = new Map<string, AgenticTurnLabel>();
  for (const r of rows) {
    if (!out.has(r.turnMoteId)) {
      out.set(r.turnMoteId, {
        turn: r.turn,
        branch: r.branch,
        toolId: r.toolId ?? "",
        toolVersion: r.toolVersion ?? "",
      });
    }
  }
  return out;
}
