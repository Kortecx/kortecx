"""The ControlSurface + NL-authoring types — ``DescribeControlSurface`` /
``ProposeControlAction`` and the durable Policy/Role registry.

Kept in its own module (the secrets.py / module-per-concern precedent).

## A proposal writes nothing, and approval is client-held

``ProposeControlAction`` returns the EXACT typed request the runtime would issue and
registers nothing. Enacting it means calling that ordinary RPC with the bytes you were
shown — never re-deriving them from a rendering. That is why :class:`ControlPreview`
carries ``rpc`` (which method to call) alongside the rendered summary.

## A role NARROWS, never grants

Assigning a Policy/Role makes a party's effective tool set the INTERSECTION of every
present authority leg, so it can only ever take capability away. Naming a tool the
party could not fire anyway simply drops out. An EMPTY role is meaningful and is not
the same as having no role: it narrows to nothing.

## What a proposal structurally cannot carry

Secrets ride a NAME-only shape and scripts carry no ``argv``/``env`` — not by
convention but because the wire types have no such field. A preview can therefore be
displayed, logged and forwarded without ever holding a credential.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ControlSurfaceEntry:
    """What ONE gateway RPC is.

    The wire half (``rpc``) is generated from the compiled descriptor; the judgement
    half (``domain`` / ``mutates`` / ``authority``) is hand-authored, because no
    descriptor can know that ``ProposeWorkflow`` writes nothing or that ``CallMcpTool``
    reaches the world despite its name.
    """

    rpc: str
    domain: str
    #: ``False`` means a successful call changes no durable state.
    mutates: bool
    #: ``caller_principal`` | ``operator_global`` | ``loopback_only``.
    authority: str
    #: ``True`` when the domain is one the NL authoring surface covers.
    authoring: bool

    @classmethod
    def from_proto(cls, e: "_g.ControlSurfaceEntry") -> "ControlSurfaceEntry":
        return cls(
            rpc=e.rpc,
            domain=e.domain,
            mutates=e.mutates,
            authority=e.authority,
            authoring=e.authoring,
        )


@dataclass(frozen=True)
class PolicyRoleTool:
    """One ``(tool_id, tool_version)`` pair a role narrows TO."""

    tool_id: str
    tool_version: str


@dataclass(frozen=True)
class PolicyRole:
    """One stored Policy/Role.

    ``tools`` EMPTY is a decision, not an absence: a role that names no tool refuses
    every tool. A party with no role assigned is a different thing entirely — it
    expresses no narrowing and resolves exactly as it did before any registry existed.
    """

    name: str
    description: str
    tools: List[PolicyRoleTool]
    created_unix_ms: int
    updated_unix_ms: int

    @classmethod
    def from_proto(cls, r: "_g.PolicyRole") -> "PolicyRole":
        return cls(
            name=r.name,
            description=r.description,
            tools=[PolicyRoleTool(tool_id=t.tool_id, tool_version=t.tool_version) for t in r.tools],
            created_unix_ms=r.created_unix_ms,
            updated_unix_ms=r.updated_unix_ms,
        )


@dataclass(frozen=True)
class ControlPreview:
    """The exact typed request the runtime WOULD issue.

    ``rpc`` names the method to call to enact it, and ``request`` is the protobuf
    message to send — the SAME message the server put in the preview, not a
    reconstruction. Forwarding it verbatim is the whole point: it is what makes
    "approve" mean "issue what I was shown".
    """

    #: The ``GatewayRpc`` wire name to call (e.g. ``"SaveWorkflow"``).
    rpc: str
    #: One-line human rendering. DISPLAY ONLY — never parse this.
    summary: str
    #: Which ``oneof`` arm is set (e.g. ``"put_policy_role"``).
    request_field: str
    #: The request message itself, ready to forward.
    request: object

    @classmethod
    def from_proto(cls, p: "_g.ControlPreview") -> "ControlPreview":
        field_name = p.WhichOneof("request") or ""
        return cls(
            rpc=p.rpc,
            summary=p.summary,
            request_field=field_name,
            request=getattr(p, field_name) if field_name else None,
        )


@dataclass(frozen=True)
class ControlProposal:
    """The outcome of :meth:`Client.propose_control_action`.

    A refusal is an ANSWER, not an error: ``proposed=False`` with a ``reason``. An
    inadmissible ask should be refused before a human is asked to approve it, and a
    transport error would hide that behind a stack trace.
    """

    proposed: bool
    preview: Optional[ControlPreview] = None
    reason: str = ""

    @classmethod
    def from_proto(cls, resp: "_g.ProposeControlActionResponse") -> "ControlProposal":
        which = resp.WhichOneof("result")
        if which == "preview":
            return cls(proposed=True, preview=ControlPreview.from_proto(resp.preview))
        if which == "rejected":
            return cls(proposed=False, reason=resp.rejected.reason)
        return cls(proposed=False, reason="the gateway returned no result")
