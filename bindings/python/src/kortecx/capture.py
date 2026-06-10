"""Morphic Data Engine capture view — one durably-captured ACTION record
enumerated by ``ListCaptureRecords``.

The serve-path action exhaust: a committed Mote's join keys (``mote_id`` /
``instance_id`` / ``result_ref`` / ``nd_class`` / ``seq``), plus the ReAct
``turn``/``branch`` when the Mote is a ReAct turn. Join-key-only (the
privacy-safe ActionsOnly scope) — no payload/reasoning. Kept in its own module
(the runs.py / module-per-concern precedent). SN-8: ids are server-derived; the
SDK only hex-encodes the bytes.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class CaptureRecord:
    """One captured action: the committed Mote's join keys + (for a ReAct turn)
    the turn index/branch. ``result_ref`` IS the action's content address — the
    join key back to the journal truth."""

    mote_id: str  # hex
    instance_id: str  # hex
    result_ref: str  # hex
    nd_class: str  # "pure" | "read_only_nondet" | "world_mutating"
    seq: int
    react_turn: Optional[int]  # set iff the Mote is a ReAct turn
    react_branch: str  # the turn's settled branch iff a ReAct turn (else "")

    @classmethod
    def from_proto(cls, r: "_g.CaptureRecordSummary") -> "CaptureRecord":
        return cls(
            mote_id=hexids.encode(r.mote_id),
            instance_id=hexids.encode(r.instance_id),
            result_ref=hexids.encode(r.result_ref),
            nd_class=r.nd_class,
            seq=r.seq,
            react_turn=r.react_turn if r.HasField("react_turn") else None,
            react_branch=r.react_branch,
        )


@dataclass(frozen=True)
class CaptureRecordPage:
    """One newest-first page of :class:`CaptureRecord` plus the ``has_more`` flag."""

    records: List[CaptureRecord]
    has_more: bool
