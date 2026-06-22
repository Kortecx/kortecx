"""Decode a committed ``CriticVerdict`` to a readable summary (T-AGENT2).

The opt-in LLM-judge gate (``kx/recipes/judge``) commits a ``CriticVerdict`` as its
terminal result. Its canonical wire encoding is a tiny, stable byte layout — a
2-byte little-endian ``CRITIC_SCHEMA_VERSION`` prefix followed by fixed-int bincode
of the verdict enum — so the SDK decodes the VALID/INVALID summary directly,
without a bincode dependency. Display-only (SN-8): the summary never authorizes
anything; the runtime's promotion gate reads the committed fact, not this string.
"""

from __future__ import annotations

from typing import Optional

# Must match ``kx_critic_types::CRITIC_SCHEMA_VERSION``.
_CRITIC_SCHEMA_VERSION = 1

# ``CriticReason`` variant discriminants (the enum's declaration order; JudgeRejected
# is the trailing T-AGENT2 addition). Only used for a human-readable summary.
_REASONS = {
    0: "schema mismatch",
    1: "duplicate detected",
    2: "stat out of bounds",
    3: "PII leak",
    4: "unparseable input",
    5: "judge rejected",
}
_JUDGE_CODES = {
    0: "judge: answer did not satisfy the rubric",
    1: "judge: response was unparseable (fail-closed)",
}


def decode_critic_verdict(payload: bytes) -> Optional[str]:
    """Decode ``payload`` as a ``CriticVerdict`` → ``"valid"`` / ``"invalid: <reason>"``.

    Returns ``None`` for any payload that is not a well-formed verdict (a model
    answer, a tool observation, an empty/short buffer, an unknown schema version),
    so callers fall back to the raw bytes. Total + panic-free over arbitrary input.
    """
    if len(payload) < 6:
        return None
    if int.from_bytes(payload[0:2], "little") != _CRITIC_SCHEMA_VERSION:
        return None
    variant = int.from_bytes(payload[2:6], "little")
    if variant == 0:  # CriticVerdict::Valid
        return "valid"
    if variant != 1:  # not Invalid either ⇒ not a verdict
        return None
    if len(payload) < 10:
        return "invalid"
    reason = int.from_bytes(payload[6:10], "little")
    detail = _REASONS.get(reason, "rejected")
    if reason == 5 and len(payload) >= 12:  # JudgeRejected { reason_code: u16 }
        code = int.from_bytes(payload[10:12], "little")
        detail = _JUDGE_CODES.get(code, f"judge: rejected (code {code})")
    return f"invalid: {detail}"
