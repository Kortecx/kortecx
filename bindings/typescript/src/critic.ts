/**
 * Decode a committed `CriticVerdict` to a readable summary (T-AGENT2).
 *
 * The opt-in LLM-judge gate (`kx/recipes/judge`) commits a `CriticVerdict` as its
 * terminal result. Its canonical wire encoding is a tiny, stable byte layout — a
 * 2-byte little-endian `CRITIC_SCHEMA_VERSION` prefix followed by fixed-int bincode
 * of the verdict enum — so the SDK decodes the VALID/INVALID summary directly,
 * without a bincode dependency. Platform-neutral (no Node imports): re-exported by
 * both the node and web entries via `common`. Display-only (SN-8): the summary
 * never authorizes anything; the runtime's promotion gate reads the committed fact.
 */

/** Must match `kx_critic_types::CRITIC_SCHEMA_VERSION`. */
const CRITIC_SCHEMA_VERSION = 1;

/** `CriticReason` variant discriminants (declaration order; `JudgeRejected` is the
 *  trailing T-AGENT2 addition). Used only for a human-readable summary. */
const REASONS: Record<number, string> = {
  0: "schema mismatch",
  1: "duplicate detected",
  2: "stat out of bounds",
  3: "PII leak",
  4: "unparseable input",
  5: "judge rejected",
};
const JUDGE_CODES: Record<number, string> = {
  0: "judge: answer did not satisfy the rubric",
  1: "judge: response was unparseable (fail-closed)",
};

// Callers length-guard before reading; `?? 0` satisfies `noUncheckedIndexedAccess`
// without changing behavior (the fallback is never reached in-bounds).
function u16le(b: Uint8Array, off: number): number {
  return (b[off] ?? 0) | ((b[off + 1] ?? 0) << 8);
}
function u32le(b: Uint8Array, off: number): number {
  // `>>> 0` keeps it an unsigned 32-bit value.
  return (
    ((b[off] ?? 0) |
      ((b[off + 1] ?? 0) << 8) |
      ((b[off + 2] ?? 0) << 16) |
      ((b[off + 3] ?? 0) << 24)) >>>
    0
  );
}

/**
 * Decode `payload` as a `CriticVerdict` → `"valid"` / `"invalid: <reason>"`.
 *
 * Returns `null` for any payload that is not a well-formed verdict (a model answer,
 * a tool observation, an empty/short buffer, an unknown schema version), so callers
 * fall back to the raw bytes. Total + never throws over arbitrary input.
 */
export function decodeCriticVerdict(payload: Uint8Array): string | null {
  if (payload.length < 6) return null;
  if (u16le(payload, 0) !== CRITIC_SCHEMA_VERSION) return null;
  const variant = u32le(payload, 2);
  if (variant === 0) return "valid"; // CriticVerdict::Valid
  if (variant !== 1) return null; // not Invalid either ⇒ not a verdict
  if (payload.length < 10) return "invalid";
  const reason = u32le(payload, 6);
  let detail = REASONS[reason] ?? "rejected";
  if (reason === 5 && payload.length >= 12) {
    // JudgeRejected { reason_code: u16 }
    const code = u16le(payload, 10);
    detail = JUDGE_CODES[code] ?? `judge: rejected (code ${code})`;
  }
  return `invalid: ${detail}`;
}
