/**
 * Pure adapters that assemble reactflow `nodes`/`edges` from the projection +
 * the memoized layout (no React). Keeping this out of `MoteDag.tsx` lets the
 * node/edge construction be unit-tested directly and keeps the component thin.
 */

import type { Edge, Node } from "@xyflow/react";
import type { MoteVM } from "../../kx/use-projection";
import { buildEdges } from "./dag-graph";
import { toRfEdge } from "./edges";
import type { XY } from "./layout";

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
