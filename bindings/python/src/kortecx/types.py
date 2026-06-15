"""Idiomatic, read-only views over the generated protobuf messages.

These wrap the raw ``kortecx.v1`` messages with hex ids and stable display names,
mirroring the ``kx`` CLI's rendering (``format.rs``) so the two surfaces agree
field-for-field. Out-of-range enum values render ``UNKNOWN`` (forward-compatible
with a future proto state — never a crash, never a silent mislabel).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional

from . import hexids
from .v1 import coordinator_pb2 as _c
from .v1 import gateway_pb2 as _g

# --- enum display names (mirror kx-cli format.rs::state_name) -----------------

_STATE_NAMES: "dict[int, str]" = {
    _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_PENDING: "PENDING",
    _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_SCHEDULED: "SCHEDULED",
    _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_COMMITTED: "COMMITTED",
    _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_FAILED: "FAILED",
    _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_REPUDIATED: "REPUDIATED",
    _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_INCONSISTENT: "INCONSISTENT",
}


def state_name(state: int) -> str:
    """Map a ``MoteSnapshotState`` discriminant to a stable name (``UNKNOWN`` if new)."""
    return _STATE_NAMES.get(state, "UNKNOWN")


def is_committed(state: int) -> bool:
    return state == _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_COMMITTED


def is_pending(state: int) -> bool:
    """True for a not-yet-terminal state (keep polling)."""
    return state in (
        _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_PENDING,
        _g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_SCHEDULED,
    )


_EDGE_KIND_NAMES: "dict[int, str]" = {
    _c.EdgeKind.EDGE_KIND_DATA: "data",
    _c.EdgeKind.EDGE_KIND_CONTROL: "control",
}


def edge_kind_name(edge_kind: int) -> str:
    """Map an ``EdgeKind`` discriminant to a stable name (``unknown`` if new) —
    mirrors ``kx-cli format.rs::edge_kind_name`` + the TS ``edgeKindName``."""
    return _EDGE_KIND_NAMES.get(edge_kind, "unknown")


# --- Mote / projection views --------------------------------------------------


@dataclass(frozen=True)
class ParentEdge:
    """One incoming DAG edge of a Mote (mirrors ``coordinator.proto`` ``ParentRef``).

    Surfaces the run's topology that the gateway already serves — the upstream
    Mote a child depends on, the edge kind, and (for CONTROL edges) whether it is
    non-cascading. Display-only projection facts (SN-8): a parent edge is
    server-derived, never a client-supplied identity input.
    """

    parent_id: str  # hex (32B parent MoteId)
    edge_kind: str  # stable name: "data" | "control" | "unknown" (parity with CLI/TS)
    non_cascade: bool  # only meaningful for CONTROL edges

    @classmethod
    def from_proto(cls, p: "_c.ParentRef") -> "ParentEdge":
        return cls(
            parent_id=hexids.encode(p.parent_id),
            edge_kind=edge_kind_name(p.edge_kind),
            non_cascade=p.non_cascade,
        )

    def to_dict(self) -> dict:
        return {
            "parent_id": self.parent_id,
            "edge_kind": self.edge_kind,
            "non_cascade": self.non_cascade,
        }


@dataclass(frozen=True)
class MoteView:
    """One Mote in a run's projection, with hex ids + a display state."""

    mote_id: str  # hex
    state: str  # display name
    state_code: int
    nd_class: int
    promotion: int
    result_ref: Optional[str]  # hex when committed
    mote_def_hash: str  # hex
    committed_seq: Optional[int]
    anomaly: Optional[int]
    # The Mote's incoming DAG edges (T-XSURF-1: the gateway serves these; the
    # field defaults to empty so callers constructing a MoteView directly stay
    # forward-compatible).
    parents: List[ParentEdge] = field(default_factory=list)

    @classmethod
    def from_proto(cls, m: "_g.MoteSnapshot") -> "MoteView":
        return cls(
            mote_id=hexids.encode(m.mote_id),
            state=state_name(m.state),
            state_code=m.state,
            nd_class=m.nd_class,
            promotion=m.promotion,
            result_ref=hexids.encode(m.result_ref) if m.HasField("result_ref") else None,
            mote_def_hash=hexids.encode(m.mote_def_hash),
            committed_seq=m.committed_seq if m.HasField("committed_seq") else None,
            anomaly=m.anomaly if m.HasField("anomaly") else None,
            parents=[ParentEdge.from_proto(p) for p in m.parents],
        )

    def to_dict(self) -> dict:
        """The CLI ``--json`` mote shape (ints for nd_class/promotion/anomaly)."""
        return {
            "mote_id": self.mote_id,
            "state": self.state,
            "nd_class": self.nd_class,
            "promotion": self.promotion,
            "result_ref": self.result_ref,
            "committed_seq": self.committed_seq,
            "anomaly": self.anomaly,
            "parents": [p.to_dict() for p in self.parents],
        }


