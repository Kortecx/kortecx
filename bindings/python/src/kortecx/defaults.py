"""Zero-config client resolution (Batch V2).

``import kortecx as kx; kx.run(...)`` uses a lazily-built, process-wide default client
so the simplest path needs no constructor. Config order (first wins):

1. explicit kwargs,
2. environment — ``KX_ENDPOINT`` / ``KX_TOKEN`` / ``KX_DEFAULT_MODEL``,
3. ``~/.kortecx/config.toml`` (a ``[client]`` table),
4. the conventional defaults (loopback endpoint, no token, server-bound model).

The explicit :class:`~kortecx.client.KxClient` / :class:`~kortecx.client.AsyncKxClient`
stay available for full control; this module is the convenience layer on top.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Optional, Union

from .client import DEFAULT_ENDPOINT, KxClient

#: What the gateway accepts as recipe args (mirrors the client's ``ArgsType``).
ArgsType = Union[dict, bytes, bytearray, str]

#: The process-wide default client (lazily built on first use).
_DEFAULT_CLIENT: Optional[KxClient] = None
_CONFIG_PATH = Path.home() / ".kortecx" / "config.toml"


def _load_config() -> dict:
    """Best-effort read of the ``[client]`` table from ``~/.kortecx/config.toml``.
    A missing file / no ``tomllib`` (Python < 3.11) / a parse error ⇒ ``{}`` (env +
    defaults still apply). Never raises."""
    try:
        import tomllib  # type: ignore[import-not-found]  # Python 3.11+
    except ModuleNotFoundError:
        return {}
    try:
        with _CONFIG_PATH.open("rb") as fh:
            data = tomllib.load(fh)
    except (OSError, ValueError):
        return {}
    section = data.get("client")
    return section if isinstance(section, dict) else {}


def resolve_endpoint(explicit: Optional[str] = None) -> str:
    """The gateway endpoint: explicit → ``KX_ENDPOINT`` → config → the loopback default."""
    cfg = _load_config()
    return explicit or os.environ.get("KX_ENDPOINT") or cfg.get("endpoint") or DEFAULT_ENDPOINT


def resolve_default_model(explicit: str = "") -> str:
    """The default model id: explicit → ``KX_DEFAULT_MODEL`` → config → ``""`` (the
    server binds the served model, SN-8)."""
    cfg = _load_config()
    return explicit or os.environ.get("KX_DEFAULT_MODEL") or cfg.get("default_model") or ""


def make_client(
    endpoint: Optional[str] = None,
    *,
    token: Optional[str] = None,
    default_model: str = "",
) -> KxClient:
    """Build a :class:`~kortecx.client.KxClient` from explicit args + env +
    ``~/.kortecx/config.toml`` + the defaults."""
    cfg = _load_config()
    tok = token or os.environ.get("KX_TOKEN") or cfg.get("token")
    return KxClient(
        resolve_endpoint(endpoint),
        token=tok,
        default_model=resolve_default_model(default_model),
    )


def default_client() -> KxClient:
    """The lazily-built, process-wide default client used by the module-level
    ``kx.run`` / ``kx.invoke`` and the :class:`~kortecx.flow.Flow` /
    :class:`~kortecx.agent.Agent` terminals.

    NOTE (async / threads): this is a SINGLE shared client (one gRPC channel) — fine
    for scripts and sync use; for concurrent or async work, construct explicit
    :class:`~kortecx.client.KxClient` / :class:`~kortecx.client.AsyncKxClient`
    instances rather than relying on this singleton."""
    global _DEFAULT_CLIENT
    if _DEFAULT_CLIENT is None:
        _DEFAULT_CLIENT = make_client()
    return _DEFAULT_CLIENT


def set_default_client(client: Optional[KxClient]) -> None:
    """Override (or clear, with ``None``) the process-wide default client — for tests
    or a custom endpoint/token without threading a client through every call."""
    global _DEFAULT_CLIENT
    _DEFAULT_CLIENT = client


def run(target: object, *, wait: bool = True, timeout: float = 120.0, **agent_kwargs):
    """Module-level convenience — run a :class:`~kortecx.flow.Flow`, a
    :class:`~kortecx.chains.Chain`, or a bare prompt (a one-line agent) via the default
    client. ``agent_kwargs`` (``tools=`` / ``reasoning=`` / …) apply only to the prompt
    form."""
    from .chains import Chain
    from .flow import Flow
    from .flow import flow as _flow

    kx = default_client()
    if isinstance(target, Flow):
        return target.run(wait=wait, timeout=timeout, client=kx)
    if isinstance(target, Chain):
        return kx.run_chain(target, wait=wait, timeout=timeout)
    if isinstance(target, str):
        return _flow().agent(target, **agent_kwargs).run(wait=wait, timeout=timeout, client=kx)
    raise TypeError(f"kx.run() accepts a Flow, Chain, or prompt str, got {type(target).__name__}")


def invoke(handle: str, args: "ArgsType", **kwargs):
    """Module-level convenience — invoke a published recipe via the default client."""
    return default_client().invoke(handle, args, **kwargs)
