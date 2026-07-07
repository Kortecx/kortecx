"""POC-1 Settings view — the connected gateway's effective configuration.

A single ``GetServerInfo`` projection of the running ``kx serve`` process: its
served model, listen/bridge/console/metrics addresses, content + journal + catalog
locations, the admission caps, the CORS allow-list, and the compiled-in feature
flags. DISPLAY/SETTINGS-ONLY (SN-8): every field is server-derived; it NEVER
carries a secret (no token, no key material) and authorizes nothing.

Kept in its own module so ``types.py`` stays a thin aggregator, mirroring the Rust
core's module-per-concern discipline (and the ``models``/``datasets`` views here).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Tuple

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ServerInfo:
    """The connected gateway's effective configuration (``GetServerInfo``).

    Server-derived display fields only (SN-8) — never a secret, never an identity
    or authorization input. An old gateway without the RPC raises
    ``KxUnimplemented``."""

    model_id: str  # the served model id ("" when FFI-free / none loaded)
    model_path: str  # the served model file path (display only)
    listen_addr: str  # the gRPC listen address
    ws_addr: str  # the optional R5 WebSocket bridge address ("" when off)
    console_addr: str  # the embedded console address ("" when off)
    metrics_addr: str  # the Prometheus /metrics address ("" when off)
    content_root: str  # the content-store root path
    journal_path: str  # the durable journal path
    catalog_dir: str  # the recipe-catalog directory
    max_lease: int  # the per-lease cap (admission)
    content_max_bytes: int  # the PutContent payload cap (bytes)
    cors_origins: Tuple[str, ...]  # the CORS allow-list (deny-by-default when empty)
    tls_enabled: bool  # serving over TLS
    auth_mode: str  # the auth posture (e.g. "token" / "open")
    feature_hnsw: bool  # compiled with the `hnsw` (RAG vector index) feature
    feature_inference: bool  # compiled with the `inference` (server-embed) feature
    feature_console: bool  # compiled with the embedded `console`
    feature_vision: bool  # compiled with the `vision` (mmproj) leg
    audit_log_enabled: bool  # the serve-path JSONL audit log is on
    # T-MULTI-ELEMENT-TOOLCALLS: the server's DEFAULT agentic budget (also the hard
    # ceilings) — a turn may fire several tools, so the two caps are independent. A run
    # overrides them per-invocation via ``run_agent(max_tool_calls=...)`` / ``kx agent
    # run --max-tool-calls``.
    react_max_turns: int = 0
    react_max_tool_calls: int = 0
    # PR-B: the configured datasets/RAG embed model id ("" on a model-less serve).
    embed_model_id: str = ""
    # RC4a: the configured embedder is a decoder LLM (weak sentence embeddings) — the
    # CLI/UI surface an honest "recommend a dedicated embed model" advisory.
    embed_model_is_decoder: bool = False
    # Model Control v2: the active default model ("" ⇒ the primary) + the download
    # posture (operator opt-in; the UI renders an honest-disabled Pull panel when off).
    active_model_id: str = ""
    allow_model_pull: bool = False
    # The resolved embedded-worker POOL size (``--workers`` / ``KX_WORKERS`` /
    # ``KX_SERVE_WORKER_POOL``). ``1`` = the historical single worker; ``>1`` runs
    # Pure/IO/tool Motes concurrently. ``0`` from an old server ⇒ treat as ``1``
    # (see :meth:`effective_worker_pool`). Display/Settings only.
    worker_pool: int = 0

    @property
    def effective_worker_pool(self) -> int:
        """The pool size to display: ``max(1, worker_pool)`` (an old server sends 0)."""
        return max(1, self.worker_pool)

    @classmethod
    def from_proto(cls, r: "_g.GetServerInfoResponse") -> "ServerInfo":
        return cls(
            model_id=r.model_id,
            model_path=r.model_path,
            listen_addr=r.listen_addr,
            ws_addr=r.ws_addr,
            console_addr=r.console_addr,
            metrics_addr=r.metrics_addr,
            content_root=r.content_root,
            journal_path=r.journal_path,
            catalog_dir=r.catalog_dir,
            max_lease=r.max_lease,
            content_max_bytes=r.content_max_bytes,
            cors_origins=tuple(r.cors_origins),
            tls_enabled=r.tls_enabled,
            auth_mode=r.auth_mode,
            feature_hnsw=r.feature_hnsw,
            feature_inference=r.feature_inference,
            feature_console=r.feature_console,
            feature_vision=r.feature_vision,
            audit_log_enabled=r.audit_log_enabled,
            react_max_turns=r.react_max_turns,
            react_max_tool_calls=r.react_max_tool_calls,
            embed_model_id=r.embed_model_id,
            embed_model_is_decoder=r.embed_model_is_decoder,
            active_model_id=r.active_model_id,
            allow_model_pull=r.allow_model_pull,
            worker_pool=r.worker_pool,
        )
