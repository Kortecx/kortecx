"""RC5a durable agentic MEMORY views — a stored memory, a recall hit, and a store
receipt, as surfaced by ``StoreMemory`` / ``ListMemories`` / ``RecallMemory`` /
``ForgetMemory``.

Cross-run, per-namespace memory: what an agent LEARNED in one run and can RECALL in a
later one. SN-8: a recall hit's ``score`` is DISPLAY-ONLY — never an identity input;
the durable result is the ordered ``memory_id`` SET, matched by EXACT hash. Every
memory is scoped to the caller's own principal (server-derived) — a client never
reaches another principal's memories.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import IntEnum

from . import hexids
from .v1 import gateway_pb2 as _g


class MemoryKind(IntEnum):
    """The kind of a memory (metadata; does not change indexing). The wire values
    match ``proto.MemoryKind`` (UNSPECIFIED ⇒ SEMANTIC)."""

    UNSPECIFIED = 0
    SEMANTIC = 1
    EPISODIC = 2


@dataclass(frozen=True)
class Memory:
    """One stored memory (the episodic-log view from ``ListMemories``)."""

    memory_id: str  # hex (the content-addressed citation key)
    content: bytes
    kind: str  # "semantic" | "episodic"
    instance_id: str  # hex (the run that wrote it; all-zero = operator/SDK write)
    created_ms: int  # unix-ms (display only; off every hash)
    dim: int

    @classmethod
    def from_proto(cls, m: "_g.MemorySummary") -> "Memory":
        return cls(
            memory_id=hexids.encode(m.memory_id),
            content=m.content,
            kind=m.kind,
            instance_id=hexids.encode(m.instance_id),
            created_ms=m.created_ms,
            dim=m.dim,
        )

    @property
    def text(self) -> str:
        """The remembered bytes decoded as UTF-8 (best-effort)."""
        return self.content.decode("utf-8", errors="replace")


@dataclass(frozen=True)
class MemoryHit:
    """One recall hit: the content-addressed ref (hex), the bytes, and the
    DISPLAY-ONLY similarity score (SN-8 — never an identity input)."""

    memory_id: str  # hex
    content: bytes
    score: float

    @classmethod
    def from_proto(cls, h: "_g.MemoryHit") -> "MemoryHit":
        return cls(
            memory_id=hexids.encode(h.memory_id),
            content=h.content,
            score=h.score,
        )

    @property
    def text(self) -> str:
        """The recalled bytes decoded as UTF-8 (best-effort)."""
        return self.content.decode("utf-8", errors="replace")


@dataclass(frozen=True)
class StoreResult:
    """The outcome of a ``StoreMemory`` (content-addressed, idempotent)."""

    memory_id: str  # hex
    inserted: bool  # False ⇒ a content-addressed dedup hit
    dim: int

    @classmethod
    def from_proto(cls, r: "_g.StoreMemoryResponse") -> "StoreResult":
        return cls(
            memory_id=hexids.encode(r.memory_id),
            inserted=r.inserted,
            dim=r.dim,
        )
