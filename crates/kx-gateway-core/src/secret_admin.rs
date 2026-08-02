//! The LOCAL secret-store admin seam (MM-3, D110 — `PutSecret` / `ListSecretNames`
//! / `DeleteSecret`).
//!
//! Spoken entirely in gateway-core's OWN vocabulary (`String` / `u64` / `bool`) — no
//! host storage type crosses the seam, the
//! [`crate::mcp_gateway_admin::McpGatewayAdmin`] pattern. The host (`kx-gateway`)
//! implements it over one operator-visible file under its catalog dir.
//!
//! # Boundaries
//!
//! - **Write-only value.** The secret VALUE is supplied ONLY to [`SecretAdmin::put`]
//!   (where the impl writes it to the local store and drops it). It NEVER appears on
//!   any return type, the wire, the journal, a `MoteId`, or the model's context.
//! - **NAMES only.** [`SecretAdmin::list_names`] returns NAMES + timestamps — the
//!   governance view — never a value.
//! - **Resolve is elsewhere.** This seam is the write/enumerate admin; resolution is
//!   the `kx-mcp` `SecretStore` (the store arm of the host `ChainedSecretStore`),
//!   gated by the broker `secret_scope` precheck (the sole authorization gate).
//! - **`None` seam ⇒ `unimplemented`.** A gateway without a secret store wired
//!   degrades forward-compatibly. The hardened KMS/HSM vault is CLOUD (D94).

/// One stored secret's NAME + non-sensitive timestamps (the `ListSecretNames` row).
/// Carries no value, by construction (D81).
#[derive(Clone, Debug)]
pub struct SecretNameView {
    /// The SecretRef NAME (what a connection's `credential_ref` points at).
    pub name: String,
    /// First-stored wall-clock (ms since epoch). Off-digest; advisory only.
    pub created_unix_ms: u64,
    /// Last-updated wall-clock (ms since epoch).
    pub updated_unix_ms: u64,
}

/// Why a [`SecretAdmin`] operation was refused.
#[derive(Debug, thiserror::Error)]
pub enum SecretAdminError {
    /// A malformed name (empty / too long / illegal chars). Maps to `invalid_argument`.
    #[error("invalid secret name: {0}")]
    InvalidArgument(String),
    /// The local store cannot be used on this host (unreadable, or refused for being
    /// readable beyond its owner). Maps to `failed_precondition` — honest, never a
    /// fabricated success.
    #[error("the local secret store is unavailable: {0}")]
    Unavailable(String),
    /// A store read/write failure. Maps to `internal`.
    #[error("secret store error: {0}")]
    Storage(String),
}

/// The local secret-store admin seam behind the 3 secret RPCs. The host implements it
/// over one operator-visible file. A `None` seam ⇒ the RPCs return
/// `unimplemented`. Write authorization (loopback-only + authed party) is enforced at
/// the gateway handler BEFORE this seam is called — the seam itself is a pure store.
pub trait SecretAdmin: Send + Sync {
    /// Store (or overwrite) the secret `value` under `name`. The value is dropped after
    /// the write; it is never returned, logged, or journaled.
    ///
    /// # Errors
    /// [`SecretAdminError::Unavailable`] if the store is unusable;
    /// [`SecretAdminError::Storage`] on a write failure. (Name validity is checked by the
    /// handler.)
    fn put(&self, name: &str, value: &str) -> Result<(), SecretAdminError>;

    /// List stored secret NAMES (deterministic `(name)` order), keyset-paged after
    /// `after_name` (exclusive; empty ⇒ from the start), up to `limit` (0 ⇒ a server
    /// default). Returns `(rows, has_more)`. NAMES only — never a value.
    ///
    /// # Errors
    /// [`SecretAdminError::Storage`] on a read failure.
    fn list_names(
        &self,
        limit: u32,
        after_name: &str,
    ) -> Result<(Vec<SecretNameView>, bool), SecretAdminError>;

    /// Delete the secret `name` from the store. Returns `true` iff a
    /// secret was removed (`false` ⇒ no such name).
    ///
    /// # Errors
    /// [`SecretAdminError::Storage`] on a delete failure.
    fn delete(&self, name: &str) -> Result<bool, SecretAdminError>;
}
