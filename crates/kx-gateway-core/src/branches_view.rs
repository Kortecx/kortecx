//! The D155 Phase-A branch seam behind `CreateBranch` / `SnapshotInto` /
//! `ListBranches` / `GetBranch` / `DeleteBranch`.
//!
//! Spoken in gateway-core's own wire vocabulary (`[u8; 16]` / `[u8; 32]` /
//! `String`) — no host type crosses the seam (the [`crate::BundleStore`] /
//! [`crate::AlertView`] precedent). The host (`kx-gateway`) implements it over a
//! `branches.db` SQLite sidecar **plus** the content store + the operator
//! `KX_SERVE_FS_ROOT` mount (the host-touching `SnapshotInto` read lives in the
//! impl, so gateway-core stays host-agnostic).
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** `branches.db` is REBUILDABLE-TO-EMPTY: a branch's
//!   `{path -> ContentRef}` manifest records which content-store blobs a snapshot
//!   grouped. Truth (the blobs) lives in the content store; losing the sidecar
//!   loses only the manifest index. Never journaled, never a `MoteId` input,
//!   never a digest input — dropping the file cannot move the canonical
//!   projection digest (the `bundles.db` / D160 precedent).
//! - **Server-derived id (SN-8).** `branch_ref = blake3("kx-branch\0" ‖ handle ‖
//!   parent ‖ canonical(items))[..16]`; the client names a handle, never an id.
//! - **Caller-scoped.** Every method takes the SERVER-RESOLVED `principal`; a
//!   branch is visible only to the party that authored it (uniform not-found for
//!   absent OR not-owned — no cross-party existence oracle).
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves the five RPCs
//!   `unimplemented`.
//! - **Phase-A is READ-ONLY w.r.t. the host.** `SnapshotInto` READS confined host
//!   files INTO CAS; it NEVER writes the host. Governed host write-back is
//!   Phase-B (after PR-8) — no `ReadWrite` grant is exercised here.

use crate::error::GatewayError;

/// Server-side cap on the number of paths in one `SnapshotInto` call (fail-closed
/// at the handler — never an unbounded host-read fan-out).
pub const MAX_SNAPSHOT_PATHS: usize = 256;

/// Server-side cap on a branch description's byte length (advisory text only).
pub const MAX_BRANCH_DESCRIPTION_BYTES: usize = 4096;

/// One entry in a branch manifest: a snapshot-relative path + the server-derived
/// ref of the file's blob in the content store (`fs-read@1` -> CAS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchItemRecord {
    /// The snapshot-relative path (the manifest key + display).
    pub path: String,
    /// The 32-byte blake3 ref of the file's blob in the content store.
    pub content_ref: [u8; 32],
}

/// A branch's resolved manifest (the governance / display view + the edit source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchManifest {
    /// 16-byte SERVER-DERIVED manifest hash (SN-8; display + dedup signal).
    pub branch_ref: [u8; 16],
    /// The canonical `namespace/collection/name` handle (the upsert key).
    pub handle: String,
    /// The CoW parent handle (lineage); empty = a root branch.
    pub parent_handle: String,
    /// Advisory free-form description (never parsed for enforcement).
    pub description: String,
    /// The resolved `{path -> ref}` set, in path-sorted order.
    pub items: Vec<BranchItemRecord>,
}

/// The branch store seam: create / snapshot-into / enumerate / fetch / unbind a
/// caller's branches. A `None` seam on the service ⇒ the five RPCs return
/// `unimplemented`. `SnapshotInto`'s host file read lives in the host impl.
pub trait BranchStore: Send + Sync {
    /// Create (or upsert) the branch `(principal, handle)`. If `parent_handle` is
    /// `Some`, the new branch inherits the parent's resolved items at create time
    /// (a point-in-time CoW fork; later parent edits do NOT propagate). Returns
    /// `(manifest, deduplicated)` where `deduplicated` is `true` iff an identical
    /// manifest was already bound to this `(principal, handle)`.
    ///
    /// # Errors
    /// [`GatewayError::NotFound`] if `parent_handle` is set but unknown to the
    /// caller; a host write failure ([`GatewayError::Internal`]).
    fn create(
        &self,
        principal: &str,
        handle: &str,
        parent_handle: Option<&str>,
        description: &str,
    ) -> Result<(BranchManifest, bool), GatewayError>;

    /// Read `paths` (each confined under the operator FS root) INTO the content
    /// store and merge the resulting `{path -> ref}` entries into the branch
    /// `(principal, handle)` — creating it (optionally from `parent_handle`) if
    /// absent. `ingested` = how many paths were read this call. Returns
    /// `(manifest, ingested, deduplicated)`.
    ///
    /// # Errors
    /// [`GatewayError::FailedPrecondition`] when the host FS root is unconfigured
    /// (`KX_SERVE_FS_ROOT` unset — snapshot-in is default-OFF); a confinement
    /// failure ([`GatewayError::InvalidArgument`]); a read / write failure
    /// ([`GatewayError::Internal`]).
    fn snapshot_into(
        &self,
        principal: &str,
        handle: &str,
        parent_handle: Option<&str>,
        description: &str,
        paths: &[String],
    ) -> Result<(BranchManifest, usize, bool), GatewayError>;

    /// Fetch the resolved manifest of `(principal, handle)`, if any (caller-scoped).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn get(&self, principal: &str, handle: &str) -> Result<Option<BranchManifest>, GatewayError>;

    /// List `principal`'s branches in deterministic handle order, paged. Returns
    /// `(manifests, has_more)`; `after_handle` is an exclusive cursor.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<BranchManifest>, bool), GatewayError>;

    /// Unbind `(principal, handle)` (the CAS blobs stay). Returns `true` iff a row
    /// was removed.
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn delete(&self, principal: &str, handle: &str) -> Result<bool, GatewayError>;
}
