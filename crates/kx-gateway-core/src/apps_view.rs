//! The POC-4 App-catalog seam behind `SaveApp` / `ListApps` / `GetApp`.
//!
//! An "App" is a `kortecx.app/v1` envelope (a portable blueprint wrapped with
//! by-REFERENCE references, a 4-axis steering config, and replay intent). Spoken
//! in gateway-core's own wire vocabulary — **opaque envelope BYTES** + a
//! host-derived [`AppRecord`] summary + a `[u8; 16]` ref. No envelope type crosses
//! the seam, so gateway-core never links `kx-app`; the host (`kx-gateway`)
//! canonicalizes + validates the envelope and derives the summary + `app_ref`.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The `apps.db` sidecar is REBUILDABLE-TO-EMPTY (the
//!   `bundles.db`/D160 posture): an App envelope references content-store blobs +
//!   registry ids; it is NOT journal-derivable. Never journaled, never a `MoteId`
//!   input, never a digest input — dropping the file cannot move the canonical
//!   projection digest.
//! - **Carries NO authority (SN-8 / BLOCKER #5).** The envelope holds references +
//!   an authorship claim only — `app run` re-compiles the blueprint and the server
//!   re-resolves every warrant from the caller's OWN grants. The host validates
//!   that the envelope carries no warrant/grant/secret/credential/`instance_id`.
//! - **Server-derived id.** `app_ref = blake3("kx-app\0" ‖ handle ‖ canonical(envelope))[..16]`;
//!   the client names a handle, never an identity. The host re-canonicalizes the
//!   received bytes so client byte-ordering never affects identity.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal`; an App
//!   is visible only to the party that authored it (uniform not-found for absent OR
//!   not-owned — no cross-party existence oracle).
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves the three RPCs
//!   `unimplemented` (a clear, fail-closed signal).
//! - **No cross-instance import** in this seam (a sharing feature, deferred).

use crate::error::GatewayError;

/// Fail-closed cap on a single App envelope's serialized size (checked at the
/// `SaveApp` handler BEFORE any host touch).
pub const MAX_APP_ENVELOPE_BYTES: usize = 1 << 20; // 1 MiB

/// A stored App's summary — the catalog/display view. The envelope bytes are
/// opaque to gateway-core; the host derives every field from the canonical JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppRecord {
    /// 16-byte SERVER-DERIVED canonical-envelope hash (display + dedup signal).
    pub app_ref: [u8; 16],
    /// The canonical `namespace/collection/name` handle (the upsert key).
    pub handle: String,
    /// Envelope name.
    pub name: String,
    /// Envelope version.
    pub version: String,
    /// Advisory description (never parsed for enforcement).
    pub description: String,
    /// Catalog tags.
    pub tags: Vec<String>,
    /// Blueprint step count (display only).
    pub step_count: u32,
}

/// The App-catalog store seam: save / enumerate / fetch a caller's App envelopes.
/// Opaque envelope bytes cross the seam; identity + summary are host-derived. A
/// `None` seam on the service ⇒ the three RPCs return `unimplemented`.
pub trait AppCatalog: Send + Sync {
    /// Upsert the envelope bound to `(principal, handle)`. The host validates +
    /// canonicalizes `envelope_json`, derives `app_ref` + the summary, and stores
    /// the canonical bytes. Returns `(record, deduplicated)` where `deduplicated`
    /// is `true` iff an identical canonical envelope was already bound here.
    ///
    /// # Errors
    /// [`GatewayError::InvalidArgument`] if the envelope fails validation;
    /// [`GatewayError::Internal`] on a host write failure.
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
    ) -> Result<(AppRecord, bool), GatewayError>;

    /// List `principal`'s apps in deterministic handle order, paged. Returns
    /// `(records, has_more)`; `after_handle` is an exclusive cursor.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<AppRecord>, bool), GatewayError>;

    /// Fetch `(record, canonical_envelope_bytes)` bound to `(principal, handle)`,
    /// if any (caller-scoped; uniform not-found for absent OR not-owned).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(AppRecord, Vec<u8>)>, GatewayError>;
}
