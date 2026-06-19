"""The Blueprint builder — author a Tier-1 DAG (a vetted palette of PURE / MODEL
steps + DATA/CONTROL edges) for ``SubmitWorkflow``.

Kept in its own module (the runs.py / module-per-concern precedent). SN-8: the
builder NEVER computes a MoteId or a warrant — it only assembles the topology +
params the SERVER compiles + admits. The server assigns each step's logic_ref from
its kind and builds every warrant from the party's grants; a tampered client DAG
only changes what is PROPOSED, never what identity it gets.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Sequence, Union

from . import hexids
from .v1 import coordinator_pb2 as _c
from .v1 import gateway_pb2 as _g

_KIND = {
    "pure": _g.WorkflowStepKind.WORKFLOW_STEP_KIND_PURE,
    "model": _g.WorkflowStepKind.WORKFLOW_STEP_KIND_MODEL,
    "exec": _g.WorkflowStepKind.WORKFLOW_STEP_KIND_EXEC,
    "tool": _g.WorkflowStepKind.WORKFLOW_STEP_KIND_TOOL,
}

#: PR-6b-2: the single canonical ``config_subset`` key a ``tool()`` step's authored
#: args ride under (one canonical-JSON object). MUST equal the Rust
#: ``kx_mote::TOOL_ARGS_KEY`` and the TS ``TOOL_ARGS_KEY`` — the coordinator's
#: ``is_authored_tool`` discriminant + args source.
TOOL_ARGS_KEY = "kx.tool.args"

#: PR-9b (D161.1): the canonical ``params`` keys a deterministic-agentic MODEL
#: step's bounded-loop budget rides under (decimal-string bytes ⇒ canonical-JSON
#: ``u32``, the form the coordinator's ``react_seed_params`` reads). MUST equal the
#: Rust ``kx_mote::REACT_MAX_TURNS_KEY`` / ``REACT_MAX_TOOL_CALLS_KEY`` + the TS keys.
REACT_MAX_TURNS_KEY = "max_turns"
REACT_MAX_TOOL_CALLS_KEY = "max_tool_calls"


@dataclass
class StepInput:
    """One authored step. ``params`` values may be ``str`` (UTF-8) or ``bytes``."""

    kind: str  # "pure" | "model" | "exec" (reserved) | "tool" (PR-6b-2)
    model_id: str = ""
    prompt: str = ""
    body_signature_id: Optional[str] = None  # EXEC only: 64-char hex of the body id
    tool_contract: Dict[str, str] = field(default_factory=dict)
    params: Dict[str, Union[bytes, str]] = field(default_factory=dict)
    #: Agentic MODEL step only (PR-9b, D161.1): the bounded reason→tool→observe loop
    #: budget. Injected into ``params`` (canonical-JSON ``u32`` keys) at lowering
    #: when the step is a MODEL step with a non-empty ``tool_contract``; ignored
    #: otherwise. Absent ⇒ the coordinator default (8 turns / 6 tool calls).
    max_turns: Optional[int] = None
    max_tool_calls: Optional[int] = None


@dataclass
class EdgeInput:
    """One authored edge between two steps (by their ``add_step`` index)."""

    parent: int
    child: int
    edge: str = "data"  # "data" (default) | "control"
    non_cascade: bool = False


def _param_bytes(v: "Union[bytes, str]") -> bytes:
    return v.encode("utf-8") if isinstance(v, str) else v


class BlueprintBuilder:
    """A fluent builder for a ``SubmitWorkflowRequest``. ``add_step`` returns the
    step index (the handle used to wire edges)."""

    def __init__(self, seed: int = 0) -> None:
        self._seed = seed
        self._steps: List[StepInput] = []
        self._edges: List[EdgeInput] = []
        self._mode = "frozen"
        self._context_bundles: List[str] = []

    def add_step(self, step: StepInput) -> int:
        self._steps.append(step)
        return len(self._steps) - 1

    def add_edge(self, edge: EdgeInput) -> "BlueprintBuilder":
        self._edges.append(edge)
        return self

    def mode(self, m: str) -> "BlueprintBuilder":
        self._mode = m
        return self

    def context_bundles(self, handles: Sequence[str]) -> "BlueprintBuilder":
        """PR-7: attach context-bundle handles to the run (verbatim order — the
        SERVER canonicalizes + injects into every entry Mote at bind, SN-8). An
        empty list ⇒ a request byte-identical to pre-PR-7."""
        self._context_bundles = list(handles)
        return self

    def build(self) -> "_g.SubmitWorkflowRequest":
        steps = [
            _g.WorkflowStep(
                kind=_KIND[s.kind],
                model_id=s.model_id,
                prompt=s.prompt,
                body_signature_id=(
                    hexids.decode_fixed(s.body_signature_id, 32) if s.body_signature_id else b""
                ),
                tool_contract=dict(s.tool_contract),
                params={k: _param_bytes(v) for k, v in s.params.items()},
            )
            for s in self._steps
        ]
        edges = [
            _g.WorkflowEdge(
                parent=e.parent,
                child=e.child,
                edge_kind=(
                    _c.EdgeKind.EDGE_KIND_CONTROL
                    if e.edge == "control"
                    else _c.EdgeKind.EDGE_KIND_DATA
                ),
                non_cascade=e.non_cascade,
            )
            for e in self._edges
        ]
        mode = (
            _g.WorkflowExecutionMode.WORKFLOW_EXECUTION_MODE_DYNAMIC
            if self._mode == "dynamic"
            else _g.WorkflowExecutionMode.WORKFLOW_EXECUTION_MODE_FROZEN
        )
        return _g.SubmitWorkflowRequest(
            seed=self._seed,
            steps=steps,
            edges=edges,
            execution_mode=mode,
            context_bundles=list(self._context_bundles),
        )
