//! Property tests for the `ContentStore` contract (SN-4 v2 #6).
//!
//! The store's job is to be a content-addressed, idempotent, atomic-per-object
//! key-value layer. The properties asserted here pin that contract over the
//! arbitrary-byte-slice input space, not just hand-picked test cases.
//!
//! Both the [`InMemoryContentStore`] and [`LocalFsContentStore`] backends are
//! exercised against the same property set — the trait abstraction is the
//! point, so any contract violation must surface on both backends.
//!
//! Properties:
//!
//!  1. **Content-addressed identity.** `ContentRef::of(b) == put(b).unwrap()`.
//!     Refs are derived from bytes alone, never from clock / counter / address.
//!  2. **Round-trip preservation.** `get(put(b))` returns exactly `b`. Stores
//!     never silently transform payloads.
//!  3. **Idempotency.** `put(b)` returns the same ref twice; the second put
//!     does not create a new on-disk object (`len` stays at 1 for in-memory;
//!     filesystem has exactly 1 file).
//!  4. **Distinct inputs → distinct refs.** Two byte slices `b1 != b2` produce
//!     `put(b1) != put(b2)`. This is BLAKE3's collision resistance pinned by a
//!     property; the cost is one BLAKE3 evaluation per case.
//!  5. **Delete idempotency.** `delete` succeeds on an absent ref;
//!     `delete(r); contains(r) == false`; `delete(r); delete(r)` is a no-op.

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore, LocalFsContentStore};
use proptest::prelude::*;
use tempfile::TempDir;

/// Strategy: arbitrary byte slices of length 0 to 4 KiB. Sized to be fast
/// enough that 64 cases run in under a second while still covering the empty,
/// single-byte, ASCII, and binary-noise inputs the store will see in
/// production.
fn byte_slice() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=4096)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    // ---- Property 1: content-addressed identity (both backends) ------------

    #[test]
    fn prop_in_memory_content_addressed_identity(bytes in byte_slice()) {
        let store = InMemoryContentStore::new();
        let r = store.put(&bytes).expect("put");
        prop_assert_eq!(r, ContentRef::of(&bytes));
    }

    #[test]
    fn prop_local_fs_content_addressed_identity(bytes in byte_slice()) {
        let tmp = TempDir::new().unwrap();
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let r = store.put(&bytes).expect("put");
        prop_assert_eq!(r, ContentRef::of(&bytes));
    }

    // ---- Property 2: round-trip preservation -------------------------------

    #[test]
    fn prop_in_memory_round_trip(bytes in byte_slice()) {
        let store = InMemoryContentStore::new();
        let r = store.put(&bytes).expect("put");
        let got = store.get(&r).expect("get");
        prop_assert_eq!(&*got, bytes.as_slice());
    }

    #[test]
    fn prop_local_fs_round_trip(bytes in byte_slice()) {
        let tmp = TempDir::new().unwrap();
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let r = store.put(&bytes).expect("put");
        let got = store.get(&r).expect("get");
        prop_assert_eq!(&*got, bytes.as_slice());
    }

    // ---- Property 3: idempotency ------------------------------------------

    #[test]
    fn prop_in_memory_put_is_idempotent(bytes in byte_slice()) {
        let store = InMemoryContentStore::new();
        let r1 = store.put(&bytes).expect("put 1");
        let r2 = store.put(&bytes).expect("put 2");
        prop_assert_eq!(r1, r2);
        prop_assert_eq!(store.len(), 1);
    }

    #[test]
    fn prop_local_fs_put_is_idempotent(bytes in byte_slice()) {
        let tmp = TempDir::new().unwrap();
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let r1 = store.put(&bytes).expect("put 1");
        let r2 = store.put(&bytes).expect("put 2");
        prop_assert_eq!(r1, r2);
        // Exactly one file on disk under the root for this distinct payload.
        let n_files = std::fs::read_dir(tmp.path())
            .expect("read_dir")
            .filter(|e| e.as_ref().map(|e| e.path().is_file()).unwrap_or(false))
            .count();
        prop_assert_eq!(n_files, 1);
    }

    // ---- Property 4: distinct inputs → distinct refs ----------------------

    #[test]
    fn prop_distinct_bytes_distinct_refs(
        b1 in byte_slice(),
        b2 in byte_slice(),
    ) {
        // Only assert the property when inputs actually differ — the
        // proptest framework can produce identical pairs.
        prop_assume!(b1 != b2);
        let r1 = ContentRef::of(&b1);
        let r2 = ContentRef::of(&b2);
        prop_assert_ne!(
            r1, r2,
            "BLAKE3 collision between two random byte slices — \
             either an astronomically unlikely event or a hash regression"
        );
    }

    // ---- Property 5: delete idempotency -----------------------------------

    #[test]
    fn prop_in_memory_delete_is_idempotent(bytes in byte_slice()) {
        let store = InMemoryContentStore::new();
        let r = store.put(&bytes).expect("put");
        prop_assert!(store.contains(&r));
        store.delete(&r).expect("delete 1");
        prop_assert!(!store.contains(&r));
        store.delete(&r).expect("delete 2 (absent)");
        prop_assert!(!store.contains(&r));
    }

    #[test]
    fn prop_local_fs_delete_is_idempotent(bytes in byte_slice()) {
        let tmp = TempDir::new().unwrap();
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let r = store.put(&bytes).expect("put");
        prop_assert!(store.contains(&r));
        store.delete(&r).expect("delete 1");
        prop_assert!(!store.contains(&r));
        store.delete(&r).expect("delete 2 (absent)");
        prop_assert!(!store.contains(&r));
    }
}

