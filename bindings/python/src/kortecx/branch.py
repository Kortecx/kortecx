"""D155 branch views — a named, content-addressed ``{path -> ContentRef}``
manifest over operator-approved host files. A caller snapshots confined host
files (under ``KX_SERVE_FS_ROOT``, default-OFF) INTO the content store and the
agent loop edits them IN-CAS (the host is never written in Phase-A).

Kept in its own module so ``types.py`` stays a thin aggregator.
``branch_ref`` is SERVER-DERIVED (blake3 over the manifest) — the client names a
handle, never an identity. The manifest lives in an off-journal ``branches.db``
sidecar (rebuildable-to-empty), scoped to the authoring party; a not-found /
not-owned branch is UNIFORM (no cross-party existence oracle).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class BranchItem:
    """One manifest entry: a snapshot-relative path + its content-store ref."""

    path: str  # the snapshot-relative path (manifest key + display)
    content_ref: str  # the 32-byte content-store ref, as 64 hex chars

    @classmethod
    def from_proto(cls, it: "_g.BranchItem") -> "BranchItem":
        return cls(path=it.path, content_ref=hexids.encode(it.content_ref))


@dataclass(frozen=True)
class Branch:
    """A branch's resolved manifest (the governance / display view + edit source)."""

    branch_ref: str  # server-derived manifest hash, as hex (16 bytes ⇒ 32 hex chars)
    handle: str  # the "namespace/collection/name" handle
    parent_handle: str  # the CoW parent handle (lineage); "" = a root branch
    description: str  # advisory free-form
    items: List[BranchItem]
    item_count: int

    @classmethod
    def from_proto(cls, b: "_g.Branch") -> "Branch":
        return cls(
            branch_ref=hexids.encode(b.branch_ref),
            handle=b.handle,
            parent_handle=b.parent_handle,
            description=b.description,
            items=[BranchItem.from_proto(it) for it in b.items],
            item_count=b.item_count,
        )


@dataclass(frozen=True)
class CreateBranchResult:
    """The outcome of a ``CreateBranch`` upsert (server-derived ref + dedup)."""

    branch_ref: str  # server-derived manifest hash, as hex
    handle: str  # echoed canonical handle
    deduplicated: bool  # an identical manifest already existed

    @classmethod
    def from_proto(cls, r: "_g.CreateBranchResponse") -> "CreateBranchResult":
        return cls(
            branch_ref=hexids.encode(r.branch_ref),
            handle=r.handle,
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class SnapshotResult:
    """The outcome of a ``SnapshotInto`` — the resolved manifest + ingest count."""

    branch_ref: str  # server-derived manifest hash, as hex
    handle: str  # echoed canonical handle
    ingested: int  # how many paths were read into CAS this call
    items: List[BranchItem]  # the resolved manifest after the snapshot
    deduplicated: bool

    @classmethod
    def from_proto(cls, r: "_g.SnapshotIntoResponse") -> "SnapshotResult":
        return cls(
            branch_ref=hexids.encode(r.branch_ref),
            handle=r.handle,
            ingested=r.ingested,
            items=[BranchItem.from_proto(it) for it in r.items],
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class AdvanceResult:
    """The outcome of an ``AdvanceBranch`` (D155 Phase-3) — the manifest after the
    in-CAS re-point. ``deduplicated`` is True iff the re-point was a no-op."""

    branch_ref: str  # server-derived manifest hash, as hex (recomputed)
    handle: str  # echoed canonical handle
    items: List[BranchItem]  # the resolved manifest after the advance
    deduplicated: bool

    @classmethod
    def from_proto(cls, r: "_g.AdvanceBranchResponse") -> "AdvanceResult":
        return cls(
            branch_ref=hexids.encode(r.branch_ref),
            handle=r.handle,
            items=[BranchItem.from_proto(it) for it in r.items],
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class BranchVersion:
    """One recorded point-in-time of a branch manifest. Every non-dedup mutation
    (create / snapshot / advance / restore) appends a version; the CAS blobs
    behind a recorded version are never collected by branch ops, so any listed
    version is restorable."""

    version: int  # 1-based per-handle version, monotone (1 = oldest retained)
    branch_ref: str  # server-derived manifest hash at this version, as hex (display only)
    recorded_unix_ms: int  # sidecar wall-clock at record time; advisory
    cause: str  # "baseline" | "create" | "snapshot" | "advance" | "restore"
    item_count: int  # manifest size at this version

    @classmethod
    def from_proto(cls, v: "_g.BranchVersion") -> "BranchVersion":
        return cls(
            version=v.version,
            branch_ref=hexids.encode(v.branch_ref),
            recorded_unix_ms=v.recorded_unix_ms,
            cause=v.cause,
            item_count=v.item_count,
        )


@dataclass(frozen=True)
class RestoreResult:
    """The outcome of a ``RestoreBranch`` — restore APPENDS a new version whose
    items are the historical items (history is never rewound; a restore is itself
    history). ``deduplicated`` is True iff the branch already matched the requested
    version (no-op, nothing recorded, ``new_version`` = 0)."""

    branch: Branch  # the manifest after the restore
    new_version: int  # the version the restore recorded (0 when deduplicated)
    deduplicated: bool

    @classmethod
    def from_proto(cls, r: "_g.RestoreBranchResponse") -> "RestoreResult":
        return cls(
            branch=Branch.from_proto(r.branch),
            new_version=r.new_version,
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class EditProposal:
    """POC-5d: the PROPOSE half of an agentic branch edit — the model's proposed new
    body for a file plus its current body, WITHOUT having advanced the branch. The
    caller reviews the diff (``current_text`` vs ``proposed_text``) then approves via
    ``advance_branch(handle, path, result_ref)`` or discards to reject (the proposed
    blob is a harmless content-addressed orphan)."""

    result_ref: str  # the committed content ref of the proposed body (advance target)
    proposed_text: str  # the model's proposed new file contents
    current_text: str  # the file's current contents (for the diff)
