#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]
// TODO(workspace.lints cleanup): the `InMemoryContentStore` backend uses
// the canonical Rust idiom `.read().expect("poisoned lock")` / `.write().
// expect("poisoned lock")` at every lock-acquisition site (6 sites in
// `in_memory.rs`). Poisoned locks indicate a panic occurred while
// another thread held the lock — the runtime's correct response is to
// propagate that panic to the caller (every higher layer ALSO operates
// under "single-writer-per-run"; a poisoned lock means catastrophic state).
// A follow-up may migrate these to typed `PoisonError` returns; until
// then, the documented `expect("poisoned lock")` is intentional.
#![allow(clippy::expect_used)]
// Inline test modules are exempted from the workspace deny on `unwrap_used`.
// `expect_used` is already allowed unconditionally above. Integration tests
// under tests/*.rs compile as separate crates and carry their own per-file
// allows.
#![cfg_attr(test, allow(clippy::unwrap_used))]

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

pub use bytes::Bytes;
pub use crate::in_memory::InMemoryContentStore;
pub use crate::local_fs::LocalFsContentStore;
pub use crate::sniff::{sniff_image_format, ImageFormat};

mod in_memory;
mod local_fs;
mod sniff;

// ---------------------------------------------------------------------------
// ContentRef — the opaque 32-byte content hash
// ---------------------------------------------------------------------------

/// A 32-byte BLAKE3 content hash. The identity of a payload in the store.
///
/// `ContentRef`s are opaque outside the store. Callers (journal, projection, executor)
/// compare them as 32-byte tokens; they do not parse subfields, derive paths, or assume any
/// prefix structure. Sharding by hash prefix is a backend-internal optimization, never a
/// caller concern.
///
/// # Examples
///
/// ```
/// use kx_content::ContentRef;
///
/// let a = ContentRef::of(b"hello world");
/// let b = ContentRef::of(b"hello world");
/// assert_eq!(a, b, "content-addressed: same bytes → same ref");
///
/// let c = ContentRef::of(b"different");
/// assert_ne!(a, c);
///
/// // 64-character lowercase hex, suitable for filesystem-safe names.
/// assert_eq!(a.to_hex().len(), 64);
/// ```
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

    /// Parse a `ContentRef` from a 64-character hex string — the inverse of
    /// [`to_hex`](Self::to_hex). Returns `None` unless the input is EXACTLY 64 hex
    /// digits (upper- or lower-case), so a malformed/short ref is fail-closed, never
    /// silently coerced into a valid-looking one. The shared decoder for every caller
    /// that resolves a content ref carried as a hex string (e.g. a bound `image_ref`
    /// recipe arg read by both the gateway executor and the coordinator anchor).
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, pair) in b.chunks_exact(2).enumerate() {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            out[i] = u8::try_from(hi * 16 + lo).ok()?;
        }
        Some(Self(out))
    }

    /// Parse a `ContentRef` from a config/recipe ARG byte value that carries a 64-hex ref
    /// EITHER as a JSON string (`"<hex>"`, what the recipe binder writes) OR as the raw
    /// hex bytes (what the chains-DSL `flow().image(ref)` params path may write). Strips a
    /// single pair of surrounding double-quotes if present, trims whitespace, then
    /// validates strictly via [`from_hex`](Self::from_hex) — so a malformed value is
    /// fail-closed, never coerced. The std-only tolerance precedent is
    /// `reasoning_mode_from_config` (this crate carries no `serde_json` dependency).
    #[must_use]
    pub fn from_arg(raw: &[u8]) -> Option<Self> {
        let s = std::str::from_utf8(raw).ok()?.trim();
        let inner = s
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(s);
        Self::from_hex(inner)
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
///
/// # Examples
///
/// Put → get round-trip via the in-memory backend (zero filesystem state):
///
/// ```
/// use kx_content::{ContentStore, InMemoryContentStore};
///
/// let store = InMemoryContentStore::new();
/// let r = store.put(b"some payload").unwrap();
/// let got = store.get(&r).unwrap();
/// assert_eq!(&*got, b"some payload");
///
/// // Idempotent: second put with same bytes returns the same ref.
/// let r2 = store.put(b"some payload").unwrap();
/// assert_eq!(r, r2);
/// assert_eq!(store.len(), 1);
/// ```
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

/// Blanket impl so a shared `Arc<S>` is itself a [`ContentStore`] (delegating to
/// the inner store). Lets components hold an `Arc`-wrapped store directly —
/// e.g. the projection's verdict resolver and the topology materializer share
/// one store without each needing a bespoke wrapper. Purely additive; no
/// behavior change for the inner store.
impl<S: ContentStore + ?Sized> ContentStore for std::sync::Arc<S> {
    type Payload = S::Payload;

    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError> {
        (**self).put(bytes)
    }

    fn get(&self, r: &ContentRef) -> Result<Self::Payload, NotFound> {
        (**self).get(r)
    }

    fn delete(&self, r: &ContentRef) -> Result<(), StoreError> {
        (**self).delete(r)
    }

    fn list_refs<'a>(&'a self) -> Box<dyn Iterator<Item = ContentRef> + 'a> {
        (**self).list_refs()
    }

    fn contains(&self, r: &ContentRef) -> bool {
        (**self).contains(r)
    }
}

// ---------------------------------------------------------------------------
// SharedContent — the object-safe erasure seam (D181.4)
// ---------------------------------------------------------------------------