// ---------------------------------------------------------------------------
// SN-4 v2 #7 — concurrency: prove the existing claim that both stores are
// `Send + Sync` and idempotent under concurrent puts.
// ---------------------------------------------------------------------------

/// Compile-time `Send + Sync` assertion for both stores. If a future refactor
/// drops thread-safety, this stops compiling at build time.
#[test]
fn both_stores_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InMemoryContentStore>();
    assert_send_sync::<LocalFsContentStore>();
}

/// 8 threads concurrently `put` the same payload into the same store. The
/// store must return a single distinct ref and contain exactly one object.
///
/// The existing `kx-content/tests/dod.rs` covers idempotent concurrent put
/// over both backends; this is a tighter wrapper-test that additionally
/// asserts `list_refs` count and that the on-disk file count matches the
/// distinct-ref count (catches a "double-write" bug that the original test
/// could miss).
#[test]
fn concurrent_identical_puts_yield_single_ref_and_single_object() {
    let tmp = TempDir::new().unwrap();
    let store: Arc<LocalFsContentStore> =
        Arc::new(LocalFsContentStore::open(tmp.path()).expect("open"));

    let payload = b"the same bytes from every thread";
    let mut handles = Vec::new();
    for _ in 0..8 {
        let s = Arc::clone(&store);
        handles.push(std::thread::spawn(move || s.put(payload).expect("put")));
    }
    let refs: Vec<ContentRef> = handles
        .into_iter()
        .map(|h| h.join().expect("thread panic"))
        .collect();

    // All 8 threads got the same ref.
    let first = refs[0];
    for (i, r) in refs.iter().enumerate() {
        assert_eq!(
            *r, first,
            "thread {i} got a different ref than thread 0 — content addressing failed"
        );
    }

    // The store reports a single ref.
    let listed: Vec<ContentRef> = store.list_refs().collect();
    assert_eq!(listed.len(), 1, "store should hold exactly one ref");
    assert_eq!(listed[0], first);

    // The on-disk file count matches.
    let n_files = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter(|e| e.as_ref().map(|e| e.path().is_file()).unwrap_or(false))
        .count();
    assert_eq!(
        n_files, 1,
        "on-disk file count must match distinct-ref count (got {n_files})"
    );
}
