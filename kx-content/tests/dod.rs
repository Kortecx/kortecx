//! P1.3 Definition-of-Done tests covering `content-store.md` §10 obligations 1–9.
//!
//! Each test is annotated with the obligation it satisfies.

use std::fs;
use std::sync::Arc;
use std::thread;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore, LocalFsContentStore, NotFound};

// ---------------------------------------------------------------------------
// Helpers — exercise the trait generically over an arbitrary backend.
// ---------------------------------------------------------------------------

fn round_trip_through<S: ContentStore>(store: &S, payload: &[u8]) -> ContentRef {
    let r = store.put(payload).expect("put must succeed");
    let got = store.get(&r).expect("get must succeed");
    assert_eq!(&got[..], payload, "get must return the bytes put");
    r
}

// ---------------------------------------------------------------------------
// Obligation 1 — round-trip put/get
// ---------------------------------------------------------------------------

#[test]
fn obligation_1a_round_trip_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();
    round_trip_through(&store, b"hello kortecx");
}

#[test]
fn obligation_1b_round_trip_in_memory() {
    let store = InMemoryContentStore::new();
    round_trip_through(&store, b"hello kortecx");
}

// ---------------------------------------------------------------------------
// Obligation 2 — auto-dedup
// ---------------------------------------------------------------------------

#[test]
fn obligation_2a_dedup_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();
    let r1 = store.put(b"same bytes").unwrap();
    let r2 = store.put(b"same bytes").unwrap();
    assert_eq!(r1, r2, "identical bytes must produce identical refs");
    let file_count = fs::read_dir(tmp.path())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_file())
        .count();
    assert_eq!(file_count, 1, "identical bytes must store one file");
}

#[test]
fn obligation_2b_dedup_in_memory() {
    let store = InMemoryContentStore::new();
    let r1 = store.put(b"same bytes").unwrap();
    let r2 = store.put(b"same bytes").unwrap();
    assert_eq!(r1, r2);
    assert_eq!(store.len(), 1, "identical bytes must share one entry");
}

// ---------------------------------------------------------------------------
// Obligation 3 — atomicity of put
// ---------------------------------------------------------------------------

#[test]
fn obligation_3_atomicity_no_observable_partial_write() {
    // A `NamedTempFile` created in the store's root but never persisted MUST not become
    // observable at any content-addressed ref. This models the failure mode: a worker
    // crashed between writing the temp and the atomic rename.
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();

    let payload = b"would-be content";
    let would_be_ref = ContentRef::of(payload);

    {
        // Create a temp in the store's root, write partial bytes, drop without persisting.
        let mut t = tempfile::NamedTempFile::new_in(tmp.path()).unwrap();
        use std::io::Write;
        t.write_all(b"would-be").unwrap();
        // Drop — tempfile crate removes the file.
    }

    assert!(
        !store.contains(&would_be_ref),
        "no observable object at the target ref after a non-persisted temp write"
    );
    assert!(
        store.get(&would_be_ref).is_err(),
        "get of would-be-ref returns NotFound"
    );

    // A subsequent normal put of the same bytes must still succeed cleanly.
    let r = store.put(payload).unwrap();
    assert_eq!(r, would_be_ref);
    assert!(store.contains(&r));
}

// ---------------------------------------------------------------------------
// Obligation 4 — get of unknown ref returns NotFound (no panic, no error escalation)
// ---------------------------------------------------------------------------

#[test]
fn obligation_4a_get_unknown_returns_notfound_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();
    let bogus = ContentRef::from_bytes([0xff; 32]);
    assert_eq!(store.get(&bogus), Err(NotFound));
}

#[test]
fn obligation_4b_get_after_delete_returns_notfound() {
    let store = InMemoryContentStore::new();
    let r = store.put(b"transient").unwrap();
    assert!(store.contains(&r));
    store.delete(&r).unwrap();
    assert_eq!(store.get(&r), Err(NotFound));
    // Same outcome as eviction by tiering — caller must be ready to handle this.
}

#[test]
fn obligation_4c_delete_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();
    let bogus = ContentRef::from_bytes([0xaa; 32]);
    // Deleting an absent ref is a no-op success.
    store.delete(&bogus).unwrap();
    store.delete(&bogus).unwrap();
}

// ---------------------------------------------------------------------------
// Obligation 5 — enumeration surface
// ---------------------------------------------------------------------------

#[test]
fn obligation_5_list_refs_enumerates_all() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();
    let mut put_refs: Vec<ContentRef> = (0..5u8)
        .map(|i| store.put(&[i, i, i, i]).unwrap())
        .collect();
    put_refs.sort();

    let mut listed: Vec<ContentRef> = store.list_refs().collect();
    listed.sort();
    assert_eq!(listed, put_refs);
}

#[test]
fn obligation_5b_list_refs_skips_stray_non_hex_files() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();

    let r = store.put(b"real object").unwrap();

    // Stray files in the root must not pollute the ref enumeration.
    fs::write(tmp.path().join("not-a-hash"), b"junk").unwrap();
    fs::write(tmp.path().join("UPPERCASE_HEX_NOT_OUR_FORMAT"), b"junk").unwrap();
    fs::write(tmp.path().join("0123"), b"too short").unwrap();

    let listed: Vec<ContentRef> = store.list_refs().collect();
    assert_eq!(
        listed,
        vec![r],
        "only real content-addressed objects are enumerated"
    );
}

// ---------------------------------------------------------------------------
// Obligation 6 — trait is backend-agnostic
// ---------------------------------------------------------------------------

