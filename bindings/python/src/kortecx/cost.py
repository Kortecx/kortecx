"""Cost-spend guardrail readout (M11) — a run's DISPLAY-ONLY local spend estimate
over the durable turn/tool counters at operator-set micro-USD rates, surfaced by
``GetRunCost``.

This is a BUDGET GUARDRAIL readout, NOT Cloud per-expert billing (the D129/D156/GR19
boundary holds — no token / price-per-expert data crosses the wire). Kept in its own
module (module-per-concern, GR3).
"""

from __future__ import annotations

from dataclasses import dataclass

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class RunCost:
    """A run's local spend estimate (micro-USD), with the priced counters + rates."""

    instance_id: str
    turns: int
    tool_calls: int
    estimated_micro_usd: int
    ceiling_micro_usd: int
    per_turn_micro_usd: int
    per_tool_call_micro_usd: int
    over_ceiling: bool

    @classmethod
    def from_proto(cls, c: "_g.GetRunCostResponse") -> "RunCost":
        return cls(
            instance_id=hexids.encode(c.instance_id),
            turns=c.turns,
            tool_calls=c.tool_calls,
            estimated_micro_usd=c.estimated_micro_usd,
            ceiling_micro_usd=c.ceiling_micro_usd,
            per_turn_micro_usd=c.per_turn_micro_usd,
            per_tool_call_micro_usd=c.per_tool_call_micro_usd,
            over_ceiling=c.over_ceiling,
        )
