// Integration-test file: compiled as a separate crate from the host lib.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for schema migration (IMP-2, M2.x-E).
//!
//! Two families:
//!
//! - **P1 — current-version identity round-trip** (`decode∘encode == id`) for the
//!   kinds/variants the existing `proptest_entry.rs` does not cover:
//!   `RunVersionsResolved` (arbitrary `idempotency_class` + strings + optional
//!   capability), `DigestSealed`, and `Failed` over the two v6 `FailureReason`
//!   variants. This is the corpus's `encode∘decode==id` fuzz.
//!
//! - **P2 — versioned-decode correctness.** For an arbitrary *v5-shaped* entry
//!   (current bytes minus the trailing capability class byte), [`migrate_entry`]
//!   (1) defaults an absent class to the safe `AtLeastOnce`, (2) equals exactly
//!   "append the default byte, then decode" — the migration mechanism is a pure
//!   transform composed with the canonical decoder, so it generalizes to future
//!   ladder links — and (3) is a fixed point on already-current bytes (idempotent).

use kx_content::ContentRef;
use kx_journal::{
    decode_entry, decode_entry_with_def_hash, encode_entry, migrate_entry, FailureReason,
    IdempotencyClassTag, JournalEntry, ResolvedCapabilityRecord, ResolvedKindTag, INSTANCE_ID_LEN,
    JOURNAL_SCHEMA_VERSION, V5_ABSENT_IDEMPOTENCY_CLASS,
};
use kx_mote::MoteDefHash;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_byte_array_32() -> impl Strategy<Value = [u8; 32]> {
    proptest::array::uniform32(any::<u8>())
}

fn arb_instance_id() -> impl Strategy<Value = [u8; INSTANCE_ID_LEN]> {
    proptest::array::uniform16(any::<u8>())
}

fn arb_short_str() -> impl Strategy<Value = String> {
    // Bounded, valid UTF-8; covers empty and typical identifiers.
    proptest::string::string_regex("[a-zA-Z0-9._/-]{0,32}").unwrap()
}

fn arb_idempotency_class() -> impl Strategy<Value = IdempotencyClassTag> {
    prop_oneof![
        Just(IdempotencyClassTag::Token),
        Just(IdempotencyClassTag::Readback),
        Just(IdempotencyClassTag::Staged),
        Just(IdempotencyClassTag::AtLeastOnce),
    ]
}

fn arb_resolved_kind() -> impl Strategy<Value = ResolvedKindTag> {
    prop_oneof![
        Just(ResolvedKindTag::Builtin),
        Just(ResolvedKindTag::LocalScript),
        Just(ResolvedKindTag::External),
        Just(ResolvedKindTag::Mcp),
        Just(ResolvedKindTag::SelfGenerated),
    ]
}

fn arb_capability() -> impl Strategy<Value = ResolvedCapabilityRecord> {
    (
        arb_short_str(),
        arb_short_str(),
        arb_resolved_kind(),
        arb_byte_array_32(),
        arb_idempotency_class(),
    )
        .prop_map(
            |(tool_id, tool_version, resolved_kind, def_hash, idempotency_class)| {
                ResolvedCapabilityRecord {
                    tool_id,
                    tool_version,
                    resolved_kind,
                    resolved_def_hash: ContentRef::from_bytes(def_hash),
                    idempotency_class,
                }
            },
        )
}

fn arb_run_versions_resolved() -> impl Strategy<Value = JournalEntry> {
    (
        arb_instance_id(),
        arb_byte_array_32(),
        arb_short_str(),
        proptest::option::of(arb_capability()),
        any::<u64>(),
    )
        .prop_map(|(instance_id, warrant_ref, model_id, capability, seq)| {
            JournalEntry::RunVersionsResolved {
                instance_id,
                warrant_ref: ContentRef::from_bytes(warrant_ref),
                model_id,
                capability,
                seq,
            }
        })
}

fn arb_digest_sealed() -> impl Strategy<Value = JournalEntry> {
    (any::<u64>(), arb_byte_array_32(), any::<u64>()).prop_map(|(through_seq, digest, seq)| {
        JournalEntry::DigestSealed {
            through_seq,
            state_digest: digest,
            seq,
        }
    })
}

fn arb_failed_v6() -> impl Strategy<Value = JournalEntry> {
    let reason = prop_oneof![
        Just(FailureReason::CompensatedAtLeastOnce),
        Just(FailureReason::QuarantinedAtLeastOnce),
    ];
    (
        arb_byte_array_32(),
        arb_byte_array_32(),
        reason,
        any::<u128>(),
    )
        .prop_map(
            |(mote_id, key, reason_class, reporter_id)| JournalEntry::Failed {
                mote_id: kx_mote::MoteId::from_bytes(mote_id),
                idempotency_key: key,
                seq: 0,
                reason_class,
                reporter_id,
            },
        )
}