@dataclass(frozen=True)
class Projection:
    """A run rendered as a DAG of Mote states (a fold frontier snapshot)."""

    instance_id: str  # hex
    recipe_fingerprint: str  # hex
    current_seq: int
    motes: List[MoteView]

    @classmethod
    def from_proto(cls, view: "_g.ProjectionView") -> "Projection":
        return cls(
            instance_id=hexids.encode(view.instance_id),
            recipe_fingerprint=hexids.encode(view.recipe_fingerprint),
            current_seq=view.current_seq,
            motes=[MoteView.from_proto(m) for m in view.motes],
        )

    def mote(self, mote_id: str) -> Optional[MoteView]:
        """Find a Mote by its hex id (``None`` if absent at this frontier)."""
        return next((m for m in self.motes if m.mote_id == mote_id), None)

    @property
    def committed(self) -> List[MoteView]:
        return [m for m in self.motes if is_committed(m.state_code)]

    def to_dict(self) -> dict:
        """The CLI ``--json`` projection shape (for parity / scripting)."""
        return {
            "instance_id": self.instance_id,
            "recipe_fingerprint": self.recipe_fingerprint,
            "current_seq": self.current_seq,
            "motes": [m.to_dict() for m in self.motes],
        }


# --- event deltas -------------------------------------------------------------


@dataclass(frozen=True)
class Delta:
    """One event delta (committed / failed / repudiated / effect_staged).

    ``kind`` is the stable lowercase discriminant. Fields not relevant to the
    kind are ``None``. Mirrors the CLI ``render_delta`` / WS-bridge JSON shape.
    """

    seq: int
    kind: str
    mote_id: Optional[str] = None
    result_ref: Optional[str] = None
    nd_class: Optional[int] = None
    reason_class: Optional[int] = None
    target_mote_id: Optional[str] = None
    target_committed_seq: Optional[int] = None

    @classmethod
    def from_proto(cls, d: "_g.EventDelta") -> "Optional[Delta]":
        """Build a view, or ``None`` for a delta with no recognized kind (skip)."""
        which = d.WhichOneof("kind")
        if which == "committed":
            c = d.committed
            return cls(
                seq=d.seq,
                kind="committed",
                mote_id=hexids.encode(c.mote_id),
                result_ref=hexids.encode(c.result_ref),
                nd_class=c.nd_class,
            )
        if which == "failed":
            f = d.failed
            return cls(
                seq=d.seq,
                kind="failed",
                mote_id=hexids.encode(f.mote_id),
                reason_class=f.reason_class,
            )
        if which == "repudiated":
            r = d.repudiated
            return cls(
                seq=d.seq,
                kind="repudiated",
                target_mote_id=hexids.encode(r.target_mote_id),
                target_committed_seq=r.target_committed_seq,
            )
        if which == "effect_staged":
            e = d.effect_staged
            return cls(seq=d.seq, kind="effect_staged", mote_id=hexids.encode(e.mote_id))
        return None

    def to_dict(self) -> dict:
        out: dict = {"seq": self.seq, "kind": self.kind}
        for key in (
            "mote_id",
            "result_ref",
            "nd_class",
            "reason_class",
            "target_mote_id",
            "target_committed_seq",
        ):
            val = getattr(self, key)
            if val is not None:
                out[key] = val
        return out


@dataclass(frozen=True)
class Frame:
    """One ``EventFrame``: a batch of deltas + the resume cursor + caught-up flag."""

    seq: int
    deltas: List[Delta]
    next_seq: int
    journal_boundary: bool

    @classmethod
    def from_proto(cls, f: "_g.EventFrame") -> "Frame":
        deltas = [d for d in (Delta.from_proto(x) for x in f.deltas) if d is not None]
        return cls(
            seq=f.seq, deltas=deltas, next_seq=f.next_seq, journal_boundary=f.journal_boundary
        )


