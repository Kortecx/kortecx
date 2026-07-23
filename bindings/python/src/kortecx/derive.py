"""App-derive result types (the :meth:`derive_app` return shape).

``derive_app`` turns ONE natural-language prompt into a reviewable App DESIGN — the
workflow, its SHAPE (which steps run in parallel), and the capabilities each step needs.
Validate-only: no envelope is saved, no branch is created, no journal is written. Nothing
exists until the caller approves and authors it through the normal path (:meth:`save_app`
+ :meth:`scaffold_app`), which re-derives every authoritative axis server-side.

What is new versus :meth:`propose_workflow`: a proposed step's ``tool_contract`` has always
come from the vetted role recipe, which is EMPTY for every authoring role — so no proposal
ever carried a tool. Here the model may NAME tool ids, but only from a server-built menu of
what this caller could already fire, and everything it names is intersected back against
that menu host-side. Naming is not granting (SN-8): what survives is a WISH that ``RunApp``
intersects again at fire.
"""

from __future__ import annotations

import dataclasses as _dataclasses
from typing import Dict, List, Tuple

from .v1 import gateway_pb2 as _g


@_dataclasses.dataclass(frozen=True)
class DerivedAppStep:
    """One designed step (display shape). ``role``/``intent`` are the model's design;
    ``kind``/``model_id`` are the server-resolved recipe axes; ``tool_contract`` is what
    SURVIVED the server's intersection against the caller's tool ceiling — the ceiling's
    version, never the model's.

    ``skills`` / ``integrations`` / ``datasets`` / ``apps`` are the per-step BINDINGS: which
    node uses each capability. ``apps`` is the one that adds WORK — each handle lowers that
    App's whole blueprint into the run, feeding its result to this step. They become the blueprint step's own lists, while the app-level lists on
    :class:`AppDerivation` are their UNION — the DECLARATION set you write into the envelope's
    ``references``. Writing only the union authors an App whose every capability falls back to
    the entry step."""

    role: str
    intent: str
    kind: str
    model_id: str
    tool_contract: Dict[str, str]
    skills: List[str]
    integrations: List[str]
    datasets: List[str]
    apps: List[str]


@_dataclasses.dataclass(frozen=True)
class DerivedAppFile:
    """One planned project file — the HOSTED lane's review surface."""

    path: str
    role: str


@_dataclasses.dataclass(frozen=True)
class AppDerivation:
    """The outcome of :meth:`derive_app`: a reviewable design (``derived`` is ``True``),
    or an honest refusal (no served model, an inadmissible workflow — ``derived`` is
    ``False`` and ``reason`` explains).

    ``edges`` is the whole of the shape decision: **a step with no incoming edge runs in
    parallel**, so an empty edge list on a multi-step design means every step runs at once,
    on purpose — it is not an omission.

    ``notices`` carries what the design did NOT get: ids dropped as outside this caller's
    ceiling, a capability menu bounded by the model's one-decode budget, a framework
    substituted. Surface them — a design that quietly asked for a tool it did not receive
    produces an App that quietly cannot do part of its job.
    """

    derived: bool
    name: str
    description: str
    delivers: str
    steps: List[DerivedAppStep]
    edges: List[Tuple[int, int]]
    files: List[DerivedAppFile]
    framework: str
    tools: Dict[str, str]
    skills: List[str]
    connections: List[str]
    datasets: List[str]
    apps: List[str]
    notices: List[str]
    reason: str

    @classmethod
    def _refused(cls, reason: str) -> "AppDerivation":
        return cls(
            derived=False,
            name="",
            description="",
            delivers="",
            steps=[],
            edges=[],
            files=[],
            framework="",
            tools={},
            skills=[],
            connections=[],
            datasets=[],
            apps=[],
            notices=[],
            reason=reason,
        )

    @classmethod
    def from_proto(cls, resp: "_g.DeriveAppResponse") -> "AppDerivation":
        which = resp.WhichOneof("result")
        if which == "app":
            a = resp.app
            return cls(
                derived=True,
                name=a.name,
                description=a.description,
                delivers=a.delivers,
                steps=[
                    DerivedAppStep(
                        role=s.role,
                        intent=s.intent,
                        kind=s.kind,
                        model_id=s.model_id,
                        tool_contract=dict(s.tool_contract),
                        skills=list(s.skills),
                        integrations=list(s.integrations),
                        datasets=list(s.datasets),
                        apps=list(s.apps),
                    )
                    for s in a.steps
                ],
                edges=[(e.parent, e.child) for e in a.edges],
                files=[DerivedAppFile(path=f.path, role=f.role) for f in a.files],
                framework=a.framework,
                tools=dict(a.tools),
                skills=list(a.skills),
                connections=list(a.connections),
                datasets=list(a.datasets),
                apps=list(a.apps),
                notices=list(a.notices),
                reason="",
            )
        if which == "rejected":
            return cls._refused(resp.rejected.reason)
        return cls._refused("the gateway returned no design")
