/**
 * Split a model reply into its leading `<think>…</think>` REASONING block and the
 * ANSWER that follows — the presentation-side mirror of the server's
 * `strip_reasoning_preamble` (kx-planner/decode.rs). The reasoning is ALREADY
 * durably committed in the turn's result bytes (raw-commit) + the `ReactRound`
 * facts; this is PURELY a display split (SN-8 — it cannot gate capture).
 *
 * Leading-block ONLY (never a mid-string scan). An UNCLOSED `<think>` fails OPEN
 * to the whole text as the answer (never hide content from the user — the inverse
 * of the planner's fail-closed, which needs a strict plan).
 */

const OPEN = "<think>";
const CLOSE = "</think>";

export interface SplitReply {
  /** The leading reasoning block, or `undefined` when there is none. */
  readonly reasoning?: string;
  /** The answer the user reads (always present; may be empty for reasoning-only). */
  readonly answer: string;
}

export function splitReasoning(text: string): SplitReply {
  const trimmed = text.replace(/^\s+/, "");
  if (!trimmed.startsWith(OPEN)) {
    return { answer: text };
  }
  const rest = trimmed.slice(OPEN.length);
  const idx = rest.indexOf(CLOSE);
  if (idx === -1) {
    // Unclosed reasoning — fail open to the whole text (don't hide it).
    return { answer: text };
  }
  const reasoning = rest.slice(0, idx).trim();
  const answer = rest.slice(idx + CLOSE.length).replace(/^\s+/, "");
  return { reasoning: reasoning.length > 0 ? reasoning : undefined, answer };
}
