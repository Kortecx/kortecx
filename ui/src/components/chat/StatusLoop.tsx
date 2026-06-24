/**
 * POC-5c (D168): the New Chat status loop — a clean, single-line "what the runtime
 * is doing right now" indicator that REPLACES the inline agent-loop DAG strip
 * (`ReactProgress`). The audited per-turn action set now lives in Monitoring; the
 * chat shows only an HONEST current-phase word.
 *
 * GR15 honesty contract: this is NOT a cosmetic/random rotation. The phase WORD is
 * derived (purely) from real runtime facts — the chain's durable `ReactRound`
 * branches (`ListReactTurns`) for an agent turn, or the run projection for a plain
 * chat turn — so the word changes ONLY when a real decode → plan → tool-call →
 * observe → settle transition has happened. The only cosmetic motion is the dot.
 */

import type { UseChat } from "../../kx/use-chat";

export type StatusPhase =
  | "submitting"
  | "planning"
  | "tool-call"
  | "replanning"
  | "settling"
  | "decode"
  | "dead-letter";

export interface StatusView {
  readonly phase: StatusPhase;
  readonly text: string;
}

type ChatStatusInput = Pick<UseChat, "busy" | "reactTurns" | "activeProjection">;

/**
 * Map the live runtime facts to the current phase. Returns `null` when nothing is in
 * flight (the assistant message itself renders the answer). Pure + total → unit-testable
 * without a runtime; every branch corresponds to a real, durable fact.
 */
export function derivePhase(chat: ChatStatusInput): StatusView | null {
  if (!chat.busy) {
    return null;
  }

  const turns = chat.reactTurns;
  if (turns !== undefined) {
    // Agent turn: the LATEST durable ReactRound fact names the phase (turns are
    // ordered by (turn, callIndex); the last row is the newest settled/anchored fact).
    const last = turns.length > 0 ? turns[turns.length - 1] : undefined;
    if (!last) {
      return { phase: "planning", text: "Reasoning…" };
    }
    switch (last.branch) {
      case "dead_lettered":
        return { phase: "dead-letter", text: "Loop ended without an answer" };
      case "rejected":
        return { phase: "replanning", text: "Re-planning (a tool proposal was refused)" };
      case "tool":
        return { phase: "tool-call", text: `Calling tool ${last.toolId}@${last.toolVersion}` };
      case "answer":
        return { phase: "settling", text: "Settling the answer" };
      default:
        return { phase: "planning", text: `Reasoning… (turn ${last.turn}/${last.maxTurns})` };
    }
  }

  // Plain chat / RAG turn: the run projection is decoding the answer.
  if (chat.activeProjection && chat.activeProjection.motes.length > 0) {
    return { phase: "decode", text: "Generating…" };
  }
  return { phase: "submitting", text: "Submitting…" };
}

export function StatusLoop({ chat }: { chat: ChatStatusInput }) {
  const view = derivePhase(chat);
  if (!view) {
    return null;
  }
  return (
    <p
      className="status-loop"
      data-testid="status-loop"
      data-phase={view.phase}
      aria-live="polite"
      aria-atomic="true"
    >
      <span className="status-loop__dot" aria-hidden="true" />
      <span className="status-loop__text" data-testid="status-loop-text">
        {view.text}
      </span>
    </p>
  );
}
