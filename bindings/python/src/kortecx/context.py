"""PR-7 context-bundle views — a named, content-addressed collection a caller
attaches to a run (``invoke(..., context=[handle])``) so a model reasons over it.

Kept in its own module so ``types.py`` stays a thin aggregator (the Rust core's
module-per-concern discipline, GR3). SN-8: ``bundle_ref`` is SERVER-DERIVED
(blake3 over the manifest) — the client names a handle, never an identity. The
manifest lives in an off-journal ``bundles.db`` sidecar (rebuildable-to-empty),
scoped to the authoring party; a not-found / not-owned bundle is UNIFORM (no
cross-party existence oracle).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ContextBundleItem:
    """One item in a context bundle: an advisory label + a content-store ref."""

    name: str  # advisory label / context heading (display only)
    content_ref: str  # the 32-byte content-store ref, as 64 hex chars
    media_type: str  # advisory mime (display / classify only)

    @classmethod
    def from_proto(cls, it: "_g.ContextItem") -> "ContextBundleItem":
        return cls(
            name=it.name,
            content_ref=hexids.encode(it.content_ref),
            media_type=it.media_type,
        )


@dataclass(frozen=True)
class ContextBundle:
    """A context bundle's bound manifest (the governance / display view)."""

    bundle_ref: str  # server-derived manifest hash, as hex (16 bytes ⇒ 32 hex chars)
    handle: str  # the "namespace/collection/name" handle
    description: str  # advisory free-form
    items: List[ContextBundleItem]
    item_count: int

    @classmethod
    def from_proto(cls, b: "_g.ContextBundle") -> "ContextBundle":
        return cls(
            bundle_ref=hexids.encode(b.bundle_ref),
            handle=b.handle,
            description=b.description,
            items=[ContextBundleItem.from_proto(it) for it in b.items],
            item_count=b.item_count,
        )


@dataclass(frozen=True)
class PutContextBundleResult:
    """The outcome of a ``PutContextBundle`` upsert (server-derived ref + dedup)."""

    bundle_ref: str  # server-derived manifest hash, as hex
    handle: str  # echoed canonical handle
    deduplicated: bool  # an identical manifest was already bound to this handle

    @classmethod
    def from_proto(cls, r: "_g.PutContextBundleResponse") -> "PutContextBundleResult":
        return cls(
            bundle_ref=hexids.encode(r.bundle_ref),
            handle=r.handle,
            deduplicated=r.deduplicated,
        )
