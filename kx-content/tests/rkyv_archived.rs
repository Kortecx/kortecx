//! P1.3 exit gate: "rkyv archived access works on a large payload."
//!
//! `content-store.md` §4 + §10 obligation 9 are explicit that rkyv is NOT in the trait
//! signature — it lives entirely on the caller side. This test demonstrates the
//! composition: the workflow author rkyv-archives a typed payload, stores the resulting
//! bytes in the content store, retrieves them, and reads the archived view zero-copy
//! (without a deserialization pass) through the `Bytes` payload's deref to `[u8]`.
//!
//! The content store sees opaque bytes throughout. The rkyv encoding/decoding is the
//! caller's concern, exactly as the spec requires.

use kx_content::{ContentStore, LocalFsContentStore};
use rkyv::rancor;

/// One megabyte. Large enough to make a non-zero-copy deserialization noticeably wasteful;
/// small enough to keep the test fast.
const LARGE_LEN: usize = 1_000_000;

#[test]
fn rkyv_archived_view_round_trips_through_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();

    // Workflow author's typed payload: a Vec<u8> of a million bytes. rkyv already provides
    // Archive / Serialize / Deserialize impls for Vec<u8>, so we don't need a custom derive.
    let original: Vec<u8> = (0..LARGE_LEN).map(|i| (i % 251) as u8).collect();

    // Caller-side rkyv archiving — completely opaque to the store.
    let archived_bytes =
        rkyv::to_bytes::<rancor::Error>(&original).expect("rkyv archive must succeed");

    // Store opaque bytes; the store is rkyv-blind.
    let r = store.put(&archived_bytes).expect("put must succeed");

    // Retrieve. The store returns its Payload (a `bytes::Bytes` for the local FS backend).
    let payload = store.get(&r).expect("get must succeed");
    assert_eq!(payload.len(), archived_bytes.len(), "byte length preserved");

    // Zero-copy archived access: cast the retrieved bytes to `&Archived<Vec<u8>>` without
    // a deserialization pass. rkyv's `access` validates the buffer is well-formed.
    let archived = rkyv::access::<rkyv::Archived<Vec<u8>>, rancor::Error>(&payload[..])
        .expect("archived access must succeed");

    // Spot-check fields without ever deserializing the full vector.
    assert_eq!(archived.len(), LARGE_LEN);
    assert_eq!(archived[0], 0u8);
    assert_eq!(archived[1], 1u8);
    assert_eq!(archived[250], 250u8);
    assert_eq!(archived[251], 0u8);
    assert_eq!(archived[LARGE_LEN - 1], ((LARGE_LEN - 1) % 251) as u8);
}

#[test]
fn rkyv_dedupes_byte_identical_payloads() {
    // Two identical typed payloads serialize to identical bytes (rkyv is canonical for
    // primitive vectors). The content store dedupes them by structure.
    let tmp = tempfile::tempdir().unwrap();
    let store = LocalFsContentStore::open(tmp.path()).unwrap();

    let payload: Vec<u32> = (0..10_000).collect();
    let bytes_a = rkyv::to_bytes::<rancor::Error>(&payload).unwrap();
    let bytes_b = rkyv::to_bytes::<rancor::Error>(&payload).unwrap();
    assert_eq!(
        &bytes_a[..],
        &bytes_b[..],
        "rkyv canonical for primitive vec"
    );

    let r1 = store.put(&bytes_a).unwrap();
    let r2 = store.put(&bytes_b).unwrap();
    assert_eq!(r1, r2);
}
