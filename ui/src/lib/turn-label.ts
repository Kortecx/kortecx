/**
 * The words one agent turn is described by — a SINGLE source, read by both surfaces
 * that describe a turn: the Timeline's cards and the run graph's nodes.
 *
 * Shared rather than duplicated on purpose. The two surfaces render the same turn from
 * the same `ListReactTurns` row, and a reader comparing them is entitled to read the
 * same words in both places; two copies of the switch below would drift apart on the
 * first branch anyone adds. The graph previously showed a Mote hash and an
 * `nd_class` enum for the same turn the Timeline called `Turn 0 · MCP · mcp-echo/echo@1`.
 */

import { type StepType, classifyStep } from "./step-kind";

/**
 * What a turn must carry to be described. Structural, so the Timeline's full row VM and
 * the graph's narrowed projection row both satisfy it with no conversion step.
 */
export interface DescribableTurn {
  /** `"pending" | "answer" | "tool" | "rejected" | "dead_lettered"`. */
  readonly branch: string;
  /** The fired tool for a `tool` branch; "" otherwise. */
  readonly toolId: string;
  readonly toolVersion: string;
}

/** The badge step-type for a turn: a `tool` turn is classified from its fired tool
 *  (no GetMoteDetail — cheaper + honest); every other branch is the model reasoning /
 *  answer turn. */
export function turnStepType(t: DescribableTurn): StepType {
  return t.branch === "tool" ? classifyStep("TOOL", { [t.toolId]: "" }) : "model";
}

/**
 * How to name the OBSERVATION a turn produced — the Mote that actually fired the
 * tool the turn proposed.
 *
 * These nodes were the half the turn-label fix did not cover. A turn reads
 * `Turn 0 · MCP · mcp-echo/echo@1`; the observation hanging off it read
 * `bca492fe…a2b3` with a `WORLD_MUTATING` badge — a content hash and a
 * determinism-class enum, which is machinery, not what happened. It IS the
 * result of that turn's tool call, and the turn already knows which tool.
 *
 * Derived in the reader from the observation's parent edge, so it needs no new RPC
 * and cannot disagree with the turn it hangs off.
 */
export function observationLabel(parent: DescribableTurn): string {
  return parent.branch === "tool" && parent.toolId
    ? `result of ${parent.toolId}@${parent.toolVersion}`
    : "tool result";
}

/** A short human phrase for the turn's branch (the card's status line). */
export function branchLabel(t: DescribableTurn): string {
  switch (t.branch) {
    case "tool":
      return `${t.toolId}@${t.toolVersion}`;
    case "answer":
      return "answered";
    case "rejected":
      return "tool proposal rejected";
    case "dead_lettered":
      return "dead-lettered";
    default:
      return "thinking…";
  }
}
