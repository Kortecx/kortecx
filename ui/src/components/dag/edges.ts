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
export function toRfEdge(e: GraphEdge, opts: { branch?: boolean; live?: boolean } = {}): Edge {
  const isControl = e.edgeKind === "control";
  const branch = opts.branch ? " dag-edge--branch" : "";
  const derived = e.derived ? " dag-edge--derived" : "";
  // An edge INTO work that has not settled draws its dash moving, so a running
  // graph looks like it is running. Purely a class — the animation is CSS keyframes on
  // `stroke-dashoffset`, which costs no eager JS and is silenced by the
  // `prefers-reduced-motion` block beside it. State-driven: it goes away when the
  // target Mote reaches a terminal state, so it can never animate a finished graph.
  const live = opts.live ? " dag-edge--live" : "";
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    className: `dag-edge dag-edge--${e.edgeKind}${e.nonCascade ? " dag-edge--noncascade" : ""}${branch}${derived}${live}`,
    // An OPEN head on a derived edge: a second, non-colour signal that this link was
    // inferred from the run's turn record, not read from the graph.
    markerEnd: {
      type: e.derived ? MarkerType.Arrow : MarkerType.ArrowClosed,
      width: 14,
      height: 14,
    },
    style: {
      // A derived edge stays visually distinct from a durable one, but `2 5` — two on,
      // five off — is 29% ink, and at default zoom that thinned into the background:
      // the turn order, the one thing a reader most wants from an agentic run, was the
      // least legible line on the canvas. `5 4` inverts the ratio to 56% while keeping
      // the dash unmistakably a dash. Colour is not the only signal either way (the
      // marker head is open on a derived edge, and the legend names them).
      // ⚠ Keep in step with `.dag-edge--derived` in app.css, which sets the same dash
      // (and the stroke colour, which has no inline counterpart). Both are needed: the
      // inline style wins on the path, the class carries the colour.
      strokeDasharray: e.derived ? "5 4" : isControl ? "5 4" : undefined,
      opacity: e.nonCascade ? 0.4 : 0.85,
    },
    data: {
      edgeKind: e.edgeKind,
      nonCascade: e.nonCascade,
      derived: e.derived ?? false,
      live: opts.live ?? false,
    },
  };
}