@dataclass(frozen=True)
class TokenChunk:
    """One ADVISORY live token chunk (PR-4.2 / T-STREAM1).

    ``text`` is the NEW model bytes for one decode step, UTF-8 decoded;
    concatenating ``text`` across a stream in ``seq`` order rebuilds the
    completion — byte-identical to the committed ``result_ref`` (the durable
    authority). The stream is out-of-band + display-only, never an authority
    input. ``done`` marks the terminal chunk. ``raw`` keeps the exact piece bytes
    (the gRPC path) for byte-faithful concatenation; it is empty on the WS path.
    """

    seq: int
    mote_id: str
    text: str
    done: bool
    raw: bytes = b""

    @classmethod
    def from_proto(cls, c: "_g.TokenChunk") -> "TokenChunk":
        return cls(
            seq=c.seq,
            mote_id=hexids.encode(c.mote_id),
            text=c.text_piece.decode("utf-8", errors="replace"),
            done=c.done,
            raw=bytes(c.text_piece),
        )

    @classmethod
    def from_ws(cls, obj: dict) -> "TokenChunk":
        """Build from a WS JSON chunk (``text_piece`` already a lossy-UTF-8 string)."""
        return cls(
            seq=int(obj.get("seq", 0)),
            mote_id=str(obj.get("mote_id", "")),
            text=str(obj.get("text_piece", "")),
            done=bool(obj.get("done", False)),
        )


@dataclass(frozen=True)
class GlobalDelta:
    """One cross-run event delta from the Batch C global tail (``StreamAllEvents``).

    ``kind`` is the stable lowercase discriminant — the four per-run kinds plus
    ``run_registered`` (a run came into existence; the per-run cursor never
    surfaces it). ``instance_id`` is the registration-WATERMARK attribution
    (display/observability only, never identity) — ``""`` before any run
    registers. Fields not relevant to the kind are ``None``. Mirrors the WS
    ``/v1/events/all`` JSON shape.
    """

    seq: int
    kind: str
    instance_id: str = ""  # hex; "" before any registration (watermark attribution)
    mote_id: Optional[str] = None
    result_ref: Optional[str] = None
    nd_class: Optional[int] = None
    reason_class: Optional[int] = None
    target_mote_id: Optional[str] = None
    target_committed_seq: Optional[int] = None
    recipe_fingerprint: Optional[str] = None
    registered_unix_ms: Optional[int] = None

    @classmethod
    def from_proto(cls, d: "_g.GlobalEventDelta") -> "GlobalDelta":
        """Build a view; a delta with no recognized kind becomes ``"unknown"``
        (never ``None``, never a throw — the global-tail contract: a future
        delta kind SURFACES on every SDK, exactly like the TS/CLI surfaces,
        deliberately diverging from the per-run ``Delta`` skip)."""
        which = d.WhichOneof("kind")
        inst = hexids.encode(d.instance_id)
        if which == "committed":
            c = d.committed
            return cls(
                seq=d.seq,
                kind="committed",
                instance_id=inst,
                mote_id=hexids.encode(c.mote_id),
                result_ref=hexids.encode(c.result_ref),
                nd_class=c.nd_class,
            )
        if which == "failed":
            f = d.failed
            return cls(
                seq=d.seq,
                kind="failed",
                instance_id=inst,
                mote_id=hexids.encode(f.mote_id),
                reason_class=f.reason_class,
            )
        if which == "repudiated":
            r = d.repudiated
            return cls(
                seq=d.seq,
                kind="repudiated",
                instance_id=inst,
                target_mote_id=hexids.encode(r.target_mote_id),
                target_committed_seq=r.target_committed_seq,
            )
        if which == "effect_staged":
            e = d.effect_staged
            return cls(
                seq=d.seq, kind="effect_staged", instance_id=inst, mote_id=hexids.encode(e.mote_id)
            )
        if which == "run_registered":
            rr = d.run_registered
            return cls(
                seq=d.seq,
                kind="run_registered",
                instance_id=inst,
                recipe_fingerprint=hexids.encode(rr.recipe_fingerprint),
                registered_unix_ms=rr.registered_unix_ms,
            )
        return cls(seq=d.seq, kind="unknown", instance_id=inst)

    def to_dict(self) -> dict:
        out: dict = {"seq": self.seq, "kind": self.kind, "instance_id": self.instance_id}
        for key in (
            "mote_id",
            "result_ref",
            "nd_class",
            "reason_class",
            "target_mote_id",
            "target_committed_seq",
            "recipe_fingerprint",
            "registered_unix_ms",
        ):
            val = getattr(self, key)
            if val is not None:
                out[key] = val
        return out


@dataclass(frozen=True)
class SignatureSummary:
    """One registered task signature (id + name)."""

    signature_id: str  # hex
    name: str

    @classmethod
    def from_proto(cls, s: "_g.SignatureSummary") -> "SignatureSummary":
        return cls(signature_id=hexids.encode(s.signature_id), name=s.name)
