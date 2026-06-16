"""T3.7 Datasets data-plane (RAG) views — a dataset summary, a retrieval hit, and
an ingest outcome, as surfaced by ``ListDatasets`` / ``QueryDataset`` /
``IngestDocuments``.

Kept in its own module so ``types.py`` stays a thin aggregator, mirroring the Rust
core's module-per-concern discipline. SN-8: a hit's ``score`` is DISPLAY-ONLY —
never an identity input; the retrieval result a downstream consumer trusts is the
ordered ``content_ref`` SET, matched by EXACT hash. Embedding is pluggable: pass a
client-computed ``embedding`` (the FFI-free path, e.g. via HuggingFace
``sentence-transformers``) or omit it to let a gateway with the ``inference``
feature embed the text server-side.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Sequence

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class DatasetSummary:
    """One dataset (RAG corpus) in a ``ListDatasets`` enumeration."""

    dataset_id: str
    name: str
    doc_count: int
    dim: int
    created_ms: int  # unix-ms create time (display only; off every hash)

    @classmethod
    def from_proto(cls, d: "_g.DatasetSummary") -> "DatasetSummary":
        return cls(
            dataset_id=d.dataset_id,
            name=d.name,
            doc_count=d.doc_count,
            dim=d.dim,
            created_ms=d.created_ms,
        )


@dataclass(frozen=True)
class DatasetHit:
    """One retrieval hit: the content-addressed ref (hex), the document bytes, and
    the DISPLAY-ONLY similarity score (SN-8 — never an identity input)."""

    content_ref: str  # hex
    content: bytes
    score: float

    @classmethod
    def from_proto(cls, h: "_g.DatasetHit") -> "DatasetHit":
        return cls(
            content_ref=hexids.encode(h.content_ref),
            content=h.content,
            score=h.score,
        )

    @property
    def text(self) -> str:
        """The retrieved document bytes decoded as UTF-8 (best-effort)."""
        return self.content.decode("utf-8", errors="replace")


@dataclass(frozen=True)
class FuzzyHit:
    """Slice-B advisory fuzzy-in / exact-out discovery hit (``FuzzyDiscovery``):
    the content-addressed ref (hex) + a DISPLAY-ONLY basis-point score (SN-8 —
    never an identity input). Join back to bytes with an EXACT ``get_content`` on
    the ref ("fuzzy in, exact out")."""

    content_ref: str  # hex — the EXACT-OUT join key
    score_bp: int  # 0..=10000 display-only basis points

    @classmethod
    def from_proto(cls, h: "_g.FuzzyHit") -> "FuzzyHit":
        return cls(content_ref=hexids.encode(h.content_ref), score_bp=h.score_bp)

    @property
    def score(self) -> float:
        """The similarity as a 0..1 fraction (display only)."""
        return self.score_bp / 10_000


@dataclass(frozen=True)
class IngestResult:
    """The outcome of an ``IngestDocuments`` call (server-derived counts)."""

    dataset_id: str
    doc_count: int
    inserted: int  # new distinct docs added by this call (post content-addressed dedup)
    dim: int

    @classmethod
    def from_proto(cls, r: "_g.IngestDocumentsResponse") -> "IngestResult":
        return cls(
            dataset_id=r.dataset_id,
            doc_count=r.doc_count,
            inserted=r.inserted,
            dim=r.dim,
        )


@dataclass
class IngestDocument:
    """One document to ingest. ``content`` is the retrievable payload (always). An
    OPTIONAL ``embedding`` takes the FFI-free client-vector path; omit it to let a
    server embedder (the ``inference`` feature) embed ``content``.

    ``doc_id`` and ``metadata`` are RESERVED (forward-compat): accepted on the wire
    but NOT YET persisted or returned. The durable id is always the server-derived
    content hash (SN-8), so ``doc_id`` is advisory; per-doc metadata is a planned add."""

    content: bytes
    embedding: Optional[Sequence[float]] = None
    doc_id: Optional[str] = None
    metadata: Optional[Dict[str, str]] = field(default=None)

    def to_proto(self) -> "_g.IngestDocument":
        msg = _g.IngestDocument(content=self.content)
        if self.embedding:
            msg.embedding.extend(self.embedding)
        if self.doc_id is not None:
            msg.doc_id = self.doc_id
        if self.metadata:
            for k, v in self.metadata.items():
                msg.metadata[k] = v
        return msg


def _to_documents(
    documents: Sequence["IngestDocument | bytes"],
) -> List["_g.IngestDocument"]:
    """Accept either :class:`IngestDocument` values or raw ``bytes`` (sugar for a
    text-only doc that the server will embed)."""
    out: List["_g.IngestDocument"] = []
    for d in documents:
        if isinstance(d, IngestDocument):
            out.append(d.to_proto())
        else:
            out.append(_g.IngestDocument(content=d))
    return out