const ZERO_DEF_HASH: fn() -> MoteDefHash = || MoteDefHash::from_bytes([0u8; 32]);

/// Derive v5-shaped bytes from a current entry: if it's a capability-present
/// `RunVersionsResolved`, drop the trailing `idempotency_class` byte (the lone
/// v5→v6 delta); otherwise the bytes are already v5-valid.
fn to_v5_bytes(entry: &JournalEntry) -> Vec<u8> {
    let mut bytes = encode_entry(entry).unwrap();
    if matches!(
        entry,
        JournalEntry::RunVersionsResolved {
            capability: Some(_),
            ..
        }
    ) {
        bytes.pop();
    }
    bytes
}

// ---------------------------------------------------------------------------
// P1 — current-version identity round-trips
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn prop_run_versions_resolved_round_trips(entry in arb_run_versions_resolved()) {
        let bytes = encode_entry(&entry).unwrap();
        let decoded = decode_entry(&bytes).unwrap();
        prop_assert_eq!(decoded, entry);
    }

    #[test]
    fn prop_digest_sealed_round_trips(entry in arb_digest_sealed()) {
        let bytes = encode_entry(&entry).unwrap();
        let decoded = decode_entry(&bytes).unwrap();
        prop_assert_eq!(decoded, entry);
    }

    #[test]
    fn prop_failed_v6_variants_round_trip(entry in arb_failed_v6()) {
        let bytes = encode_entry(&entry).unwrap();
        let decoded = decode_entry(&bytes).unwrap();
        prop_assert_eq!(decoded, entry);
    }

    // ---------------------------------------------------------------------
    // P2 — versioned-decode correctness
    // ---------------------------------------------------------------------

    #[test]
    fn prop_v5_capability_defaults_to_safe_class(entry in arb_run_versions_resolved()) {
        // Only meaningful when a capability is present.
        prop_assume!(matches!(entry, JournalEntry::RunVersionsResolved { capability: Some(_), .. }));
        let v5 = to_v5_bytes(&entry);
        let migrated = migrate_entry(&v5, 5, ZERO_DEF_HASH()).unwrap();
        match migrated {
            JournalEntry::RunVersionsResolved { capability: Some(cap), .. } => {
                prop_assert_eq!(cap.idempotency_class, V5_ABSENT_IDEMPOTENCY_CLASS);
                prop_assert_eq!(cap.idempotency_class, IdempotencyClassTag::AtLeastOnce);
            }
            other => prop_assert!(false, "expected cap-present RunVersionsResolved, got {:?}", other),
        }
    }

    #[test]
    fn prop_v5_upconvert_equals_append_default_then_decode(entry in arb_run_versions_resolved()) {
        // The migration mechanism is exactly "append the safe-default byte to a
        // capability-present body, then run the canonical decoder" — a pure
        // transform composed with decode. This is the invariant that lets the
        // ladder generalize to future links.
        let v5 = to_v5_bytes(&entry);
        let migrated = migrate_entry(&v5, 5, ZERO_DEF_HASH()).unwrap();

        let expected = if matches!(entry, JournalEntry::RunVersionsResolved { capability: Some(_), .. }) {
            let mut appended = v5.clone();
            appended.push(V5_ABSENT_IDEMPOTENCY_CLASS.as_u8());
            decode_entry_with_def_hash(&appended, ZERO_DEF_HASH()).unwrap()
        } else {
            decode_entry_with_def_hash(&v5, ZERO_DEF_HASH()).unwrap()
        };
        prop_assert_eq!(migrated, expected);
    }

    #[test]
    fn prop_migrate_current_version_is_fixed_point(entry in arb_run_versions_resolved()) {
        // Migrating already-current bytes equals decoding them (idempotent).
        let bytes = encode_entry(&entry).unwrap();
        let migrated = migrate_entry(&bytes, JOURNAL_SCHEMA_VERSION, ZERO_DEF_HASH()).unwrap();
        prop_assert_eq!(migrated, entry);
    }

    #[test]
    fn prop_v5_non_capability_kinds_decode_unchanged(entry in arb_digest_sealed()) {
        // A version-stable kind (DigestSealed) migrates identically to decoding.
        let bytes = encode_entry(&entry).unwrap();
        let migrated = migrate_entry(&bytes, 5, ZERO_DEF_HASH()).unwrap();
        let direct = decode_entry_with_def_hash(&bytes, ZERO_DEF_HASH()).unwrap();
        prop_assert_eq!(migrated.clone(), direct);
        prop_assert_eq!(migrated, entry);
    }
}
