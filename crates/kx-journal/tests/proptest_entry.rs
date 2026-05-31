// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `JournalEntry` encode/decode + journal append round-trip
//! (SN-4 v2 #6).
//!
//! These pin the contracts that `journal-entry.md` (P0.11) defines for the
//! canonical byte layout — across the entire input space rather than a few
//! hand-picked entries.
//!
//! Properties asserted:
//!
//! 1. **Encode/decode round-trip** — for Proposed / Repudiated / Failed kinds,
//!    `decode_entry(encode_entry(e)) == e`. Committed has a non-canonical
//!    `mote_def_hash` field stored in a separate column, so it round-trips via
//!    `decode_entry_with_def_hash` instead.
//! 2. **Encoding is deterministic** — `encode_entry(e)` produces identical
//!    bytes on every call (the journal's byte-determinism guarantee).
//! 3. **Size cap** — `encode_entry(e).len() <= MAX_ENTRY_LEN` for any valid
//!    entry, including Committed entries near the 128-parent limit.
//! 4. **Append round-trip across backends** — `journal.append(e).seq` is
//!    monotonic, and `read_committed` returns the appended Committed entry.
//!    Both `InMemoryJournal` and `SqliteJournal` honor the property.

use kx_content::ContentRef;
use kx_journal::{
    decode_entry, decode_entry_with_def_hash, encode_entry, FailureReason, InMemoryJournal,
    Journal, JournalEntry, ParentEntry, RepudiationReason, SqliteJournal, INSTANCE_ID_LEN,
    MAX_ENTRY_LEN, MAX_PARENTS,
};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Strategies — generate arbitrary valid entries
// ---------------------------------------------------------------------------

fn arb_byte_array_32() -> impl Strategy<Value = [u8; 32]> {
    proptest::array::uniform32(any::<u8>())
}

fn arb_mote_id() -> impl Strategy<Value = MoteId> {
    arb_byte_array_32().prop_map(MoteId::from_bytes)
}

fn arb_nd_class() -> impl Strategy<Value = NdClass> {
    prop_oneof![
        Just(NdClass::Pure),
        Just(NdClass::ReadOnlyNondet),
        Just(NdClass::WorldMutating),
    ]
}

fn arb_repudiation_reason() -> impl Strategy<Value = RepudiationReason> {
    prop_oneof![
        Just(RepudiationReason::OperatorAction),
        Just(RepudiationReason::CriticInvalidated),
        Just(RepudiationReason::DefinitionLevelRepudiation),
        Just(RepudiationReason::UpstreamCascade),
        Just(RepudiationReason::SafetyInvariantBreach),
        Just(RepudiationReason::ExternalSystemReportedFailure),
    ]
}

fn arb_failure_reason() -> impl Strategy<Value = FailureReason> {
    prop_oneof![
        Just(FailureReason::TimedOut),
        Just(FailureReason::ExecutorRefused),
        Just(FailureReason::ValidatorRejected),
        Just(FailureReason::WorkerCrashed),
        Just(FailureReason::UpstreamRepudiated),
        Just(FailureReason::UnsafeWorldMutatingConstruction),
    ]
}

/// Arbitrary `ParentEntry`. Encoded constraint: when `edge_kind == 0` (Data),
/// `non_cascade` MUST be 0 (anti-pattern in §11 of journal-entry.md). The
/// strategy generates only valid pairs so the encoder never rejects them.
fn arb_parent_entry() -> impl Strategy<Value = ParentEntry> {
    (arb_byte_array_32(), 0u8..=1, any::<bool>()).prop_map(|(id, edge_kind, nc_for_control)| {
        // Data edges (edge_kind=0): non_cascade MUST be 0.
        // Control edges (edge_kind=1): non_cascade is free (0 or 1).
        let non_cascade = if edge_kind == 0 {
            0
        } else {
            u8::from(nc_for_control)
        };
        ParentEntry {
            parent_id: MoteId::from_bytes(id),
            edge_kind,
            non_cascade,
        }
    })
}

