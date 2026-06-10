"""Re-plan-round view — one ``ReplanRound`` fact enumerated by ``ListReplanRounds``.

The durable, queryable history of a run's model-driven re-plan loop in ``kx serve``
(PR-2c-2): each round's shaper Mote id, the resolved model, the failed steps that
triggered it, and whether the model escalated to a human (the run quiesces). Kept
in its own module (the runs.py / module-per-concern precedent). SN-8: ids are
server-derived; the SDK only hex-encodes the bytes.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ReplanRound:
    """One re-plan round fact: the round index (0 = the initial-plan anchor), the
    shaper Mote id, the resolved model, the failed steps that triggered it, the
    escalation flag, and the journal seq (the pagination cursor)."""

    round: int
    shaper_mote_id: str  # hex
    model_id: str
    failed_step_ids: List[str]  # hex each
    escalated: bool
    seq: int

    @classmethod
    def from_proto(cls, r: "_g.ReplanRoundSummary") -> "ReplanRound":
        return cls(
            round=r.round,
            shaper_mote_id=hexids.encode(r.shaper_mote_id),
            model_id=r.model_id,
            failed_step_ids=[hexids.encode(s) for s in r.failed_step_ids],
            escalated=r.escalated,
            seq=r.seq,
        )


@dataclass(frozen=True)
class ReplanRoundPage:
    """One newest-first page of :class:`ReplanRound` plus the ``has_more`` flag."""

    rounds: List[ReplanRound]
    has_more: bool
