//! The PR-7 context-bundle seam behind `PutContextBundle` / `ListContextBundles`
//! / `GetContextBundle` / `DeleteContextBundle` and the bind-time resolution of
//! the `context_bundles` field on Invoke / SubmitWorkflow.
//!
//! Spoken in gateway-core's own wire vocabulary (`[u8; 16]` / `[u8; 32]` /
//! `String`) — no host type crosses the seam (the [`crate::UploadsLedger`] /
//! [`crate::AlertView`] precedent). The host (`kx-gateway`) implements it over a
//! `bundles.db` SQLite sidecar.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The sidecar is REBUILDABLE-TO-EMPTY: a bundle's
//!   manifest records which content-store blobs a caller grouped under a handle.
//!   Truth (the blobs) lives in the content store; losing the sidecar loses only
//!   the manifest index, and re-authoring restores it at the SAME `bundle_ref`
//!   (content-addressed). Never journaled, never a `MoteId` input, never a digest
//!   input — dropping the file cannot move the canonical projection digest.
//! - **Server-derived id (SN-8).** `bundle_ref = blake3("kx-bundle\0" ‖ handle ‖
//!   canonical(items))[..16]`; the client names a handle, never an identity.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal` (from
//!   the auth interceptor); a bundle is visible only to the party that authored
//!   it (uniform not-found for absent OR not-owned — no cross-party oracle).
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves the four RPCs
//!   `unimplemented` and `context_bundles` resolution empty (a clear bind error).

use crate::error::GatewayError;

/// Server-side cap on the number of items in one bundle (fail-closed at the
/// `PutContextBundle` handler — mirrors `MAX_BATCH_REFS` for batch reads).
pub const MAX_CONTEXT_BUNDLE_ITEMS: usize = 256;

/// Server-side cap on a bundle description's byte length (advisory text only).
pub const MAX_BUNDLE_DESCRIPTION_BYTES: usize = 4096;

/// One item in a context bundle: an advisory label + the server-derived ref of a
/// blob already in the content store (`PutContent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleItemRecord {
    /// Advisory label / context heading (display only — never identity).
    pub name: String,
    /// The 32-byte blake3 ref of a blob in the content store (the join key).
    pub content_ref: [u8; 32],
    /// Advisory mime as supplied by the caller (display / classify only).
    pub media_type: String,
}

/// A bundle's bound manifest (the governance / display view + the bind source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleManifest {
    /// 16-byte SERVER-DERIVED manifest hash (SN-8; display + dedup signal).
    pub bundle_ref: [u8; 16],
    /// The canonical `namespace/collection/name` handle (the upsert key).
    pub handle: String,
    /// Advisory free-form description (never parsed for enforcement).
    pub description: String,
    /// The bound items, in author order.
    pub items: Vec<BundleItemRecord>,
}

/// The context-bundle store seam: author / enumerate / fetch / unbind a caller's
/// bundles, and (via [`BundleStore::get`]) resolve a handle to its item refs at
/// bind time. A `None` seam on the service ⇒ the four RPCs return `unimplemented`
/// and `context_bundles` cannot be resolved (a clear bind error, fail-closed).
pub trait BundleStore: Send + Sync {
    /// Upsert the bundle bound to `(principal, handle)`; the server derives
    /// `bundle_ref` from `(handle, items)`. Returns `(bundle_ref, deduplicated)`
    /// where `deduplicated` is `true` iff an identical manifest was already bound
    /// to this `(principal, handle)`.
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn upsert(
        &self,
        principal: &str,
        handle: &str,
        description: &str,
        items: &[BundleItemRecord],
    ) -> Result<([u8; 16], bool), GatewayError>;

    /// Fetch the bundle bound to `(principal, handle)`, if any (caller-scoped).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn get(&self, principal: &str, handle: &str) -> Result<Option<BundleManifest>, GatewayError>;

    /// List `principal`'s bundles in deterministic handle order, paged. Returns
    /// `(manifests, has_more)`; `after_handle` is an exclusive cursor.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<BundleManifest>, bool), GatewayError>;

    /// Unbind `(principal, handle)` (the CAS blobs stay). Returns `true` iff a row
    /// was removed.
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn delete(&self, principal: &str, handle: &str) -> Result<bool, GatewayError>;
}
