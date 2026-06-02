//! [`CredentialRef`] — a reference to a secret, never the secret itself (D81).
//!
//! Auth for an MCP server (API keys, OAuth tokens) is supplied **out-of-band**: a
//! `CredentialRef` names a host environment variable (or, later, a cloud
//! secret-broker key); the secret *value* is read transiently at transport-setup
//! time and handed to the child process's environment. The value is never stored on
//! any struct, never placed in an `EffectRequest`, a `BrokerHandle`, the journal, a
//! `MoteId`, or a `StepRecord`. `Debug`/`Display` print only the *identity* (the
//! variable name), so a logged credential ref never leaks the secret.

use std::process::Command;

/// A reference to a credential by the name of the host environment variable that
/// holds it. The secret value is read on demand at injection time and never stored.
///
/// `Debug`/`Display` deliberately print only the variable name (the identity that
/// acted), never the secret — D81's "records *which* credential identity acted,
/// never the secret".
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRef(String);

impl CredentialRef {
    /// Construct a credential reference from the host environment-variable name
    /// that holds the secret.
    #[must_use]
    pub fn from_env_var(var_name: impl Into<String>) -> Self {
        Self(var_name.into())
    }

    /// The credential's identity (the env-var name). Safe to log — never the secret.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.0
    }

    /// Inject the referenced secret into `cmd`'s environment, reading it from the
    /// host environment transiently. If the variable is unset, no env entry is
    /// added (the server then fails its own auth — the runtime never fabricates a
    /// credential). The secret value is dropped at the end of this call; it is
    /// never returned or stored.
    pub fn inject_into(&self, cmd: &mut Command) {
        if let Ok(secret) = std::env::var(&self.0) {
            cmd.env(&self.0, secret);
        }
    }
}

// Manual `Debug` — print ONLY the identity, never let a derived Debug expose a
// future secret-bearing field (the type holds none today, and this keeps it so).
impl std::fmt::Debug for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRef")
            .field("identity", &self.0)
            .finish()
    }
}

impl std::fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "credential:{}", self.0)
    }
}
