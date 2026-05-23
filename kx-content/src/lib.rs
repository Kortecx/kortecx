#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # kx-content — content-addressed payload store
//!
//! Stores large payloads (inference outputs, staged effects, topology decisions) keyed by
//! BLAKE3 content hash. The journal layer holds only 32-byte [`ContentRef`] values; the
//! payloads themselves live here.
//!
//! ## Why content-addressing
//!
//! - **Auto-dedup**: two callers writing identical bytes produce identical refs and share one
//!   underlying object. The runtime relies on this when two workers race on a Mote.
//! - **Atomic-per-object writes**: either the object at `name = blake3(bytes)` exists with
//!   exactly those bytes, or it does not exist. Partial writes are never observable. The
//!   local-FS backend gets this via write-to-temp + atomic rename; an S3 backend would use
//!   native atomic PUT.
//! - **Structural idempotence**: writing the same bytes twice is a no-op on the second call.
//! - **No in-band rollback**: a writer that crashes between [`ContentStore::put`] and the
//!   journal txn that anchors the ref leaves an orphan — reclaimed later by an out-of-band
//!   GC walker. The store itself has no rollback path.
//!
//! ## Trait surface is backend-agnostic
//!
//! The [`ContentStore`] trait does not name `bytes::Bytes` or `rkyv` — those are
//! impl-ergonomics for the local-FS backend, not contract requirements. A future replicated /
//! S3 backend implements the same trait with its own [`ContentStore::Payload`] type. This is
//! what makes the OSS-vs-cloud split a feature flag rather than a fork.
//!
//! ## What lives here
//!
//! - [`ContentRef`] — 32-byte BLAKE3 hash newtype, opaque outside the store.
//! - [`ContentStore`] — the backend-agnostic trait.
//! - [`LocalFsContentStore`] — the OSS local-filesystem backend with `bytes::Bytes`
//!   zero-copy reads and POSIX-rename atomicity.
//! - [`InMemoryContentStore`] — an in-memory backend used in tests and as proof that the
//!   trait is genuinely backend-agnostic (no in-process or filesystem assumptions in the
//!   trait signature).
//!
//! ## What does NOT live here
//!
//! - The journal (`kx-journal`, P1.4) — content store must NOT depend on the journal; the
//!   orphan-GC walker lives outside both crates and joins their views.
//! - Tag-driven storage tiering (P1.12) — the store is tag-blind. The tiering pass joins the
//!   journal's per-Mote `NdClass` with the store's enumeration to decide what to evict.
//! - Streaming reads (post-P1) — [`ContentStore::get`] returns the full payload.

use std::ops::Deref;

use serde::{Deserialize, Serialize};

pub use crate::in_memory::InMemoryContentStore;
pub use crate::local_fs::LocalFsContentStore;

mod in_memory;
mod local_fs;

// ---------------------------------------------------------------------------
// ContentRef — the opaque 32-byte content hash
// ---------------------------------------------------------------------------

/// A 32-byte BLAKE3 content hash. The identity of a payload in the store.
///
/// `ContentRef`s are opaque outside the store. Callers (journal, projection, executor)
/// compare them as 32-byte tokens; they do not parse subfields, derive paths, or assume any
/// prefix structure. Sharding by hash prefix is a backend-internal optimization, never a
/// caller concern.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ContentRef(pub [u8; 32]);

impl ContentRef {
    /// Construct a `ContentRef` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Compute the `ContentRef` of an arbitrary byte slice without writing it to a store.
    ///
    /// Useful for callers that want to know the ref of bytes they have not yet uploaded
    /// (e.g., precomputing an idempotency key derived from a payload that will be staged
    /// later). The store's [`ContentStore::put`] implementation MUST yield the same ref for
    /// the same bytes.
    #[inline]
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Lowercase 64-character hex of the underlying hash. Suitable for use as a
    /// filesystem-safe object name.
    #[must_use]
    pub fn to_hex(&self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl std::fmt::Debug for ContentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ContentRef({})", self.to_hex())
    }
}

