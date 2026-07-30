"""Hosted Apps — a saved App's project tree served from a confined child process.

Kept in its own module so ``types.py`` stays a thin aggregator, mirroring
``branch.py`` and ``scripts.py``.

The whole surface is feature-gated server-side (``hosted-apps``). A serve built
without it answers every hosted RPC ``UNIMPLEMENTED``, which surfaces here as
``KxUnimplemented`` — a real, testable contract rather than an empty list that
looks like "you have no hosted apps".
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from .v1 import gateway_pb2 as _g

#: Wire enum -> the name used here. Kept explicit rather than derived from the
#: protobuf enum's own names so a renamed wire constant is a visible diff.
_STATE_NAMES = {
    0: "unspecified",
    1: "stopped",
    2: "materializing",
    3: "installing",
    4: "starting",
    5: "running",
    6: "failed",
    7: "building",
}


@dataclass(frozen=True)
class HostedAppStatus:
    """A hosted App's current state.

    ``detail`` and ``recent_logs`` are ADVISORY — they explain, they never carry
    authority. ``url`` is live only while ``state == "running"``.
    """

    handle: str
    state: str  # see _STATE_NAMES
    url: str  # live only while running
    recent_logs: List[str]  # tail of install/build/server logs (advisory)
    framework: str  # "vite_react" | "next_js" | "svelte"
    port: int  # loopback port; 0 when not running
    detail: str  # advisory status / failure text (never authority)
    serve_mode: str  # "dev" | "production"

    @classmethod
    def from_proto(cls, s: "_g.HostedAppStatus") -> "HostedAppStatus":
        # An EMPTY serve_mode means an older serve that did not report one. It is
        # read as "dev", never guessed as "production": telling an operator their
        # app is production-served when it is not is the expensive direction of
        # this mistake. Mirrors the TypeScript SDK's rule exactly.
        return cls(
            handle=s.handle,
            state=_STATE_NAMES.get(s.state, "unspecified"),
            url=s.url,
            recent_logs=list(s.recent_logs),
            framework=s.framework,
            port=s.port,
            detail=s.detail,
            serve_mode=s.serve_mode or "dev",
        )