/// The narrow, object-safe view of a [`ContentStore`] that the runtime holds
/// type-erased behind a trait object.
///
/// [`ContentStore`] itself is NOT object-safe — its `type Payload` associated
/// type makes `dyn ContentStore` illegal (the established `BodyResolver` /
/// `ContentStoreBodyResolver<S>` pattern in `kx-executor` exists for exactly
/// this reason). `SharedContent` sidesteps that by committing the seam to the
/// canonical `Bytes` payload and exposing only the three methods the worker
/// and coordinator actually use (`get`/`put`/`contains` — never `delete` /
/// `list_refs`). It is `Send + Sync` (mirroring `BodyResolver`) so the fleet
/// can share one store across `tokio::spawn`ed workers.
///
/// Any `ContentStore<Payload = Bytes>` (the local FS store, the in-memory test
/// store, and the cloud S3 store all qualify) is a `SharedContent` via the
/// blanket impl below, so `Arc<ConcreteStore>` unsize-coerces to
/// [`SharedStore`] at every call site with no per-site change. This is what
/// keeps the OSS/cloud content split a config flag rather than a fork.
pub trait SharedContent: Send + Sync {
    /// Read the payload at `r`. See [`ContentStore::get`].
    fn get(&self, r: &ContentRef) -> Result<Bytes, NotFound>;

    /// Write `bytes`, returning the resulting [`ContentRef`]. See
    /// [`ContentStore::put`].
    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError>;

    /// `true` if the store currently has an object at `r`. See
    /// [`ContentStore::contains`].
    fn contains(&self, r: &ContentRef) -> bool;
}

/// Blanket impl: every `Send + Sync` content store over the canonical `Bytes`
/// payload is a [`SharedContent`]. Additive; delegates verbatim to the
/// underlying [`ContentStore`] methods (byte-identical behavior). The `?Sized`
/// bound lets an `Arc<dyn ContentStore<…>>` also satisfy it, harmlessly.
impl<S: ContentStore<Payload = Bytes> + Send + Sync + ?Sized> SharedContent for S {
    fn get(&self, r: &ContentRef) -> Result<Bytes, NotFound> {
        ContentStore::get(self, r)
    }

    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError> {
        ContentStore::put(self, bytes)
    }

    fn contains(&self, r: &ContentRef) -> bool {
        ContentStore::contains(self, r)
    }
}

/// A type-erased, shareable content handle — the runtime's store type once the
/// concrete backend (local FS or S3) has been chosen at the edge. Holds only
/// the [`SharedContent`] surface (`get`/`put`/`contains`).
pub type SharedStore = std::sync::Arc<dyn SharedContent>;

#[cfg(test)]
mod tests {
    use super::ContentRef;

    #[test]
    fn shared_content_erasure_roundtrips_and_is_digest_stable() {
        use std::sync::Arc;

        use super::{ContentStore, InMemoryContentStore, SharedStore};

        let bytes = b"content-store bridge D181.4";

        // Baseline: the concrete store's ContentRef for these bytes.
        let concrete = InMemoryContentStore::new();
        let concrete_ref = concrete.put(bytes).unwrap();

        // The same backend, type-erased to the runtime's `SharedStore` handle
        // (this is the coercion every Worker / CoordinatorService call site does).
        let shared: SharedStore = Arc::new(InMemoryContentStore::new());
        let shared_ref = shared.put(bytes).unwrap();

        // The erasure moves no bytes: identical ContentRef (digest-stable),
        // identical payload on read-back, and `contains` agrees — proving the
        // seam delegates verbatim to `ContentStore`.
        assert_eq!(shared_ref, concrete_ref, "erased put must be digest-identical");
        assert_eq!(&shared.get(&shared_ref).unwrap()[..], &bytes[..]);
        assert!(shared.contains(&shared_ref));
        assert!(!shared.contains(&ContentRef::from_bytes([0x00; 32])));
    }

    #[test]
    fn from_hex_roundtrips_and_is_fail_closed() {
        let r = ContentRef::from_bytes([0xab; 32]);
        // Round-trips through to_hex (lower-case) and accepts upper-case input too.
        assert_eq!(ContentRef::from_hex(&r.to_hex()), Some(r));
        assert_eq!(ContentRef::from_hex(&r.to_hex().to_uppercase()), Some(r));
        // Fail-closed on wrong length and non-hex digits — never coerced.
        assert_eq!(ContentRef::from_hex(""), None);
        assert_eq!(ContentRef::from_hex(&"a".repeat(63)), None);
        assert_eq!(ContentRef::from_hex(&"a".repeat(65)), None);
        assert_eq!(ContentRef::from_hex(&"z".repeat(64)), None);
    }

    #[test]
    fn from_arg_accepts_json_string_or_raw_hex_and_is_fail_closed() {
        let r = ContentRef::from_bytes([0xcd; 32]);
        let hex = r.to_hex();
        // Raw hex bytes (the chains-DSL params path).
        assert_eq!(ContentRef::from_arg(hex.as_bytes()), Some(r));
        // A JSON string of the hex (what the recipe binder writes) — quotes stripped.
        assert_eq!(
            ContentRef::from_arg(format!("\"{hex}\"").as_bytes()),
            Some(r)
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            ContentRef::from_arg(format!("  {hex}\n").as_bytes()),
            Some(r)
        );
        // Fail-closed: a half-quoted / wrong-length / non-hex value is never coerced.
        assert_eq!(ContentRef::from_arg(format!("\"{hex}").as_bytes()), None);
        assert_eq!(ContentRef::from_arg(b"\"\""), None);
        assert_eq!(ContentRef::from_arg(b"not-hex"), None);
        assert_eq!(ContentRef::from_arg(&[0xff, 0xfe]), None); // invalid UTF-8
    }
}
