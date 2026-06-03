//! [`SecretStore`] — the resolver seam that turns an authorized [`SecretRef`]
//! into its secret value, transiently, at the transport (D110.2).
//!
//! The warrant (`secret_scope`, D110.3) AUTHORIZES which refs may resolve; the
//! broker precheck (`secret_scope ⊆ warrant`) is the single authorization gate.
//! This trait is the MECHANISM consumed *after* that gate — it makes no
//! authorization decision. The returned value is the ONLY place a secret
//! materializes; the caller injects it into a header / child env and drops it.
//! It is never stored on any struct, `EffectRequest`, `BrokerHandle`, the
//! journal, a `MoteId`, a `StepRecord`, or the model's context (D81/D110).
//!
//! OSS ships [`EnvSecretStore`] (host-environment passthrough). The hardened
//! per-tenant KMS/HSM vault (envelope encryption, rotation, mTLS, audit) is a
//! `kx-cloud/*` impl behind this same trait — the deployment boundary is the
//! trait seam (D28/D94). OSS makes no "best-cryptography vault" claim.

use kx_warrant::SecretRef;

/// Resolves a [`SecretRef`] to its secret value, transiently, at the transport.
///
/// Object-safe (`Send + Sync`) so a `Box<dyn McpTransport>` can hold an
/// `Arc<dyn SecretStore>`. Implementations MUST be pure-read (no mutation) and
/// MUST NOT log or persist the value.
pub trait SecretStore: Send + Sync {
    /// Resolve `secret_ref` to its value, or `None` if absent (the runtime then
    /// never fabricates a credential — the server fails its own auth).
    fn resolve(&self, secret_ref: &SecretRef) -> Option<String>;
}

/// The OSS simple impl: resolve a [`SecretRef`] as the name of a host
/// environment variable. `SecretRef("API_KEY")` → `std::env::var("API_KEY")`.
/// This is the pre-M5.3 `CredentialRef` behavior, now behind the seam.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvSecretStore;

impl SecretStore for EnvSecretStore {
    fn resolve(&self, secret_ref: &SecretRef) -> Option<String> {
        std::env::var(&secret_ref.0).ok()
    }
}
