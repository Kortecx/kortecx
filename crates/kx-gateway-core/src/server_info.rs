//! POC-1 (Settings "Workspace"): the NON-SECRET server-configuration facts the host
//! projects via `GetServerInfo`.
//!
//! A plain value the host fills from its `GatewayConfig` + the resolved serve model +
//! the build's feature flags; the service handler maps it to the wire response, gated
//! on an authenticated caller. By construction it carries **no secret** — there is no
//! field for a bearer-token value or a TLS private key (only `tls_enabled` + an
//! `auth_mode` LABEL) — so the projection cannot leak credentials. The POC-2
//! token-never-leaks negative is therefore a type-level guarantee: there is nothing
//! secret to leak.

/// Non-secret server-configuration facts (the Settings "Workspace" view). The host
/// constructs this once at serve startup; the gateway projects it read-only.
///
/// The boolean fields are independent feature/posture flags (TLS on/off + the build's
/// feature flags), NOT a state machine — a flat projection is the honest shape (it
/// mirrors the wire `GetServerInfoResponse` one-to-one), so the
/// `struct_excessive_bools` refactor suggestion (enums / a state machine) does not apply.
#[derive(Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct ServerInfoFacts {
    /// Resolved serve model id (empty on a model-less serve).
    pub model_id: String,
    /// PR-B: the configured dataset embed model id (`KX_SERVE_EMBED_MODEL` else the
    /// primary). Empty on a model-less serve. Display/Settings only (never identity).
    pub embed_model_id: String,
    /// RC4a (T-RAG-EMBED-QUALITY): `true` iff the configured embedder is a generative
    /// DECODER LLM, not a dedicated embedding model (weak embeddings). Drives the
    /// honest "recommend a dedicated embed model" advisory across CLI/UI/Settings.
    pub embed_model_is_decoder: bool,
    /// Resolved serve model GGUF path (empty on a model-less serve).
    pub model_path: String,
    /// gRPC listener bind address (`addr:port`).
    pub listen_addr: String,
    /// R5 WebSocket live-event bridge bind address.
    pub ws_addr: String,
    /// Embedded console bind address (empty if the console is disabled).
    pub console_addr: String,
    /// Prometheus `/metrics` bind address (empty if metrics are off).
    pub metrics_addr: String,
    /// Content store directory.
    pub content_root: String,
    /// SQLite journal path.
    pub journal_path: String,
    /// Durable catalog directory.
    pub catalog_dir: String,
    /// Worker lease batch size.
    pub max_lease: u64,
    /// Fail-closed `PutContent` payload cap (bytes).
    pub content_max_bytes: u64,
    /// Browser CORS allowlist (display only; empty = deny-by-default).
    pub cors_origins: Vec<String>,
    /// Whether the gRPC listener serves in-binary TLS (POSTURE — never the key bytes).
    pub tls_enabled: bool,
    /// The auth posture LABEL: `deny-all` | `dev-local` | `token` (never a token value).
    pub auth_mode: String,
    /// The datasets / RAG data-plane is available (the `hnsw` feature).
    pub feature_hnsw: bool,
    /// Live model inference is available (the `inference` feature).
    pub feature_inference: bool,
    /// The embedded console is available (the `console` feature).
    pub feature_console: bool,
    /// The resolved serve model is image-capable (a vision projector was wired).
    pub feature_vision: bool,
    /// A JSONL operator audit log is configured.
    pub audit_log_enabled: bool,
    /// T-MULTI-ELEMENT-TOOLCALLS: the server's DEFAULT agentic model-turn budget — the
    /// `max_turns` cap a react/agent run is admitted under when the client omits it
    /// (also the hard ceiling). Read-only; a run overrides it per-invocation via the
    /// `--max-turns` / SDK `max_turns` param.
    pub react_max_turns: u32,
    /// T-MULTI-ELEMENT-TOOLCALLS: the server's DEFAULT total tool-call budget — the
    /// `max_tool_calls` cap (and ceiling). A turn may fire several tools at once, so
    /// this is independent of `react_max_turns`. Read-only; overridable per-invocation
    /// via `--max-tool-calls` / SDK `max_tool_calls`.
    pub react_max_tool_calls: u32,
    /// Model Control v2: whether operator-enabled model downloads are ON
    /// (`KX_SERVE_ALLOW_MODEL_PULL`). `false` ⇒ `PullModel` refuses (deny-by-default);
    /// the UI renders an honest-disabled Pull panel. Posture only (never a URL/secret).
    pub allow_model_pull: bool,
}
