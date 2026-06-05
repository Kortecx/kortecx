"""Idiomatic, read-only views over the generated protobuf messages.

These wrap the raw ``kortecx.v1`` messages with hex ids and stable display names,
mirroring the ``kx`` CLI's rendering (``format.rs``) so the two surfaces agree
field-for-field. Out-of-range enum values render ``UNKNOWN`` (forward-compatible
with a future proto state — never a crash, never a silent mislabel).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from . import hexids
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


# --- Mote / projection views --------------------------------------------------


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
class SignatureSummary:
    """One registered task signature (id + name)."""

    signature_id: str  # hex
    name: str

    @classmethod
    def from_proto(cls, s: "_g.SignatureSummary") -> "SignatureSummary":
        return cls(signature_id=hexids.encode(s.signature_id), name=s.name)
