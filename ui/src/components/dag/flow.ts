/**
 * Pure adapters that assemble reactflow `nodes`/`edges` from the projection +
 * the memoized layout (no React). Keeping this out of `MoteDag.tsx` lets the
 * node/edge construction be unit-tested directly and keeps the component thin.
 */

import type { Edge, Node } from "@xyflow/react";
import type { BatchedContentVM } from "../../kx/use-content-batch";
import type { MoteVM } from "../../kx/use-projection";
import { isTerminalState, stateVisual } from "../../lib/colors";
import type { DecodedContent } from "../../lib/content-decode";
import type { StepType } from "../../lib/step-kind";
import { buildEdges } from "./dag-graph";
import type { GraphEdge } from "./dag-graph";
import type { AgenticTurnLabel } from "./derived-lineage";
import { toRfEdge } from "./edges";
import type { XY } from "./layout";

/**
 * Fallback hex per state tone, used ONLY where the document cannot be measured
 * (jsdom, SSR, a detached render). These are the LIGHT-theme `--t-*` values.
 *
 * ⚠ They used to be the whole story, and that was one of two reasons the minimap
 * rendered as a flat panel: a mirrored table cannot follow a theme, so every node
 * painted a light-theme colour on a dark canvas. The live path now reads the token
 * itself, so `app.css` is the single source and dark theme is not a second table to
 * keep in sync.
 */
const TONE_UNKNOWN_HEX = "#4b5563";
const TONE_FALLBACK_HEX: Readonly<Record<string, string>> = {
  pending: "#475569",
  scheduled: "#b45309",
  committed: "#047857",
  failed: "#dc2626",
  repudiated: "#c2410c",
  inconsistent: "#7c3aed",
  unknown: TONE_UNKNOWN_HEX,
};

/** Resolved `--t-<tone>` values, keyed `<theme>:<tone>`. `getComputedStyle` is far too
 *  expensive to call per node per frame; the theme is in the key so a toggle re-resolves
 *  rather than serving the previous theme's colour. */
const toneHexCache = new Map<string, string>();

/**
 * MiniMap node fill for a Mote, keyed by its state tone (single source: `stateVisual`).
 *
 * The MiniMap paints SVG `fill` ATTRIBUTES, which — unlike CSS — do not resolve
 * `var(--t-*)`, so the value has to be a concrete hex. It is read from the same
 * custom property the rest of the console styles from, rather than mirrored.
 */
export function miniMapColor(stateCode: number): string {
  const { tone } = stateVisual(stateCode);
  const fallback = TONE_FALLBACK_HEX[tone] ?? TONE_UNKNOWN_HEX;
  if (typeof document === "undefined") {
    return fallback;
  }
  const root = document.documentElement;
  const key = `${root.dataset.theme ?? "light"}:${tone}`;
  const hit = toneHexCache.get(key);
  if (hit !== undefined) {
    return hit;
  }
  const resolved = getComputedStyle(root).getPropertyValue(`--t-${tone}`).trim() || fallback;
  toneHexCache.set(key, resolved);
  return resolved;
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
  /** PR-B: this Mote is the swarm gather (fan-in) sink — marks it on the canvas. */
  readonly swarmRole?: "gather";
  /** PR-D: the high-level step type (model/MCP/connector/tool/action) for the review. */
  readonly stepType?: StepType;
  /** This Mote is an agent TURN, and these are the turn's own facts — the node names
   *  the turn and its tool instead of a Mote hash. Absent for a non-agentic Mote. */
  readonly turnLabel?: AgenticTurnLabel;
  /** This Mote is the OBSERVATION a turn produced (its only parent is that turn), and
   *  this is the PARENT turn's label — so the node can say which turn's tool result it
   *  is instead of showing a hash. Absent for everything that is not an observation. */
  readonly observationOf?: AgenticTurnLabel;
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
  gatherId?: string,
  stepKinds?: ReadonlyMap<string, StepType>,
  turnLabels?: ReadonlyMap<string, AgenticTurnLabel>,
): MoteFlowNode[] {
  return motes.map((m) => {
    const vm = m.resultRef ? results?.byRef.get(m.resultRef) : undefined;
    const turnLabel = turnLabels?.get(m.moteId);
    return {
      id: m.moteId,
      type: "mote",
      position: positions.get(m.moteId) ?? { x: 0, y: 0 },
      data: {
        mote: m,
        resultContent: vm?.content,
        resultMissing: vm?.missing ?? false,
        resultLoading: m.resultRef ? (results?.loading ?? false) : false,
        swarmRole: m.moteId === gatherId ? "gather" : undefined,
        stepType: stepKinds?.get(m.moteId),
        turnLabel,
        // An OBSERVATION is not a turn and carries exactly one parent: the turn whose
        // tool call it fired. Resolving that parent's label here is what lets the node
        // say `Turn 0 · result of mcp-echo/echo@1` instead of a Mote hash. Guarded on
        // `!turnLabel` so a turn never labels itself as its own observation.
        observationOf: turnLabel ? undefined : observationParentLabel(m, turnLabels),
      },
      draggable: false,
    };
  });
}

/** The label of the TURN a Mote is the observation of, or `undefined` if it is not one.
 *  An observation has exactly one parent and that parent is a turn — anything else (a
 *  fan-in, a root, an ordinary DAG step) is deliberately left alone. */
function observationParentLabel(
  m: MoteVM,
  turnLabels?: ReadonlyMap<string, AgenticTurnLabel>,
): AgenticTurnLabel | undefined {
  const only = turnLabels && m.parents.length === 1 ? m.parents[0] : undefined;
  return only ? turnLabels?.get(only.parentId) : undefined;
}

/** Styled reactflow edges from the Motes' parent links (dangling dropped). PR-B:
 *  `branchEdges` (edge ids) mark the swarm branch→gather fan-in for highlighting.
 *  `derived` appends reader-synthesised edges (an agent's turn order, which the runtime
 *  records off-DAG) AFTER the durable ones — they carry their own visual treatment. */
export function buildFlowEdges(
  motes: readonly MoteVM[],
  branchEdges?: ReadonlySet<string>,
  derived: readonly GraphEdge[] = [],
): Edge[] {
  // Motion: an edge is LIVE while the work it feeds has not settled. Derived
  // from the projection's own state codes, so the animation is a reading of the run
  // rather than a decoration on it — a finished graph has no live edges at all, and a
  // replayed one animates nothing.
  const unsettled = new Set(
    motes.filter((m) => !isTerminalState(m.stateCode)).map((m) => m.moteId),
  );
  return [...buildEdges(motes), ...derived].map((e) =>
    toRfEdge(e, {
      branch: branchEdges?.has(e.id) ?? false,
      live: unsettled.has(e.target),
    }),
  );
}
