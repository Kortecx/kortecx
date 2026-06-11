/**
 * Pure adapters that assemble reactflow `nodes`/`edges` from the projection +
 * the memoized layout (no React). Keeping this out of `MoteDag.tsx` lets the
 * node/edge construction be unit-tested directly and keeps the component thin.
 */

import type { Edge, Node } from "@xyflow/react";
import type { MoteVM } from "../../kx/use-projection";
import { stateVisual } from "../../lib/colors";
import { buildEdges } from "./dag-graph";
import { toRfEdge } from "./edges";
import type { XY } from "./layout";

/**
 * Concrete hex per state tone for the MiniMap. The MiniMap paints SVG `fill`
 * attributes, which (unlike CSS) do NOT resolve `var(--t-*)` — so this is the one
 * place we mirror the `--t-*` light-theme values from `app.css`. Keep in sync.
 */
const TONE_UNKNOWN_HEX = "#4b5563";
const TONE_HEX: Readonly<Record<string, string>> = {
  pending: "#475569",
  scheduled: "#b45309",
  committed: "#047857",
  failed: "#dc2626",
  repudiated: "#c2410c",
  inconsistent: "#7c3aed",
  unknown: TONE_UNKNOWN_HEX,
};

/** MiniMap node fill for a Mote, keyed by its state tone (single source: `stateVisual`). */
export function miniMapColor(stateCode: number): string {
  return TONE_HEX[stateVisual(stateCode).tone] ?? TONE_UNKNOWN_HEX;
}

/** The data a `MoteNode` renders. The index signature satisfies reactflow's `Node<T>`. */
export interface MoteNodeData {
  readonly mote: MoteVM;
  readonly [key: string]: unknown;
}

export type MoteFlowNode = Node<MoteNodeData, "mote">;

/** Positioned reactflow nodes (positions come from the memoized dagre layout). */
export function buildFlowNodes(
  motes: readonly MoteVM[],
  positions: ReadonlyMap<string, XY>,
): MoteFlowNode[] {
  return motes.map((m) => ({
    id: m.moteId,
    type: "mote",
    position: positions.get(m.moteId) ?? { x: 0, y: 0 },
    data: { mote: m },
    draggable: false,
  }));
}

/** Styled reactflow edges from the Motes' parent links (dangling dropped). */
export function buildFlowEdges(motes: readonly MoteVM[]): Edge[] {
  return buildEdges(motes).map(toRfEdge);
}