/// Bounded parent vector — up to MAX_PARENTS to exercise the size cap.
fn arb_parents() -> impl Strategy<Value = SmallVec<[ParentEntry; 4]>> {
    proptest::collection::vec(arb_parent_entry(), 0..=MAX_PARENTS).prop_map(SmallVec::from_vec)
}

fn arb_proposed() -> impl Strategy<Value = JournalEntry> {
    (
        arb_mote_id(),
        arb_byte_array_32(),
        any::<u64>(),
        arb_nd_class(),
        any::<u128>(),
        arb_byte_array_32(),
    )
        .prop_map(
            |(mote_id, idempotency_key, seq, nondeterminism, placement_hint, warrant_ref_bytes)| {
                JournalEntry::Proposed {
                    mote_id,
                    idempotency_key,
                    seq,
                    nondeterminism,
                    placement_hint,
                    warrant_ref: ContentRef::from_bytes(warrant_ref_bytes),
                }
            },
        )
}

fn arb_committed() -> impl Strategy<Value = JournalEntry> {
    (
        arb_mote_id(),
        arb_byte_array_32(),
        any::<u64>(),
        arb_nd_class(),
        arb_byte_array_32(),
        arb_parents(),
        arb_byte_array_32(),
        arb_byte_array_32(),
    )
        .prop_map(
            |(
                mote_id,
                idempotency_key,
                seq,
                nondeterminism,
                ref_bytes,
                parents,
                warrant_ref_bytes,
                def_hash,
            )| {
                JournalEntry::Committed {
                    mote_id,
                    idempotency_key,
                    seq,
                    nondeterminism,
                    result_ref: ContentRef::from_bytes(ref_bytes),
                    parents,
                    warrant_ref: ContentRef::from_bytes(warrant_ref_bytes),
                    mote_def_hash: MoteDefHash::from_bytes(def_hash),
                }
            },
        )
}

/// **v3 (M1.1): RunRegistered strategy.** `instance_id` is the 16-byte run nonce;
/// `recipe_fingerprint` is a 32-byte discovery/dedup hash; `ts` is audit-only.
fn arb_run_registered() -> impl Strategy<Value = JournalEntry> {
    (
        proptest::array::uniform16(any::<u8>()),
        arb_byte_array_32(),
        any::<u64>(),
        any::<u64>(),
    )
        .prop_map(|(instance_id, recipe_fingerprint, ts, seq)| {
            // The strategy already produces a 16-byte array; pin it to
            // INSTANCE_ID_LEN so a future length change fails to compile here.
            let _: [u8; INSTANCE_ID_LEN] = instance_id;
            JournalEntry::RunRegistered {
                instance_id,
                recipe_fingerprint,
                ts,
                seq,
            }
        })
}

/// **v4 (M1.2): ResolvedKindTag strategy** — one of the five closed variants.
fn arb_resolved_kind() -> impl Strategy<Value = kx_journal::ResolvedKindTag> {
    use kx_journal::ResolvedKindTag::{Builtin, External, LocalScript, Mcp, SelfGenerated};
    prop_oneof![
        Just(Builtin),
        Just(LocalScript),
        Just(External),
        Just(Mcp),
        Just(SelfGenerated),
    ]
}

/// **v4 (M1.2): RunVersionsResolved strategy.** `instance_id` is the 16-byte run
/// nonce; `model_id`/`tool_id`/`tool_version` are bounded UTF-8 ids; the
/// capability is present or absent (zero-grant warrant).
fn arb_run_versions_resolved() -> impl Strategy<Value = JournalEntry> {
    let arb_cap = (
        "[a-z0-9._-]{0,40}",
        "[a-z0-9._-]{0,16}",
        arb_resolved_kind(),
        arb_byte_array_32(),
    )
        .prop_map(|(tool_id, tool_version, resolved_kind, def_hash)| {
            kx_journal::ResolvedCapabilityRecord {
                tool_id,
                tool_version,
                resolved_kind,
                resolved_def_hash: ContentRef::from_bytes(def_hash),
            }
        });
    (
        proptest::array::uniform16(any::<u8>()),
        arb_byte_array_32(),
        "[a-z0-9._-]{0,40}",
        proptest::option::of(arb_cap),
        any::<u64>(),
    )
        .prop_map(|(instance_id, warrant_ref, model_id, capability, seq)| {
            let _: [u8; INSTANCE_ID_LEN] = instance_id;
            JournalEntry::RunVersionsResolved {
                instance_id,
                warrant_ref: ContentRef::from_bytes(warrant_ref),
                model_id,
                capability,
                seq,
            }
        })
}

