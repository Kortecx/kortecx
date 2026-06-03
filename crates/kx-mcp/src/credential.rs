//! [`CredentialRef`] — a reference to a secret, never the secret itself (D81).
//!
//! Auth for an MCP server (API keys, OAuth tokens) is supplied **out-of-band**: a
//! `CredentialRef` names a [`SecretRef`] (the env-var name, or — via the cloud
//! [`SecretStore`] — a vault key); the secret *value* is read transiently at
//! transport-setup time through the transport's [`SecretStore`] and handed to the
//! child process's environment (stdio) or a request header (HTTP). The value is
//! never stored on any struct, never placed in an `EffectRequest`, a
//! `BrokerHandle`, the journal, a `MoteId`, or a `StepRecord`. `Debug`/`Display`
//! print only the *identity* (the ref name), so a logged credential ref never
//! leaks the secret.

use std::process::Command;

use kx_warrant::SecretRef;

use crate::secret_store::SecretStore;

/// A reference to a credential by its [`SecretRef`] (the env-var name, or a cloud
/// vault key). The secret value is read on demand at injection time through a
/// [`SecretStore`] and never stored.
///
/// `Debug`/`Display` deliberately print only the ref name (the identity that
/// acted), never the secret — D81's "records *which* credential identity acted,
/// never the secret".
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRef {
    secret_ref: SecretRef,
}

impl CredentialRef {
    /// Construct a credential reference from the host environment-variable name
    /// that holds the secret (resolved by the OSS [`crate::EnvSecretStore`]).
    #[must_use]
    pub fn from_env_var(var_name: impl Into<String>) -> Self {
        Self {
            secret_ref: SecretRef(var_name.into()),
        }
    }

    /// Construct a credential reference from an explicit [`SecretRef`] (e.g. a
    /// cloud vault key resolved by a `kx-cloud` [`SecretStore`]).
    #[must_use]
    pub fn from_secret_ref(secret_ref: SecretRef) -> Self {
        Self { secret_ref }
    }

    /// The underlying [`SecretRef`] — used by the transport to check the dispatch's
    /// authorized `secret_scope` before resolving (defense-in-depth).
    #[must_use]
    pub fn secret_ref(&self) -> &SecretRef {
        &self.secret_ref
    }

    /// The credential's identity (the ref name). Safe to log — never the secret.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.secret_ref.0
    }

    /// Inject the referenced secret into `cmd`'s environment, reading it through
    /// `store` transiently. If the ref is unresolvable, no env entry is added (the
    /// server then fails its own auth — the runtime never fabricates a credential).
    /// The secret value is dropped at the end of this call; it is never returned or
    /// stored.
    pub fn inject_into(&self, store: &dyn SecretStore, cmd: &mut Command) {
        if let Some(secret) = store.resolve(&self.secret_ref) {
            cmd.env(&self.secret_ref.0, secret);
        }
    }

    /// Read the referenced secret transiently through `store`, for injection as an
    /// HTTP request header (the M5.2b [`crate::HttpTransport`] path — stdio injects
    /// into the child env instead, [`inject_into`]).
    ///
    /// Returns `None` when the ref is unresolvable (the server then fails its own
    /// auth — the runtime never fabricates a credential). The returned `String` is
    /// the ONLY place the secret materializes: the caller injects it into a header
    /// and drops it; it is never stored on this struct, an `EffectRequest`, a
    /// `BrokerHandle`, the journal, a `MoteId`, or a `StepRecord` (D81). Because the
    /// type still holds only the ref name, `Debug`/`Display` redaction is unaffected.
    ///
    /// [`inject_into`]: Self::inject_into
    #[must_use]
    pub fn read_secret(&self, store: &dyn SecretStore) -> Option<String> {
        store.resolve(&self.secret_ref)
    }
}

// Manual `Debug` — print ONLY the identity, never let a derived Debug expose a
// future secret-bearing field (the type holds none today, and this keeps it so).
impl std::fmt::Debug for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRef")
            .field("identity", &self.secret_ref.0)
            .finish()
    }
}

impl std::fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "credential:{}", self.secret_ref.0)
    }
}
