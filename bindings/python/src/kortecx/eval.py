"""Per-run quality readout (RC1/D172) — an EXPECTATION-FREE summary of a live run's
trajectory (did it reach an answer, turns / tool-calls spent, budget burn, rejection
count), surfaced by ``ScoreRun``.

The golden-suite GATE (task success / tool-call correctness / groundedness vs a known
expectation) runs OFFLINE via the ``kx eval run`` CLI / ``just eval`` — it never crosses
this wire. Kept in its own module (module-per-concern, GR3).
"""

from __future__ import annotations

from dataclasses import dataclass

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class RunScore:
    """A live run's expectation-free quality summary (the ``ScoreRun`` readout)."""

    instance_id: str
    terminal: str
    reached_answer: bool
    turns_used: int
    tool_calls_used: int
    max_turns: int
    max_tool_calls: int
    rejections: int
    turn_budget_used_per_mille: int
    tool_budget_used_per_mille: int

    @classmethod
    def from_proto(cls, s: "_g.RunScore") -> "RunScore":
        return cls(
            instance_id=hexids.encode(s.instance_id),
            terminal=s.terminal,
            reached_answer=s.reached_answer,
            turns_used=s.turns_used,
            tool_calls_used=s.tool_calls_used,
            max_turns=s.max_turns,
            max_tool_calls=s.max_tool_calls,
            rejections=s.rejections,
            turn_budget_used_per_mille=s.turn_budget_used_per_mille,
            tool_budget_used_per_mille=s.tool_budget_used_per_mille,
        )