impl std::fmt::Display for ContentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A read for a `ContentRef` that the store does not have.
///
/// Distinct from [`StoreError`] because `NotFound` is a *normal* outcome — a PURE payload
/// may have been evicted by the tiering pass (`mote.md` §6) and recomputing is the expected
/// recovery. The caller decides whether to recompute, fail the workflow, or escalate to ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("content ref not found in store")]
pub struct NotFound;

/// Errors raised by mutating store operations.
///
/// Reads use [`NotFound`] as a non-fatal signal; mutating operations return this richer
/// error type for genuine I/O / filesystem failures.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// An I/O error from the backend (filesystem, network, etc.).
    #[error("backend I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The backend wrote bytes whose BLAKE3 hash does not match the requested ref. Indicates
    /// data corruption mid-write; should not happen with the content-addressed naming
    /// discipline but is checked for defensively (the asymmetry rule: refuse loudly).
    #[error("content hash mismatch: wrote bytes whose blake3 != target ref")]
    HashMismatch,
}

// ---------------------------------------------------------------------------
// The trait — backend-agnostic surface
// ---------------------------------------------------------------------------

/// The content-addressed payload store.
///
/// Implementors choose their own `Payload` deref-target (e.g., `bytes::Bytes` for the local
/// backend; a streaming-bytes wrapper for a future remote backend). The trait does not name
/// any in-process-specific type — that is what keeps the OSS and cloud impls behind one
/// signature.
///
/// ## Contracts
///
/// - [`ContentStore::put`] is atomic-per-object: a successful return means the bytes are
///   durably present at the returned ref; a failure means no object at that ref exists.
///   Partial writes are not observable.
/// - [`ContentStore::put`] is idempotent: writing identical bytes twice yields identical
///   refs and stores one underlying object.
/// - [`ContentStore::get`] returns [`NotFound`] for refs that were never written OR for
///   refs whose backing object was evicted (by tiering or by GC). Callers MUST be ready to
///   handle the same `NotFound` either way.
/// - [`ContentStore::delete`] is idempotent: deleting a non-existent ref is a no-op success.
pub trait ContentStore {
    /// The owned-or-borrowed bytes type returned by [`ContentStore::get`]. Implementors
    /// choose; callers see only the `Deref<Target = [u8]>` view.
    type Payload: Deref<Target = [u8]>;

    /// Write `bytes` to the store, returning the resulting [`ContentRef`].
    ///
    /// Atomic-per-object: either the object at the returned ref exists with exactly `bytes`,
    /// or the call returns an error and the object does not exist. Idempotent on the bytes:
    /// a second call with identical bytes is a no-op that returns the same ref.
    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError>;

    /// Read the payload at `r`. Returns [`NotFound`] if the ref is unknown or its backing
    /// object has been reclaimed.
    fn get(&self, r: &ContentRef) -> Result<Self::Payload, NotFound>;

    /// Delete the object at `r`, if present. Idempotent: deleting an absent ref is a no-op
    /// success. The orphan-GC walker (out-of-band) and the tiering pass (P1.12) both call
    /// this; the store does not distinguish.
    fn delete(&self, r: &ContentRef) -> Result<(), StoreError>;

    /// Enumerate every ref currently present in the store.
    ///
    /// Used by the orphan-GC walker (joined against the journal's `list_committed_refs`)
    /// and by the tiering pass. Implementors return a boxed iterator; callers do not assume
    /// any particular order.
    fn list_refs<'a>(&'a self) -> Box<dyn Iterator<Item = ContentRef> + 'a>;

    /// `true` if the store currently has an object at `r`. Convenience over `get(r).is_ok()`
    /// that lets the backend skip materializing the payload.
    fn contains(&self, r: &ContentRef) -> bool;
}
