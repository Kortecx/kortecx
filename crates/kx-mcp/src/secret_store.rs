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

use std::sync::Arc;

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

/// A two-arm resolver: try `primary`, then fall back to `secondary`.
///
/// This is the seam that lets a host-side local store take precedence while
/// [`EnvSecretStore`] stays a permanent fallback — so every connection whose
/// `credential_ref` names a host env var keeps resolving unchanged (back-compat),
/// and a name present in BOTH wins from the `primary` (the local store). It is a
/// pure combinator with no dependency of its own; the concrete store impl lives in
/// the host crate, keeping this adapter dependency-clean.
pub struct ChainedSecretStore {
    primary: Arc<dyn SecretStore>,
    secondary: Arc<dyn SecretStore>,
}

impl ChainedSecretStore {
    /// Resolve through `primary` first, then `secondary`.
    #[must_use]
    pub fn new(primary: Arc<dyn SecretStore>, secondary: Arc<dyn SecretStore>) -> Self {
        Self { primary, secondary }
    }
}

impl SecretStore for ChainedSecretStore {
    fn resolve(&self, secret_ref: &SecretRef) -> Option<String> {
        self.primary
            .resolve(secret_ref)
            .or_else(|| self.secondary.resolve(secret_ref))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed in-memory store, standing in for the host secret store in unit tests.
    struct MapStore(std::collections::BTreeMap<String, String>);
    impl SecretStore for MapStore {
        fn resolve(&self, secret_ref: &SecretRef) -> Option<String> {
            self.0.get(&secret_ref.0).cloned()
        }
    }

    #[test]
    fn chained_prefers_primary_then_falls_back() {
        let primary = Arc::new(MapStore(
            [("SHARED".to_string(), "from-primary".to_string())]
                .into_iter()
                .collect(),
        ));
        let secondary = Arc::new(MapStore(
            [
                ("SHARED".to_string(), "from-secondary".to_string()),
                ("ONLY_SECONDARY".to_string(), "fallback-value".to_string()),
            ]
            .into_iter()
            .collect(),
        ));
        let chained = ChainedSecretStore::new(primary, secondary);

        // present in both ⇒ primary wins
        assert_eq!(
            chained.resolve(&SecretRef("SHARED".into())).as_deref(),
            Some("from-primary")
        );
        // absent in primary ⇒ falls back to secondary (the env back-compat path)
        assert_eq!(
            chained
                .resolve(&SecretRef("ONLY_SECONDARY".into()))
                .as_deref(),
            Some("fallback-value")
        );
        // absent in both ⇒ None (the runtime never fabricates a credential)
        assert_eq!(chained.resolve(&SecretRef("MISSING".into())), None);
    }
}
