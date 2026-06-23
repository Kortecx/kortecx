"""The agent-runner result — the one-object answer of ``run_agent`` (PR-9c-1).

``run_agent(goal) → AgentResult``: the model's final answer PLUS the audited set of
tool actions it took (each a durable ``ReactRound`` ``tool`` fact, server-derived). A
thin, read-only projection over the steered ``kx/recipes/react`` chain — no new wire
surface, no proto change (assembled client-side from ``ListReactTurns`` +
``GetContent``). SN-8: every id + action is server-derived; the SDK only shapes them.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional

from .critic import decode_critic_verdict
from .react import ReactTurn


@dataclass(frozen=True)
class AuditedAction:
    """One tool action the agent took — a settled ReAct ``tool`` turn. The
    ``tool_id`` / ``tool_version`` are the GRANTED tool's (SN-8), never the model's
    raw proposal. T-MULTI-ELEMENT-TOOLCALLS: when a turn fires N tools at once, each
    is a distinct action sharing ``turn``, ordered by ``call_index`` (0..N-1)."""

    tool_id: str
    tool_version: str
    turn: int
    call_index: int = 0

    @classmethod
    def from_turn(cls, t: ReactTurn) -> "AuditedAction":
        return cls(
            tool_id=t.tool_id,
            tool_version=t.tool_version,
            turn=t.turn,
            call_index=t.call_index,
        )


@dataclass(frozen=True)
class AgentResult:
    """The terminal answer of an agent run + its audited action set + the durable,
    re-attachable run handle (the instance id)."""

    answer: Optional[str]  # the final answer decoded UTF-8 (None if non-text/absent)
    answer_bytes: Optional[bytes]  # the raw committed answer bytes
    actions: List[AuditedAction] = field(default_factory=list)
    run_handle: str = ""  # hex instance id — the durable handle to re-attach to this run
    instance_id: str = ""  # hex instance id (== run_handle)

    @property
    def ok(self) -> bool:
        """True iff the agent produced a committed answer."""
        return self.answer_bytes is not None

    @property
    def verdict(self) -> Optional[str]:
        """T-AGENT2: if this run's terminal is an LLM-judge (``kx/recipes/judge``),
        the decoded ``"valid"`` / ``"invalid: <reason>"`` summary; ``None`` for a
        plain answer. Display-only (SN-8)."""
        if self.answer_bytes is None:
            return None
        return decode_critic_verdict(self.answer_bytes)

    def to_dict(self) -> dict:
        """A JSON-able view (the ``kx agent run --json`` shape)."""
        out: dict = {
            "instance_id": self.instance_id,
            "run_handle": self.run_handle,
            "actions": [
                {
                    "tool_id": a.tool_id,
                    "tool_version": a.tool_version,
                    "turn": a.turn,
                    "call_index": a.call_index,
                }
                for a in self.actions
            ],
        }
        if self.answer is not None:
            out["answer"] = self.answer
        verdict = self.verdict
        if verdict is not None:
            out["verdict"] = verdict
        return out

    def json(self) -> dict:
        """Alias of :meth:`to_dict` (mirrors :meth:`kortecx.run.Result.json` + the TS SDK)."""
        return self.to_dict()


def assemble_actions(turns: List[ReactTurn]) -> List[AuditedAction]:
    """The audited action set = the chain's settled ``tool`` turns, ordered by
    ``(turn, call_index)`` so a multi-tool turn's parallel calls read in emission
    order (T-MULTI-ELEMENT-TOOLCALLS). Pure client-side derivation over the durable
    ``ListReactTurns`` facts."""
    return [
        AuditedAction.from_turn(t)
        for t in sorted(turns, key=lambda t: (t.turn, t.call_index))
        if t.branch == "tool"
    ]
