"""User feedback on an answer (PR-4.1) — a 👍/👎 rating + optional note recorded
by ``SubmitFeedback`` and read back by ``ListFeedback``.

Client-origin product signal the gateway records into a rebuildable-to-empty
``feedback.db`` sidecar — AUDIT/DISPLAY ONLY, never truth, never identity, never
a digest input. Kept in its own module (the telemetry.py / module-per-concern
precedent). SN-8: the caller principal + the ``feedback_id`` are server-derived;
the SDK only hex-encodes the bytes + maps the rating enum.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from . import hexids
from .v1 import gateway_pb2 as _g

#: A 👍/👎 rating as a stable string (the wire enum, lowercased).
Rating = str  # Literal["up", "down"]


def rating_to_proto(rating: str) -> "_g.FeedbackRating":
    """Map a :data:`Rating` string to the proto rating enum (UP=1, DOWN=2)."""
    if rating == "up":
        return _g.FEEDBACK_RATING_UP
    if rating == "down":
        return _g.FEEDBACK_RATING_DOWN
    raise ValueError(f"rating must be 'up' or 'down', got {rating!r}")


def rating_from_proto(value: int) -> Optional[str]:
    """Map the proto rating enum int back to a :data:`Rating` (``None`` if unset)."""
    if value == _g.FEEDBACK_RATING_UP:
        return "up"
    if value == _g.FEEDBACK_RATING_DOWN:
        return "down"
    return None


@dataclass(frozen=True)
class FeedbackRow:
    """One recorded feedback row. ``instance_id`` is all-zero hex (``""`` after
    normalization) when the turn had no run; ``mote_id``/``content_ref`` are
    ``""`` when absent. ``rowid`` is the pagination cursor (never identity)."""

    feedback_id: str  # hex, server-derived
    rating: Optional[str]  # "up" | "down" | None
    message_id: str
    instance_id: str  # hex ("" when no run)
    mote_id: str  # hex ("" when absent)
    content_ref: str  # hex ("" when absent)
    comment: str
    recipe_handle: str
    model_id: str
    submitted_unix_ms: int  # audit-only wall clock (off every hash)
    rowid: int

    @classmethod
    def from_proto(cls, r: "_g.FeedbackRow") -> "FeedbackRow":
        def non_empty(b: bytes) -> str:
            return hexids.encode(b) if any(b) else ""

        return cls(
            feedback_id=hexids.encode(r.feedback_id),
            rating=rating_from_proto(r.rating),
            message_id=r.message_id,
            instance_id=non_empty(r.instance_id),
            mote_id=non_empty(r.mote_id),
            content_ref=non_empty(r.content_ref),
            comment=r.comment,
            recipe_handle=r.recipe_handle,
            model_id=r.model_id,
            submitted_unix_ms=r.submitted_unix_ms,
            rowid=r.rowid,
        )


@dataclass(frozen=True)
class FeedbackPage:
    """One newest-first page of :class:`FeedbackRow` plus the ``has_more`` flag."""

    rows: List[FeedbackRow]
    has_more: bool
