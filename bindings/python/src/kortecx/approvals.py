"""HITL pre-action approval admin (D114) — the operator control plane over pending
world-mutating action approvals, surfaced by ``ListPendingApprovals`` /
``GrantApproval`` / ``DenyApproval``.

Kept in its own module (the feedback.py / module-per-concern precedent, GR3). The
``request_id`` / ``instance_id`` / ``mote_id`` are server-derived (the SDK only
hex-encodes the bytes); grant/deny are OPERATOR decisions over the server-derived
``request_id`` — they release/reject a STAGED action, never mint a client warrant
(SN-8).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class PendingApproval:
    """One world-mutating action withheld awaiting an operator decision. Display-only
    — it carries NO authority; the grant/deny is keyed by ``request_id``."""

    request_id: str
    instance_id: str
    mote_id: str
    tool_id: str
    tool_version: str
    intent: str
    deadline_unix_ms: int
    created_unix_ms: int

    @classmethod
    def from_proto(cls, a: "_g.PendingApproval") -> "PendingApproval":
        return cls(
            request_id=hexids.encode(a.request_id),
            instance_id=hexids.encode(a.instance_id),
            mote_id=hexids.encode(a.mote_id),
            tool_id=a.tool_id,
            tool_version=a.tool_version,
            intent=a.intent,
            deadline_unix_ms=a.deadline_unix_ms,
            created_unix_ms=a.created_unix_ms,
        )


@dataclass(frozen=True)
class PendingApprovalsPage:
    """A page of pending approvals."""

    approvals: List[PendingApproval]
