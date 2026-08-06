/**
 * Pure DAG-edge → reactflow-edge visual mapping (no React). DATA edges are solid;
 * CONTROL edges are dashed; a non-cascade CONTROL edge is dimmed (it does not
 * propagate failure to its child). A DERIVED edge — one the reader synthesised rather
 * than read off a Mote — is finely dotted with an OPEN arrow head, so it can never be
 * mistaken for a recorded parent. Isolated so the styling is unit-testable.
 */

import { MarkerType } from "@xyflow/react";
import type { Edge } from "@xyflow/react";
import type { GraphEdge } from "./dag-graph";

/** Map one DAG edge to its styled reactflow edge. `branch` marks a swarm
 *  branch→gather fan-in edge (PR-B) so the fan-in reads at a glance; the default
 *  path is byte-identical to before. */
export function toRfEdge(e: GraphEdge, opts: { branch?: boolean } = {}): Edge {
  const isControl = e.edgeKind === "control";
  const branch = opts.branch ? " dag-edge--branch" : "";
  const derived = e.derived ? " dag-edge--derived" : "";
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    className: `dag-edge dag-edge--${e.edgeKind}${e.nonCascade ? " dag-edge--noncascade" : ""}${branch}${derived}`,
    // An OPEN head on a derived edge: a second, non-colour signal that this link was
    // inferred from the run's turn record, not read from the graph.
    markerEnd: {
      type: e.derived ? MarkerType.Arrow : MarkerType.ArrowClosed,
      width: 14,
      height: 14,
    },
    style: {
      strokeDasharray: e.derived ? "2 5" : isControl ? "5 4" : undefined,
      opacity: e.nonCascade ? 0.4 : 0.85,
    },
    data: { edgeKind: e.edgeKind, nonCascade: e.nonCascade, derived: e.derived ?? false },
  };
}