/// **v2 (PR 7): EffectStaged strategy.** Header-only; no body fields.
fn arb_effect_staged() -> impl Strategy<Value = JournalEntry> {
    (arb_mote_id(), arb_byte_array_32(), any::<u64>()).prop_map(
        |(mote_id, idempotency_key, seq)| JournalEntry::EffectStaged {
            mote_id,
            idempotency_key,
            seq,
        },
    )
}

fn arb_repudiated() -> impl Strategy<Value = JournalEntry> {
    (
        arb_mote_id(),
        arb_byte_array_32(),
        any::<u64>(),
        any::<u64>(),
        arb_repudiation_reason(),
        any::<u128>(),
    )
        .prop_map(
            |(
                target_mote_id,
                idempotency_key,
                seq,
                target_committed_seq,
                reason_class,
                repudiator_id,
            )| {
                JournalEntry::Repudiated {
                    target_mote_id,
                    idempotency_key,
                    seq,
                    target_committed_seq,
                    reason_class,
                    repudiator_id,
                }
            },
        )
}

fn arb_failed() -> impl Strategy<Value = JournalEntry> {
    (
        arb_mote_id(),
        arb_byte_array_32(),
        any::<u64>(),
        arb_failure_reason(),
        any::<u128>(),
    )
        .prop_map(
            |(mote_id, idempotency_key, seq, reason_class, reporter_id)| JournalEntry::Failed {
                mote_id,
                idempotency_key,
                seq,
                reason_class,
                reporter_id,
            },
        )
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    // Property 1 — encode/decode round-trip for Proposed entries.
    #[test]
    fn prop_proposed_round_trip(entry in arb_proposed()) {
        let bytes = encode_entry(&entry).expect("encode");
        let decoded = decode_entry(&bytes).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // Property 1 — encode/decode round-trip for Repudiated entries.
    #[test]
    fn prop_repudiated_round_trip(entry in arb_repudiated()) {
        let bytes = encode_entry(&entry).expect("encode");
        let decoded = decode_entry(&bytes).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // Property 1 — encode/decode round-trip for Failed entries.
    #[test]
    fn prop_failed_round_trip(entry in arb_failed()) {
        let bytes = encode_entry(&entry).expect("encode");
        let decoded = decode_entry(&bytes).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // **v2 (PR 7)** — encode/decode round-trip for EffectStaged entries
    // (header-only body per D38 §2b).
    #[test]
    fn prop_effect_staged_round_trip(entry in arb_effect_staged()) {
        let bytes = encode_entry(&entry).expect("encode");
        // EffectStaged is header-only — bytes.len() MUST equal HEADER_LEN.
        prop_assert_eq!(bytes.len(), kx_journal::HEADER_LEN);
        let decoded = decode_entry(&bytes).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // **v2 (PR 7)** — encoding determinism for EffectStaged.
    #[test]
    fn prop_encoding_is_deterministic_effect_staged(entry in arb_effect_staged()) {
        let a = encode_entry(&entry).expect("encode a");
        let b = encode_entry(&entry).expect("encode b");
        prop_assert_eq!(a, b);
    }

    // **v3 (M1.1)** — encode/decode round-trip for RunRegistered entries.
    // Fixed 130-byte size (74 header + 56 body).
    #[test]
    fn prop_run_registered_round_trip(entry in arb_run_registered()) {
        let bytes = encode_entry(&entry).expect("encode");
        prop_assert_eq!(bytes.len(), 130);
        let decoded = decode_entry(&bytes).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // **v3 (M1.1)** — encoding determinism for RunRegistered.
    #[test]
    fn prop_encoding_is_deterministic_run_registered(entry in arb_run_registered()) {
        let a = encode_entry(&entry).expect("encode a");
        let b = encode_entry(&entry).expect("encode b");
        prop_assert_eq!(a, b);
    }

    // **v3 (M1.1)** — size cap holds for RunRegistered.
    #[test]
    fn prop_size_cap_run_registered(entry in arb_run_registered()) {
        let bytes = encode_entry(&entry).expect("encode");
        prop_assert!(
            bytes.len() <= MAX_ENTRY_LEN,
            "RunRegistered encoded to {} bytes; cap is {}",
            bytes.len(),
            MAX_ENTRY_LEN
        );
    }

    // **v4 (M1.2)** — encode/decode round-trip for RunVersionsResolved entries
    // (variable-length body; capability present or absent).
    #[test]
    fn prop_run_versions_round_trip(entry in arb_run_versions_resolved()) {
        let bytes = encode_entry(&entry).expect("encode");
        let decoded = decode_entry(&bytes).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // **v4 (M1.2)** — encoding determinism + size cap for RunVersionsResolved.
    #[test]
    fn prop_run_versions_deterministic_and_capped(entry in arb_run_versions_resolved()) {
        let a = encode_entry(&entry).expect("encode a");
        let b = encode_entry(&entry).expect("encode b");
        prop_assert_eq!(&a, &b);
        prop_assert!(a.len() <= MAX_ENTRY_LEN);
    }

    // Property 1 — encode/decode round-trip for Committed entries via
    // `decode_entry_with_def_hash` (mote_def_hash is non-canonical metadata,
    // stored in a separate column, supplied to the decoder out-of-band).
    #[test]
    fn prop_committed_round_trip_with_def_hash(entry in arb_committed()) {
        let bytes = encode_entry(&entry).expect("encode");
        let def_hash = match &entry {
            JournalEntry::Committed { mote_def_hash, .. } => *mote_def_hash,
            _ => unreachable!("strategy produces only Committed"),
        };
        let decoded = decode_entry_with_def_hash(&bytes, def_hash).expect("decode");
        prop_assert_eq!(decoded, entry);
    }

    // Property 2 — encoding is byte-deterministic.
    #[test]
    fn prop_encoding_is_deterministic_proposed(entry in arb_proposed()) {
        let a = encode_entry(&entry).expect("encode a");
        let b = encode_entry(&entry).expect("encode b");
        prop_assert_eq!(a, b);
    }

    #[test]
    fn prop_encoding_is_deterministic_committed(entry in arb_committed()) {
        let a = encode_entry(&entry).expect("encode a");
        let b = encode_entry(&entry).expect("encode b");
        prop_assert_eq!(a, b);
    }

    // Property 3 — size cap holds for every entry, including 128-parent Committed.
    #[test]
    fn prop_size_cap_proposed(entry in arb_proposed()) {
        let bytes = encode_entry(&entry).expect("encode");
        prop_assert!(
            bytes.len() <= MAX_ENTRY_LEN,
            "Proposed encoded to {} bytes; cap is {}",
            bytes.len(),
            MAX_ENTRY_LEN
        );
    }

    #[test]
    fn prop_size_cap_committed(entry in arb_committed()) {
        let bytes = encode_entry(&entry).expect("encode");
        prop_assert!(
            bytes.len() <= MAX_ENTRY_LEN,
            "Committed encoded to {} bytes; cap is {}",
            bytes.len(),
            MAX_ENTRY_LEN
        );
    }

    #[test]
    fn prop_size_cap_repudiated(entry in arb_repudiated()) {
        let bytes = encode_entry(&entry).expect("encode");
        prop_assert!(
            bytes.len() <= MAX_ENTRY_LEN,
            "Repudiated encoded to {} bytes; cap is {}",
            bytes.len(),
            MAX_ENTRY_LEN
        );
    }

    #[test]
    fn prop_size_cap_failed(entry in arb_failed()) {
        let bytes = encode_entry(&entry).expect("encode");
        prop_assert!(
            bytes.len() <= MAX_ENTRY_LEN,
            "Failed encoded to {} bytes; cap is {}",
            bytes.len(),
            MAX_ENTRY_LEN
        );
    }

    // Property 4 — append + read_committed round-trip across both backends.
    // Each test case uses a fresh journal so the per-run `seq` always starts at 1.
    #[test]
    fn prop_in_memory_committed_round_trip(entry in arb_committed()) {
        let journal = InMemoryJournal::new();
        let stored = journal.append(entry.clone()).expect("append");
        // Append assigns seq=1 to the first entry; the input's seq is ignored.
        prop_assert_eq!(stored.seq(), 1);

        let mote_id = stored.mote_id();
        let read = journal
            .read_committed(&mote_id)
            .expect("read_committed")
            .expect("Some");
        prop_assert_eq!(read, stored);
    }

    #[test]
    fn prop_sqlite_committed_round_trip(entry in arb_committed()) {
        let journal = SqliteJournal::open_in_memory().expect("open");
        let stored = journal.append(entry.clone()).expect("append");
        prop_assert_eq!(stored.seq(), 1);

        let mote_id = stored.mote_id();
        let read = journal
            .read_committed(&mote_id)
            .expect("read_committed")
            .expect("Some");
        prop_assert_eq!(read, stored);
    }
}

// ---------------------------------------------------------------------------
// Compile-time + reader-writer interleaving tests (SN-4 v2 #7 extension)
// ---------------------------------------------------------------------------

/// `Journal` impls must be `Send + Sync` so the executor can share a single
/// journal handle across worker threads (`journal-txn.md` §7 — single-writer-
/// per-run; multiple readers).
#[test]
fn both_journal_impls_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InMemoryJournal>();
    assert_send_sync::<SqliteJournal>();
}

/// One writer thread + one reader thread, interleaved against the same
/// `Arc<InMemoryJournal>`. Existing `dod.rs` covers concurrent writers; this
/// adds the reader-while-writer pattern to verify a reader never observes a
/// torn entry (the writer's append is atomic per `journal-txn.md` §6).
#[test]
fn reader_never_observes_partial_entry() {
    use std::sync::Arc;
    use std::thread;

    let journal = Arc::new(InMemoryJournal::new());

    let writer_journal = Arc::clone(&journal);
    let writer = thread::spawn(move || {
        for i in 0..50u8 {
            let entry = JournalEntry::Failed {
                mote_id: MoteId::from_bytes([i; 32]),
                idempotency_key: [i; 32],
                seq: 0, // assigned by journal
                reason_class: FailureReason::TimedOut,
                reporter_id: u128::from(i),
            };
            writer_journal.append(entry).expect("writer append");
        }
    });

    let reader_journal = Arc::clone(&journal);
    let reader = thread::spawn(move || {
        // Spin-read; any entry observed must round-trip its byte encoding
        // (proves the entry is a complete, valid object — never partial).
        let mut max_seen: u64 = 0;
        for _ in 0..200 {
            let seq = reader_journal.current_seq().expect("current_seq");
            assert!(seq >= max_seen, "current_seq regressed: {seq} < {max_seen}");
            max_seen = seq;
            // Read every entry committed so far — each must encode + decode
            // without error (atomicity property).
            let entries: Vec<_> = reader_journal
                .read_entries_by_seq(1..(seq + 1))
                .expect("read_entries_by_seq")
                .collect();
            for e in &entries {
                let bytes = encode_entry(e).expect("encode visible entry");
                let decoded = decode_entry(&bytes).expect("decode visible entry");
                assert_eq!(decoded, *e, "round-trip mismatch — entry torn?");
            }
        }
    });

    writer.join().expect("writer panic");
    reader.join().expect("reader panic");

    // Final state: writer wrote 50 entries; journal reflects them.
    assert_eq!(journal.count_entries().expect("count"), 50);
}
