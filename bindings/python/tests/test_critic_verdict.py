"""T-AGENT2: decode a committed ``CriticVerdict`` (the ``kx/recipes/judge`` terminal).

Pins the SDK decoder against the EXACT ``kx_critic_types::CriticVerdict::encode``
byte layout (2-byte LE schema version ‖ fixed-int bincode) — a pure unit test, no
serve. Mirrors the CLI ``critic_verdict_summary`` + the TS ``decodeCriticVerdict``.
"""

from kortecx import decode_critic_verdict

# Canonical encodings (must match the Rust `CriticVerdict::encode` bytes the
# kx-critic-types `trailing_judge_variants_are_byte_neutral` test pins).
_VALID = bytes([1, 0, 0, 0, 0, 0])  # version 1 ‖ enum variant 0 (Valid)
# version 1 ‖ Invalid (variant 1) ‖ CriticReason JudgeRejected (variant 5) ‖ code u16
_INVALID_JUDGE_0 = bytes([1, 0, 1, 0, 0, 0, 5, 0, 0, 0, 0, 0])
_INVALID_JUDGE_1 = bytes([1, 0, 1, 0, 0, 0, 5, 0, 0, 0, 1, 0])


def test_decode_valid() -> None:
    assert decode_critic_verdict(_VALID) == "valid"


def test_decode_invalid_judge_reasons() -> None:
    assert decode_critic_verdict(_INVALID_JUDGE_0) == (
        "invalid: judge: answer did not satisfy the rubric"
    )
    assert decode_critic_verdict(_INVALID_JUDGE_1) == (
        "invalid: judge: response was unparseable (fail-closed)"
    )


def test_non_verdict_payload_is_none() -> None:
    # A plain model answer / short / wrong-version buffer is left alone.
    assert decode_critic_verdict(b"Paris") is None
    assert decode_critic_verdict(b"") is None
    assert decode_critic_verdict(bytes([2, 0, 0, 0, 0, 0])) is None  # wrong version
