"""The script registry — sandboxed code an agent may call as a tool.

Kept in its own module so ``types.py`` stays a thin aggregator, mirroring
``branch.py`` and ``apps.py``.

Three properties are worth knowing before you register anything, because they
shape what these types can and cannot carry:

**Registration grants nothing.** ``fs_mounts`` and ``net_hosts`` DECLARE what the
script says it needs. That declaration becomes the tool's requirement, and the
broker still refuses any dispatch whose requirement is not a subset of the
granting warrant. Declaring ``rw:/`` does not grant it; it guarantees the script
is refused everywhere that authority is absent.

**argv and the environment are fixed at registration.** A model calling a script
controls exactly one thing: the ``input`` string on stdin. The child's
environment is CLEARED before the registered pairs are set. That asymmetry is
deliberate — argv and env are where a shell script is easiest to subvert.

**``script_id`` is server-derived.** The client never names an id and never
supplies a warrant; there is no field in which it could.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class RegisteredScript:
    """One row of the durable script registry (the governance / inventory view).

    ``source_ref`` names the EXACT bytes that run: changing the source, the argv,
    the environment or the declared scopes is a different record, a different
    ref, and therefore a different registration.
    """

    script_id: str  # 16-byte server-derived id, as 32 hex chars
    script_name: str
    script_version: str
    interpreter: str
    description: str  # advisory; NEVER parsed for enforcement
    source_ref: str  # content ref of the exact bytes that run
    fs_scope: str  # display: "none" | "ro:/a,rw:/b"
    net_scope: str  # display: "none" | "egress:host[,host]"
    wall_clock_ms: int
    max_output_bytes: int  # exceeding it REFUSES the call, never truncates

    @classmethod
    def from_proto(cls, s: "_g.RegisteredScript") -> "RegisteredScript":
        return cls(
            script_id=hexids.encode(s.script_id),
            script_name=s.script_name,
            script_version=s.script_version,
            interpreter=s.interpreter,
            description=s.description,
            source_ref=s.source_ref_hex,
            fs_scope=s.fs_scope_summary,
            net_scope=s.net_scope_summary,
            wall_clock_ms=s.wall_clock_ms,
            max_output_bytes=s.max_output_bytes,
        )


@dataclass(frozen=True)
class RegisteredScriptsPage:
    """One page of the registry, in ``(name, version)`` order."""

    scripts: List[RegisteredScript]
    has_more: bool

    @classmethod
    def from_proto(cls, r: "_g.ListScriptsResponse") -> "RegisteredScriptsPage":
        return cls(
            scripts=[RegisteredScript.from_proto(s) for s in r.scripts],
            has_more=r.has_more,
        )


@dataclass(frozen=True)
class ScriptWithSource:
    """A script plus the registered source bytes it runs."""

    script: Optional[RegisteredScript]
    source: bytes

    @classmethod
    def from_proto(cls, r: "_g.GetScriptResponse") -> "ScriptWithSource":
        return cls(
            script=RegisteredScript.from_proto(r.script) if r.HasField("script") else None,
            source=bytes(r.source),
        )