/// Generic over the trait. If the trait carried in-process-specific assumptions in its
/// signature (e.g., concrete `bytes::Bytes` instead of an associated `Payload` type), this
/// function would not compile against two backends.
fn exercise_through_trait<S: ContentStore>(store: S) {
    let r = store.put(b"trait-only access").unwrap();
    let got = store.get(&r).unwrap();
    assert_eq!(&got[..], b"trait-only access");
    assert!(store.contains(&r));
    let listed: Vec<ContentRef> = store.list_refs().collect();
    assert_eq!(listed.len(), 1);
}

#[test]
fn obligation_6a_trait_works_for_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    exercise_through_trait(LocalFsContentStore::open(tmp.path()).unwrap());
}

#[test]
fn obligation_6b_trait_works_for_in_memory() {
    exercise_through_trait(InMemoryContentStore::new());
}

// ---------------------------------------------------------------------------
// Obligation 7 — backend-atomic put under concurrency
// ---------------------------------------------------------------------------

#[test]
fn obligation_7a_concurrent_puts_local_fs_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFsContentStore::open(tmp.path()).unwrap());

    let payload: Vec<u8> = (0..1024u32).flat_map(u32::to_le_bytes).collect();
    let payload_arc: Arc<Vec<u8>> = Arc::new(payload);

    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let payload = Arc::clone(&payload_arc);
        handles.push(thread::spawn(move || store.put(&payload).unwrap()));
    }
    let refs: Vec<ContentRef> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // All concurrent puts of identical bytes resolved to the same ref.
    let first = refs[0];
    assert!(
        refs.iter().all(|r| *r == first),
        "all concurrent puts must produce the same ref"
    );

    // Exactly one stored file on disk.
    let file_count = fs::read_dir(tmp.path())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_file())
        .count();
    assert_eq!(file_count, 1, "concurrent identical puts store one file");
}

#[test]
fn obligation_7b_concurrent_puts_in_memory_safe() {
    let store = Arc::new(InMemoryContentStore::new());
    let payload: Vec<u8> = (0..1024u32).flat_map(u32::to_le_bytes).collect();
    let payload_arc: Arc<Vec<u8>> = Arc::new(payload);

    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let payload = Arc::clone(&payload_arc);
        handles.push(thread::spawn(move || store.put(&payload).unwrap()));
    }
    let refs: Vec<ContentRef> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let first = refs[0];
    assert!(refs.iter().all(|r| *r == first));
    assert_eq!(store.len(), 1);
}

// ---------------------------------------------------------------------------
// Obligation 8 — GC walker integration (light)
// ---------------------------------------------------------------------------

#[test]
fn obligation_8_orphan_detection_via_set_difference() {
    // Models the walker: it asks the journal for the live ref set and the store for the
    // present ref set; orphans are (store \ live). The journal stub is just a HashSet.
    use std::collections::HashSet;

    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();

    let live = store.put(b"referenced by a committed entry").unwrap();
    let orphan_a = store.put(b"staged but never committed (a)").unwrap();
    let orphan_b = store.put(b"staged but never committed (b)").unwrap();

    let live_set: HashSet<ContentRef> = [live].into_iter().collect();
    let store_set: HashSet<ContentRef> = store.list_refs().collect();
    let orphans: HashSet<ContentRef> = store_set.difference(&live_set).copied().collect();

    let expected: HashSet<ContentRef> = [orphan_a, orphan_b].into_iter().collect();
    assert_eq!(orphans, expected);

    // Deletion is idempotent; running the walker twice is safe.
    for o in &orphans {
        store.delete(o).unwrap();
    }
    for o in &orphans {
        store.delete(o).unwrap();
    }
    let after: HashSet<ContentRef> = store.list_refs().collect();
    assert_eq!(after, live_set);
}

// ---------------------------------------------------------------------------
// Obligation 9 — Payload: Deref<Target=[u8]> discipline + zero-copy via Bytes
// ---------------------------------------------------------------------------

#[test]
fn obligation_9_payload_dereferences_as_byte_slice() {
    // The trait method returns `Self::Payload` which is `Deref<Target=[u8]>`. Both
    // backends use `bytes::Bytes`, which supports zero-copy slicing and ref-counted
    // sharing.
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();
    let payload: Vec<u8> = (0..256u32).flat_map(u32::to_le_bytes).collect();
    let r = store.put(&payload).unwrap();

    let bytes = store.get(&r).unwrap();
    // Slice into the payload without copying — Bytes supports zero-copy subviews.
    let head = bytes.slice(0..16);
    let tail = bytes.slice(bytes.len() - 16..);
    assert_eq!(&head[..], &payload[0..16]);
    assert_eq!(&tail[..], &payload[payload.len() - 16..]);

    // Hash the read bytes — must match the original ref (defensive audit; opt-in per
    // content-store.md §11, but cheap enough to do in a DoD test).
    let recomputed = ContentRef::of(&bytes);
    assert_eq!(recomputed, r);
}

// ---------------------------------------------------------------------------
// Extra coverage — ref formatting, dedup-via-different-instances
// ---------------------------------------------------------------------------

#[test]
fn content_ref_hex_round_trip() {
    let payload = b"any payload";
    let r = ContentRef::of(payload);
    let hex = r.to_hex();
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn dedup_across_distinct_store_instances_on_same_root() {
    // Two LocalFsContentStore handles pointing at the same directory share state — a
    // put through one is observable through the other. Real-world: multiple worker
    // processes / threads using the same backing directory.
    let tmp = tempfile::tempdir().unwrap();
    let s1 = LocalFsContentStore::open(tmp.path()).unwrap();
    let s2 = LocalFsContentStore::open(tmp.path()).unwrap();

    let r = s1.put(b"shared object").unwrap();
    assert!(s2.contains(&r));
    let got = s2.get(&r).unwrap();
    assert_eq!(&got[..], b"shared object");
}
