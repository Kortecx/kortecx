"""ReAct-chain turn view — one ``ReactRound`` fact enumerated by ``ListReactTurns``.

The durable, queryable history of a live ReAct chain in ``kx serve`` (PR-2d-1/2):
each turn's run-salted Mote id, its settled branch (``pending`` | ``answer`` |
``tool`` | ``rejected`` | ``dead_lettered``), and — for a ``tool`` branch — the
fired tool's ``id@version``, or — for a ``rejected`` branch (PR-3/A2) — the
fail-closed ``rejection_reason`` the model re-prompts over. Kept in its own module
(the runs.py / module-per-concern precedent). SN-8: every id is server-derived;
the SDK only hex-encodes the bytes.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ReactTurn:
    """One ReAct turn fact: hex ids + the frozen branch (and, for a ``tool``
    branch, the fired tool's id/version) + the run's durable budget caps + the
    journal seq (the pagination cursor)."""

    turn: int
    turn_mote_id: str  # hex
    instance_id: str  # hex
    model_id: str
    branch: str  # "pending" | "answer" | "tool" | "rejected" | "dead_lettered"
    tool_id: str  # set iff branch == "tool"
    tool_version: str  # set iff branch == "tool"
    max_turns: int
    max_tool_calls: int
    seq: int
    rejection_reason: str = ""  # set iff branch == "rejected" (PR-3/A2)
    step_salt: str = ""  # PR-R1: the chain key (hex 32B); "" for a legacy run-level chain
    # T-MULTI-ELEMENT-TOOLCALLS: when a model fires N tools in ONE turn, the gateway
    # fans that turn into N "tool" rows sharing ``turn``/``turn_mote_id``/``seq``,
    # distinguished by ``call_index`` (0..N-1, emission order). 0 for a single-call
    # turn and every non-tool branch.
    call_index: int = 0

    @classmethod
    def from_proto(cls, r: "_g.ReactTurnSummary") -> "ReactTurn":
        return cls(
            turn=r.turn,
            turn_mote_id=hexids.encode(r.turn_mote_id),
            instance_id=hexids.encode(r.instance_id),
            model_id=r.model_id,
            branch=r.branch,
            tool_id=r.tool_id,
            tool_version=r.tool_version,
            max_turns=r.max_turns,
            max_tool_calls=r.max_tool_calls,
            seq=r.seq,
            rejection_reason=r.rejection_reason,
            step_salt=hexids.encode(r.step_salt) if r.step_salt else "",
            call_index=r.call_index,
        )


@dataclass(frozen=True)
class ReactTurnPage:
    """One newest-first page of :class:`ReactTurn` plus the ``has_more`` flag."""

    turns: List[ReactTurn]
    has_more: bool
