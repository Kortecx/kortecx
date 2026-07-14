"""NL workflow-proposal result types (the :meth:`propose_workflow` return shape).

``propose_workflow`` turns a natural-language goal into a PROPOSED multi-step DAG
(propose-then-confirm, D209.3 / SN-8): the served model plans, the gateway decodes +
compiles the plan through the vetted planner, and returns it for the caller to preview
and confirm. Validate-only — nothing runs until the caller authors the returned steps.
"""

from __future__ import annotations

import dataclasses as _dataclasses
from typing import Dict, List, Tuple

from .v1 import gateway_pb2 as _g


@_dataclasses.dataclass(frozen=True)
class ProposedWorkflowStep:
    """One step of an NL-proposed workflow (display shape). ``role``/``intent``/``kind``
    are the model's plan; ``model_id``/``tool_contract`` are the server-resolved recipe
    axes (the authoritative axes are re-derived server-side at author/run — SN-8)."""

    role: str
    intent: str
    kind: str
    model_id: str
    tool_contract: Dict[str, str]


@_dataclasses.dataclass(frozen=True)
class WorkflowProposal:
    """The outcome of :meth:`propose_workflow`: a compiled multi-step proposal to preview
    + confirm (``proposed`` is ``True``), or an honest rejection (no served model, an
    inadmissible plan — ``proposed`` is ``False``, ``reason`` explains)."""

    proposed: bool
    steps: List[ProposedWorkflowStep]
    edges: List[Tuple[int, int]]
    reason: str

    @classmethod
    def from_proto(cls, resp: "_g.ProposeWorkflowResponse") -> "WorkflowProposal":
        which = resp.WhichOneof("result")
        if which == "plan":
            steps = [
                ProposedWorkflowStep(
                    role=s.role,
                    intent=s.intent,
                    kind=s.kind,
                    model_id=s.model_id,
                    tool_contract=dict(s.tool_contract),
                )
                for s in resp.plan.steps
            ]
            edges = [(e.parent, e.child) for e in resp.plan.edges]
            return cls(proposed=True, steps=steps, edges=edges, reason="")
        if which == "rejected":
            return cls(proposed=False, steps=[], edges=[], reason=resp.rejected.reason)
        return cls(proposed=False, steps=[], edges=[], reason="the gateway returned no proposal")
