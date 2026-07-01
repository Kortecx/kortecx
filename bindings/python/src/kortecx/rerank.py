"""Re-rank-turn view — one ``ReRankRound`` fact enumerated by ``ListReRankTurns``.

The durable, queryable history of a live listwise LLM re-rank loop in ``kx serve``
(RC4c-2): each turn's run-salted re-rank Mote id, the resolved model, the settled
``outcome`` (``pending`` | ``reranked`` | ``failed_closed``), the candidate count,
and — for a ``reranked`` outcome — the exact ``permutation`` (reordered source
indices) the runtime enforced. Kept in its own module (the react.py / replan.py
module-per-concern precedent). SN-8: every id is server-derived; the SDK only
hex-encodes the bytes, and the permutation is an exact reordering, never a score.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ReRankTurn:
    """One re-rank turn fact: the round index, the run-salted re-rank Mote id, the
    resolved model, the settled outcome, the candidate count, the enforced
    permutation (set iff outcome == ``reranked``), and the journal seq (the
    pagination cursor)."""

    round: int
    rerank_mote_id: str  # hex
    instance_id: str  # hex
    model_id: str
    outcome: str  # "pending" | "reranked" | "failed_closed"
    candidate_count: int
    permutation: List[int]  # reordered source indices; set iff outcome == "reranked"
    seq: int

    @classmethod
    def from_proto(cls, r: "_g.ReRankTurnSummary") -> "ReRankTurn":
        return cls(
            round=r.round,
            rerank_mote_id=hexids.encode(r.rerank_mote_id),
            instance_id=hexids.encode(r.instance_id),
            model_id=r.model_id,
            outcome=r.outcome,
            candidate_count=r.candidate_count,
            permutation=list(r.permutation),
            seq=r.seq,
        )


@dataclass(frozen=True)
class ReRankTurnPage:
    """One newest-first page of :class:`ReRankTurn` plus the ``has_more`` flag."""

    turns: List[ReRankTurn]
    has_more: bool
