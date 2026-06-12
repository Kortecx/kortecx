"""Batch A content views — a ``PutContent`` upload outcome and one
``GetContentBatch`` item.

Kept in its own module so ``types.py`` stays a thin aggregator (the Rust core's
module-per-concern discipline, GR3). SN-8: ``content_ref`` is SERVER-DERIVED
(blake3 over the payload) — the client never names an identity. An upload is a
CONTENT-STORE write, never a journal write; ``media_type``/``filename`` are
advisory audit fields. A batch item whose ref was unauthorized / missing /
malformed comes back UNIFORMLY EMPTY (``payload == b"" and full_size == 0``) —
no existence oracle (D120.1).
"""

from __future__ import annotations

from dataclasses import dataclass

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class PutResult:
    """The outcome of a ``PutContent`` upload (server-derived ref + dedup flag)."""

    content_ref: str  # the server-derived blake3 ref, as 64 hex chars
    size: int  # stored byte count
    deduplicated: bool  # an identical blob already existed (advisory display state)

    @classmethod
    def from_proto(cls, r: "_g.PutContentResponse") -> "PutResult":
        return cls(
            content_ref=hexids.encode(r.content_ref),
            size=r.size,
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class ContentItem:
    """One ``GetContentBatch`` item, in request order."""

    content_ref: str  # the requested ref echoed back, as hex
    payload: bytes  # EMPTY when unauthorized/missing/malformed (uniform)
    truncated: bool  # the payload was cut at the per-item clamp
    full_size: int  # the stored size; 0 when unauthorized/missing (uniform, honest)

    @classmethod
    def from_proto(cls, i: "_g.ContentBatchItem") -> "ContentItem":
        return cls(
            content_ref=hexids.encode(i.content_ref),
            payload=i.payload,
            truncated=i.truncated,
            full_size=i.full_size,
        )

    @property
    def missing(self) -> bool:
        """True iff the server returned the uniform empty item for this ref."""
        return not self.payload and self.full_size == 0

    @property
    def text(self) -> str:
        """The payload decoded as UTF-8 (best-effort) — for text content."""
        return self.payload.decode("utf-8", errors="replace")
