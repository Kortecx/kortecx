/**
 * Pure DAG-edge → reactflow-edge visual mapping (no React). DATA edges are solid;
 * CONTROL edges are dashed; a non-cascade CONTROL edge is dimmed (it does not
 * propagate failure to its child). Isolated so the styling is unit-testable.
 */

import { MarkerType } from "@xyflow/react";
import type { Edge } from "@xyflow/react";
import type { GraphEdge } from "./dag-graph";

/** Map one DAG edge to its styled reactflow edge. */
export function toRfEdge(e: GraphEdge): Edge {
  const isControl = e.edgeKind === "control";
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    className: `dag-edge dag-edge--${e.edgeKind}${e.nonCascade ? " dag-edge--noncascade" : ""}`,
    markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14 },
    style: {
      strokeDasharray: isControl ? "5 4" : undefined,
      opacity: e.nonCascade ? 0.4 : 0.85,
    },
    data: { edgeKind: e.edgeKind, nonCascade: e.nonCascade },
  };
}
