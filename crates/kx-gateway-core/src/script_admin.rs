//! The script-registry admin seam — `RegisterScript` / `DeregisterScript` /
//! `ListScripts` / `GetScript`.
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`String` / `Vec<u8>` /
//! `[u8; N]`), the [`crate::tool_registry_admin::ToolRegistryAdmin`] pattern — no
//! host type crosses the seam. The host (`kx-gateway`) implements it over the
//! same durable registry that backs tools, plus the content store the source
//! bytes live in.
//!
//! # Boundaries
//!
//! - **A script is a tool.** It registers into the same durable registry, is
//!   granted by the same `(name, version)` key, and fires through the same
//!   broker. These RPCs exist because a script's *declaration* differs (source
//!   bytes, an interpreter, a resource wish), not because its authority does.
//! - **Registration grants NO authority.** The declared wish becomes the tool's
//!   requirement; the broker still refuses any dispatch whose requirement is not
//!   a subset of the granting warrant. The client never supplies a warrant and
//!   never names an id — `script_id` is server-derived.
//! - **Off the truth path.** The registry is off-journal and off-digest; the
//!   source bytes are content-addressed. Nothing here can move the canonical
//!   digest.
//! - **Fail-closed admission.** An unknown interpreter, an interpreter absent
//!   from the host, or a serve with no sandbox available REFUSES the
//!   registration. A script that cannot run sandboxed must never become
//!   offerable, because appearing in the registry is what makes it offerable.
//! - **`None` seam ⇒ `unimplemented`.** A gateway without scripts wired degrades
//!   forward-compatibly.

/// One declared filesystem mount a script wishes for. `mode` is the closed set
/// `"ro"` | `"rw"` | `"exec"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptMountWire {
    /// Absolute path.
    pub path: String,
    /// `"ro"` | `"rw"` | `"exec"`.
    pub mode: String,
}

/// One environment pair fixed at registration. NEVER model-controlled.
#[derive(Clone, Debug)]
pub struct ScriptEnvWire {
    /// Variable name.
    pub key: String,
    /// Variable value.
    pub value: String,
}

/// A `RegisterScript` request. The server derives identity and compiles the
/// declared wish into a requirement; the client supplies neither.
#[derive(Clone, Debug)]
pub struct ScriptRegistration {
    /// Identity half — the grant-set key.
    pub script_name: String,
    /// Identity half.
    pub script_version: String,
    /// Free-form; shown to a model in its tool menu, never parsed for
    /// enforcement.
    pub description: String,
    /// The interpreter token, validated against the host's closed allowlist.
    pub interpreter: String,
    /// The script's source bytes.
    pub source: Vec<u8>,
    /// Fixed arguments appended after the script. NEVER model-controlled.
    pub argv: Vec<String>,
    /// Fixed environment. NEVER model-controlled; empty ⇒ no environment.
    pub env: Vec<ScriptEnvWire>,
    /// The filesystem the script declares it needs.
    pub fs_mounts: Vec<ScriptMountWire>,
    /// The hosts the script declares it needs. Empty ⇒ no egress.
    pub net_hosts: Vec<String>,
    /// Wall-clock budget in ms (0 ⇒ the host default).
    pub wall_clock_ms: u64,
    /// Memory ceiling in bytes (0 ⇒ unset).
    pub mem_bytes: u64,
    /// Output ceiling in bytes (0 ⇒ the host default). Exceeding it REFUSES the
    /// call rather than truncating.
    pub max_output_bytes: u64,
}

/// One registered script's inventory row. Every scope field is a DISPLAY
/// summary; authority never rides this seam.
#[derive(Clone, Debug)]
pub struct RegisteredScriptEntry {
    /// 16-byte server-derived id.
    pub script_id: [u8; 16],
    /// Identity half.
    pub script_name: String,
    /// Identity half.
    pub script_version: String,
    /// The interpreter token.
    pub interpreter: String,
    /// Free-form description.
    pub description: String,
    /// Lowercase hex of the source's content ref — the exact bytes that run.
    pub source_ref_hex: String,
    /// Display: `"none"` or `"ro:/a,rw:/b"`.
    pub fs_scope_summary: String,
    /// Display: `"none"` or `"egress:host[,host]"`.
    pub net_scope_summary: String,
    /// Wall-clock budget in ms.
    pub wall_clock_ms: u64,
    /// Output ceiling in bytes.
    pub max_output_bytes: u64,
}

/// Why a [`ScriptAdmin::register`] was refused.
#[derive(Debug, thiserror::Error)]
pub enum ScriptAdminError {
    /// A malformed or unsupported field — an unknown interpreter, an empty
    /// identity half, an empty or oversized source. Maps to `invalid_argument`.
    #[error("invalid script: {0}")]
    InvalidArgument(String),
    /// The host cannot run scripts sandboxed (no shim, or the declared
    /// interpreter is not installed), so it will not register one. Maps to
    /// `failed_precondition` — the request is well-formed but the serve cannot
    /// honour it, and running unsandboxed is not an alternative.
    #[error("scripts cannot run on this serve: {0}")]
    Unavailable(String),
    /// A durable-store failure. Maps to `internal`.
    #[error("script storage error: {0}")]
    Storage(String),
}

/// The script-registry admin seam. A `None` seam ⇒ the RPCs return
/// `unimplemented`.
pub trait ScriptAdmin: Send + Sync {
    /// Register a declared script: store its source, compile its wish into a
    /// requirement, and make it fireable. Returns the 16-byte SERVER-DERIVED id.
    ///
    /// # Errors
    /// [`ScriptAdminError`] on an invalid field, an unavailable sandbox or
    /// interpreter, or a storage failure.
    fn register(&self, reg: ScriptRegistration) -> Result<[u8; 16], ScriptAdminError>;

    /// Deregister a script by exact `(name, version)`. Returns `true` iff a row
    /// was removed.
    ///
    /// # Errors
    /// [`crate::error::GatewayError`] on a durable-store failure.
    fn deregister(
        &self,
        script_name: &str,
        script_version: &str,
    ) -> Result<bool, crate::error::GatewayError>;

    /// One deterministic `(name, version)`-ordered page, after an exclusive
    /// cursor. `limit` is pre-clamped by the service. Returns `(rows, has_more)`.
    ///
    /// # Errors
    /// [`crate::error::GatewayError`] on a read failure.
    fn list(
        &self,
        limit: usize,
        after: Option<(String, String)>,
    ) -> Result<(Vec<RegisteredScriptEntry>, bool), crate::error::GatewayError>;

    /// One script's row plus its source bytes, by exact `(name, version)`.
    /// `None` when absent.
    ///
    /// # Errors
    /// [`crate::error::GatewayError`] on a read failure.
    fn get(
        &self,
        script_name: &str,
        script_version: &str,
    ) -> Result<Option<(RegisteredScriptEntry, Vec<u8>)>, crate::error::GatewayError>;
}
