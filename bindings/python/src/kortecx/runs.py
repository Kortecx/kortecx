"""UI-2 run-summary view — one registered run instance enumerated by ``ListRuns``.

Kept in its own module so ``types.py`` stays a thin aggregator, mirroring the Rust
core's module-per-concern discipline. SN-8: every id is server-derived; the SDK
only hex-encodes the bytes. ``registered_unix_ms`` is an audit-only wall-clock
(off every hash) — a legitimate "started at", never identity.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class RunSummary:
    """One registered run instance: hex ids + the registered seq (pagination
    cursor) + the registered wall-clock (unix-ms; audit-only)."""

    instance_id: str  # hex
    recipe_fingerprint: str  # hex
    registered_seq: int
    registered_unix_ms: int

    @classmethod
    def from_proto(cls, r: "_g.RunSummary") -> "RunSummary":
        return cls(
            instance_id=hexids.encode(r.instance_id),
            recipe_fingerprint=hexids.encode(r.recipe_fingerprint),
            registered_seq=r.registered_seq,
            registered_unix_ms=r.registered_unix_ms,
        )


@dataclass(frozen=True)
class RunPage:
    """One newest-first page of :class:`RunSummary` plus the ``has_more`` flag."""

    runs: List[RunSummary]
    has_more: bool


@dataclass(frozen=True)
class RunInputs:
    """The args a run was submitted with (PR-D ``GetRunInputs``) — the baseline a
    client edits and re-invokes ("Re-run with changes").

    ``args`` is decoded from the opaque JSON object bytes the run was submitted
    with; ``handle`` is what :meth:`KxClient.get_recipe_form` needs to re-render
    the form (a durable run otherwise carries only the fingerprint). SN-8 /
    off-digest: the args never become committed facts. A run with nothing captured
    raises ``KxNotFound``; an old gateway raises ``KxUnimplemented``.
    """

    instance_id: str  # hex
    recipe_fingerprint: str  # hex
    handle: str
    args: Dict[str, Any]

    @classmethod
    def from_proto(cls, r: "_g.GetRunInputsResponse") -> "RunInputs":
        args: Dict[str, Any] = {}
        if r.args:
            try:
                parsed = json.loads(r.args.decode("utf-8"))
                if isinstance(parsed, dict):
                    args = parsed
            except (json.JSONDecodeError, UnicodeDecodeError):
                # A corrupt/non-JSON capture degrades to empty args rather than
                # throwing inside the SDK — never fake, never crash.
                args = {}
        return cls(
            instance_id=hexids.encode(r.instance_id),
            recipe_fingerprint=hexids.encode(r.recipe_fingerprint),
            handle=r.handle,
            args=args,
        )
