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
#[derive(Clone, Debug, Default)]
pub struct ServerInfoFacts {
    /// Resolved serve model id (empty on a model-less serve).
    pub model_id: String,
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
}
