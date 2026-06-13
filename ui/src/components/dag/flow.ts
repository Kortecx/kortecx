/**
 * Pure adapters that assemble reactflow `nodes`/`edges` from the projection +
 * the memoized layout (no React). Keeping this out of `MoteDag.tsx` lets the
 * node/edge construction be unit-tested directly and keeps the component thin.
 */

import type { Edge, Node } from "@xyflow/react";
import type { BatchedContentVM } from "../../kx/use-content-batch";
import type { MoteVM } from "../../kx/use-projection";
import { stateVisual } from "../../lib/colors";
import type { DecodedContent } from "../../lib/content-decode";
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
  /** The resolved committed result (D142.2: text headline on the node). */
  readonly resultContent?: DecodedContent;
  /** The batch returned the uniform-empty item for this result ref. */
  readonly resultMissing?: boolean;
  /** The batch is still resolving (show `resolving…`). */
  readonly resultLoading?: boolean;
  readonly [key: string]: unknown;
}

export type MoteFlowNode = Node<MoteNodeData, "mote">;

/** A run's resolved results, indexed by content ref (the `useResultMap` shape). */
export interface ResultLookup {
  readonly byRef: ReadonlyMap<string, BatchedContentVM>;
  readonly loading: boolean;
}

/**
 * Positioned reactflow nodes (positions come from the memoized dagre layout).
 * When `results` is provided, each node carries its RESOLVED result so the DAG
 * node shows the text headline (D142.2) — the same `byRef` map the table uses,
 * so the two surfaces resolve identically from one batch round trip.
 */
export function buildFlowNodes(
  motes: readonly MoteVM[],
  positions: ReadonlyMap<string, XY>,
  results?: ResultLookup,
): MoteFlowNode[] {
  return motes.map((m) => {
    const vm = m.resultRef ? results?.byRef.get(m.resultRef) : undefined;
    return {
      id: m.moteId,
      type: "mote",
      position: positions.get(m.moteId) ?? { x: 0, y: 0 },
      data: {
        mote: m,
        resultContent: vm?.content,
        resultMissing: vm?.missing ?? false,
        resultLoading: m.resultRef ? (results?.loading ?? false) : false,
      },
      draggable: false,
    };
  });
}

/** Styled reactflow edges from the Motes' parent links (dangling dropped). */
export function buildFlowEdges(motes: readonly MoteVM[]): Edge[] {
  return buildEdges(motes).map(toRfEdge);
}
