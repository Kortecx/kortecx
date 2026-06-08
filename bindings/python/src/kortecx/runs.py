"""UI-2 run-summary view — one registered run instance enumerated by ``ListRuns``.

Kept in its own module so ``types.py`` stays a thin aggregator, mirroring the Rust
core's module-per-concern discipline. SN-8: every id is server-derived; the SDK
only hex-encodes the bytes. ``registered_unix_ms`` is an audit-only wall-clock
(off every hash) — a legitimate "started at", never identity.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class RunSummary:
    """One registered run instance: hex ids + the registered seq (pagination
    cursor) + the registered wall-clock (unix-ms; audit-only)."""

    instance_id: str  # hex
    recipe_fingerprint: str  # hex
    registered_seq: int
    registered_unix_ms: int

    @classmethod
    def from_proto(cls, r: "_g.RunSummary") -> "RunSummary":
        return cls(
            instance_id=hexids.encode(r.instance_id),
            recipe_fingerprint=hexids.encode(r.recipe_fingerprint),
            registered_seq=r.registered_seq,
            registered_unix_ms=r.registered_unix_ms,
        )


@dataclass(frozen=True)
class RunPage:
    """One newest-first page of :class:`RunSummary` plus the ``has_more`` flag."""

    runs: List[RunSummary]
    has_more: bool
