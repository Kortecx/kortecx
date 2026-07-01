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
    access_count: int = 0  # RC5b: recall count (salience; display only)
    last_accessed_ms: int = 0  # RC5b: last recall time, unix-ms (0 = never)
    tombstoned_ms: int = 0  # RC5b: decay tombstone time (0 = live; >0 = decayed, restorable)

    @classmethod
    def from_proto(cls, m: "_g.MemorySummary") -> "Memory":
        return cls(
            memory_id=hexids.encode(m.memory_id),
            content=m.content,
            kind=m.kind,
            instance_id=hexids.encode(m.instance_id),
            created_ms=m.created_ms,
            dim=m.dim,
            access_count=m.access_count,
            last_accessed_ms=m.last_accessed_ms,
            tombstoned_ms=m.tombstoned_ms,
        )

    @property
    def text(self) -> str:
        """The remembered bytes decoded as UTF-8 (best-effort)."""
        return self.content.decode("utf-8", errors="replace")

    @property
    def is_decayed(self) -> bool:
        """True if this memory has been decayed (soft-tombstoned; restorable)."""
        return self.tombstoned_ms > 0


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


@dataclass(frozen=True)
class DecayCandidate:
    """One memory a decay policy matched (RC5b) — a reversible soft-tombstone, never a
    hard delete."""

    memory_id: str  # hex
    content: bytes
    kind: str  # "semantic" | "episodic"
    created_ms: int
    access_count: int
    last_accessed_ms: int
    age_days: int

    @classmethod
    def from_proto(cls, c: "_g.DecayCandidate") -> "DecayCandidate":
        return cls(
            memory_id=hexids.encode(c.memory_id),
            content=c.content,
            kind=c.kind,
            created_ms=c.created_ms,
            access_count=c.access_count,
            last_accessed_ms=c.last_accessed_ms,
            age_days=c.age_days,
        )

    @property
    def text(self) -> str:
        """The memory bytes decoded as UTF-8 (best-effort)."""
        return self.content.decode("utf-8", errors="replace")


@dataclass(frozen=True)
class DecayReport:
    """The outcome of a ``DecayMemory`` sweep (RC5b). ``dry_run`` ⇒ a preview that
    evicted nothing; evictions are reversible via ``restore_memory``."""

    candidates: tuple[DecayCandidate, ...]
    would_evict: int
    evicted: int
    kept: int
    dry_run: bool

    @classmethod
    def from_proto(cls, r: "_g.DecayMemoryResponse") -> "DecayReport":
        return cls(
            candidates=tuple(DecayCandidate.from_proto(c) for c in r.candidates),
            would_evict=r.would_evict,
            evicted=r.evicted,
            kept=r.kept,
            dry_run=r.dry_run,
        )


@dataclass(frozen=True)
class MemoryStats:
    """Namespace memory statistics (RC5b) — live counts by kind, tombstoned count, dim,
    the embed fingerprint, and the live age range."""

    total: int
    semantic: int
    episodic: int
    tombstoned: int
    dim: int
    embed_fingerprint: str
    oldest_ms: int
    newest_ms: int
    namespace: str

    @classmethod
    def from_proto(cls, s: "_g.MemoryStatsResponse") -> "MemoryStats":
        return cls(
            total=s.total,
            semantic=s.semantic,
            episodic=s.episodic,
            tombstoned=s.tombstoned,
            dim=s.dim,
            embed_fingerprint=s.embed_fingerprint,
            oldest_ms=s.oldest_ms,
            newest_ms=s.newest_ms,
            namespace=s.namespace,
        )
