//! Schema migration — the forward-migration story (IMP-2, M2.x-E).
//!
//! A durability product that cannot be upgraded is not durable. Every prior
//! `JOURNAL_SCHEMA_VERSION` bump shipped a *loud refusal* of the previous version
//! (`SchemaVersionMismatch`) and no migration, so shipping a new binary would
//! orphan every durable run on disk. This module closes that gap **before real
//! journals exist in the wild**: it up-converts an entry encoded under an older,
//! still-supported schema version into the current in-memory [`JournalEntry`]
//! shape, on the fly, without mutating the source bytes.
//!
//! ## What migration guarantees (and what it does not)
//!
//! - **Product identity is invariant across migration.** The run-identity product
//!   digest (`kx-runtime::digest_projection`) is computed over committed facts only
//!   (`mote_id ‖ result_ref ‖ nd_class`); none of the fields a bump adds are
//!   identity inputs. A migrated run keeps its identity. *This is the durability
//!   law.*
//! - **The resume/state digest is version-local.** `kx_projection`'s full-state
//!   `state_digest()` includes lineage metadata (e.g. a resolved capability's
//!   `idempotency_class`), so it is **not** byte-stable across a digest-shape
//!   bump. A pre-bump seal therefore will not match a post-migration re-fold; the
//!   checkpoint self-heals (full-fold + re-seal), since seals are discardable
//!   optimizations, never authoritative (D92/D103).
//!
//! ## The ladder
//!
//! [`migrate_entry`] dispatches on the on-disk `from_version`. Each older version
//! has a single-step up-converter to the current shape; the next real bump adds
//! one arm here plus one frozen fixture in `tests/fixtures/`, never a rewrite.
//! Versions newer than this binary, or older than [`MIN_SUPPORTED_SCHEMA_VERSION`],
//! are refused loudly (forward-compat preserved).
//!
//! ### v5 → v6 (the only production link today)
//!
//! The *only* on-disk byte difference between v5 and v6 is that a v6
//! `RunVersionsResolved` (kind 6) capability-present body carries a **trailing**
//! `idempotency_class` tag byte (M2.3b, D105.4) that v5 lacks. Because the
//! capability is the last field of the body, a v5 capability-present entry is
//! exactly a v6 one minus its final byte — so up-conversion appends the safe
//! default and delegates to the canonical [`decode_entry_with_def_hash`] (one
//! source of truth for parsing/validation). Every other v5 byte — including a
//! capability-absent `RunVersionsResolved` and a `DigestSealed` (kind 7, whose
//! 40-byte body is unchanged across v5→v6) — decodes identically under current
//! code, so it passes through untouched.

use kx_mote::MoteDefHash;

use crate::entry::{
    decode_entry_with_def_hash, IdempotencyClassTag, JournalEntry, HEADER_LEN, INSTANCE_ID_LEN,
    JOURNAL_SCHEMA_VERSION, KIND_REACT_ROUND, KIND_RUN_VERSIONS_RESOLVED,
};
use crate::JournalError;

/// The oldest on-disk schema version this binary can replay or migrate. Files
/// older than this are refused loudly (`SchemaVersionMismatch`) — no production
/// journals are retained across the pre-v5 bumps per the corpus, so inventing
/// fixtures for versions that never shipped durably would be dead weight.
pub const MIN_SUPPORTED_SCHEMA_VERSION: u16 = 5;

/// The idempotency class assigned to a v5 resolved-capability record that
/// predates the durable `idempotency_class` field (added v6, M2.3b/D105.4).
///
/// [`IdempotencyClassTag::AtLeastOnce`] is the **safest** choice: it has no
/// closing mechanism, so crash recovery never auto-redispatches a migrated
/// world-mutating effect — the worst case is a Quarantine/Compensate, never a
/// silent double-fire. Migration therefore cannot weaken exactly-once. This value
/// is baked into the frozen replay-corpus goldens, so it must not change without
/// re-baselining the corpus.
pub const V5_ABSENT_IDEMPOTENCY_CLASS: IdempotencyClassTag = IdempotencyClassTag::AtLeastOnce;

/// Up-convert one entry's canonical on-disk bytes from `from_version` to the
/// current in-memory [`JournalEntry`]. The source bytes are never mutated.
///
/// `def_hash` supplies the `mote_def_hash` metadata for `Committed` entries (it is
/// not in the canonical body bytes; the journal backend stores it in a column).
/// It is ignored for every other kind, exactly as [`decode_entry_with_def_hash`].
///
/// Refuses (`SchemaVersionMismatch`) a `from_version` newer than this binary's
/// [`JOURNAL_SCHEMA_VERSION`] or older than [`MIN_SUPPORTED_SCHEMA_VERSION`]; a
/// malformed body for a supported version surfaces as `Decode`.
///
/// When `from_version == JOURNAL_SCHEMA_VERSION` this is a thin pass-through to
/// [`decode_entry_with_def_hash`] — the current read path is behaviour-identical.
pub fn migrate_entry(
    bytes: &[u8],
    from_version: u16,
    def_hash: MoteDefHash,
) -> Result<JournalEntry, JournalError> {
    if !(MIN_SUPPORTED_SCHEMA_VERSION..=JOURNAL_SCHEMA_VERSION).contains(&from_version) {
        return Err(JournalError::SchemaVersionMismatch {
            expected: JOURNAL_SCHEMA_VERSION,
            found: from_version,
        });
    }
    // Dispatch on the EXACT on-disk version — never fall an older version through a
    // newer up-converter (a v6 capability body run through the v5→v6 step would
    // double-append its `idempotency_class` byte). `from_version` is guaranteed in
    // [MIN_SUPPORTED, CURRENT] by the guard above.
    match from_version {
        // v14 (current): no transform, the single source of truth for decode.
        JOURNAL_SCHEMA_VERSION => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v13 → v14: a PURE pass-through. The lone v13→v14 delta is the trailing
        // `image_ref` presence byte on a `ReactRound` (kind 9) body, stacked directly
        // after the v12 `context_items_ref` byte; a v13 body lacks it, and the canonical
        // decoder up-converts a byte-absent body to `image_ref == None` — so v13 bytes
        // decode correctly under v14 unchanged.
        13 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v12 → v13: a PURE pass-through. The lone v12→v13 delta is the brand-new
        // `ReactBranch::ToolBatch` (branch tag 5) — no v12 journal can contain a
        // tag-5 `ReactRound` body (the exact v9→v10 `Rejected`=4 precedent) — so
        // every existing kind/tag is byte-identical and v12 bytes decode correctly
        // under v13 unchanged.
        12 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v11 → v12: a PURE pass-through. The lone v11→v12 delta is the trailing
        // `context_items_ref` presence byte on a `ReactRound` (kind 9) body; a v11
        // body lacks it, and the canonical decoder up-converts a byte-absent body to
        // `context_items_ref == None` — so v11 bytes decode correctly under v12 unchanged.
        11 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v10 → v12: a PURE pass-through. A v10 `ReactRound` body lacks BOTH the
        // `is_agentic_launch` (v11) and `context_items_ref` (v12) trailing bytes; the
        // canonical decoder up-converts a byte-absent body to `is_agentic_launch ==
        // step_salt.is_some()` (the OLD Some-means-agentic semantics) and
        // `context_items_ref == None` — so v10 bytes decode correctly + with the SAME
        // disposition under v12 unchanged.
        10 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v9 → v11: a PURE pass-through. The v9→v10 delta is the brand-new
        // `ReactBranch::Rejected` (branch tag 4) — no v9 journal can contain a tag-4
        // `ReactRound` body — and the v10→v11 delta is the byte-absent
        // `is_agentic_launch` up-convert above; every existing kind/tag is
        // byte-identical, so v9 bytes decode correctly under v11 unchanged.
        9 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v8 → current: append the safe-default `None` step_salt presence byte to
        // each `ReactRound` (kind 9) body (the lone v8→v9 delta — a trailing
        // additive byte, exactly the v5→v6 shape); every other kind is
        // byte-identical and passes through. The result is current-shaped (the
        // v9→v10 delta adds nothing to an existing v8 entry).
        8 => upconvert_v8_to_current(bytes, def_hash),
        // v7 → v9: a PURE pass-through. Kinds 0..8 are byte-identical and v7
        // predates `ReactRound` (kind 9, added v8), so a v7 journal carries no
        // kind-9 body to grow — its existing bytes decode correctly under v9.
        7 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v6 → v9: a PURE pass-through. Kinds 0..7 are byte-identical and v6
        // predates both `ReplanRound` (kind 8) and `ReactRound` (kind 9) — no
        // kind-8/9 body to grow.
        6 => Ok(decode_entry_with_def_hash(bytes, def_hash)?),
        // v5 → v9: append the safe-default `idempotency_class` byte (the lone v5→v6
        // delta); v5 predates `ReactRound`, so no step_salt byte is needed. The
        // result is v6-shaped, which decodes identically under v9.
        5 => upconvert_v5_to_current(bytes, def_hash),
        // Unreachable (guarded above); kept total + fail-closed.
        other => Err(JournalError::SchemaVersionMismatch {
            expected: JOURNAL_SCHEMA_VERSION,
            found: other,
        }),
    }
}

/// v8 → current up-converter. The lone transform is appending the safe-default
/// `None` step_salt presence byte (`0`) to a `ReactRound` (kind 9) body; all other
/// v8 bytes are already valid current-version bytes and pass through. A v8
/// `ReactRound` body ends at `max_tool_calls` (no trailing byte), so appending `0`
/// produces a valid v9 `step_salt: None` body that the canonical decoder accepts.
fn upconvert_v8_to_current(
    bytes: &[u8],
    def_hash: MoteDefHash,
) -> Result<JournalEntry, JournalError> {
    if bytes.first() == Some(&KIND_REACT_ROUND) {
        let mut upconverted = Vec::with_capacity(bytes.len() + 1);
        upconverted.extend_from_slice(bytes);
        upconverted.push(0u8); // step_salt present == false (None)
        Ok(decode_entry_with_def_hash(&upconverted, def_hash)?)
    } else {
        Ok(decode_entry_with_def_hash(bytes, def_hash)?)
    }
}

/// v5 → current up-converter. The lone transform is appending the safe-default
/// `idempotency_class` byte to a capability-present `RunVersionsResolved`; all
/// other v5 bytes are already valid current-version bytes and pass through.
fn upconvert_v5_to_current(
    bytes: &[u8],
    def_hash: MoteDefHash,
) -> Result<JournalEntry, JournalError> {
    if v5_run_versions_has_capability(bytes) {
        let mut upconverted = Vec::with_capacity(bytes.len() + 1);
        upconverted.extend_from_slice(bytes);
        upconverted.push(V5_ABSENT_IDEMPOTENCY_CLASS.as_u8());
        Ok(decode_entry_with_def_hash(&upconverted, def_hash)?)
    } else {
        Ok(decode_entry_with_def_hash(bytes, def_hash)?)
    }
}

/// Returns `true` **only** when `bytes` is confidently a kind-6
/// (`RunVersionsResolved`) entry with `has_cap == 1`. Any bounds ambiguity yields
/// `false` so the canonical decoder (not this sniff) is the one to report the
/// error — keeping migration fail-closed. Total; never panics.
fn v5_run_versions_has_capability(bytes: &[u8]) -> bool {
    // Body prefix = instance_id(16) ‖ warrant_ref(32), then a u16-LE model_id_len.
    const PREFIX: usize = INSTANCE_ID_LEN + 32;
    if bytes.first() != Some(&KIND_RUN_VERSIONS_RESOLVED) {
        return false;
    }
    let Some(body) = bytes.get(HEADER_LEN..) else {
        return false;
    };
    let Some(model_id_len_bytes) = body.get(PREFIX..PREFIX + 2) else {
        return false;
    };
    let model_id_len = u16::from_le_bytes([model_id_len_bytes[0], model_id_len_bytes[1]]) as usize;
    // has_cap byte sits immediately after the (variable) model_id.
    let has_cap_offset = PREFIX + 2 + model_id_len;
    body.get(has_cap_offset) == Some(&1u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{encode_entry, ResolvedCapabilityRecord, ResolvedKindTag};
    use kx_content::ContentRef;

    // Build a current-version (v6) RunVersionsResolved entry with a capability,
    // using the given idempotency class.
    fn v6_run_versions_with_cap(class: IdempotencyClassTag) -> JournalEntry {
        JournalEntry::RunVersionsResolved {
            instance_id: [7u8; INSTANCE_ID_LEN],
            warrant_ref: ContentRef::from_bytes([9u8; 32]),
            model_id: "qwen2-0_5b".to_string(),
            capability: Some(ResolvedCapabilityRecord {
                tool_id: "fs.read".to_string(),
                tool_version: "1.2.3".to_string(),
                resolved_kind: ResolvedKindTag::Builtin,
                resolved_def_hash: ContentRef::from_bytes([3u8; 32]),
                idempotency_class: class,
            }),
            seq: 42,
        }
    }

    // Derive the *v5* on-disk bytes of a capability-present RunVersionsResolved by
    // encoding it at v6 and dropping the trailing idempotency_class byte — the one
    // and only v5→v6 delta. (This mirrors what the frozen fixtures hold.)
    fn v5_bytes_with_cap() -> Vec<u8> {
        let v6 = v6_run_versions_with_cap(IdempotencyClassTag::Token);
        let mut bytes = encode_entry(&v6).unwrap();
        bytes.pop(); // remove trailing idempotency_class tag → v5 shape
        bytes
    }

    #[test]
    fn refuses_version_newer_than_current() {
        let bytes = encode_entry(&v6_run_versions_with_cap(IdempotencyClassTag::Token)).unwrap();
        let err = migrate_entry(
            &bytes,
            JOURNAL_SCHEMA_VERSION + 1,
            MoteDefHash::from_bytes([0u8; 32]),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            JournalError::SchemaVersionMismatch { found, .. } if found == JOURNAL_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn refuses_version_older_than_min_supported() {
        let bytes = encode_entry(&v6_run_versions_with_cap(IdempotencyClassTag::Token)).unwrap();
        let err = migrate_entry(
            &bytes,
            MIN_SUPPORTED_SCHEMA_VERSION - 1,
            MoteDefHash::from_bytes([0u8; 32]),
        )
        .unwrap_err();
        assert!(matches!(err, JournalError::SchemaVersionMismatch { .. }));
    }

    #[test]
    fn current_version_is_passthrough() {
        let entry = v6_run_versions_with_cap(IdempotencyClassTag::Readback);
        let bytes = encode_entry(&entry).unwrap();
        let migrated = migrate_entry(
            &bytes,
            JOURNAL_SCHEMA_VERSION,
            MoteDefHash::from_bytes([0u8; 32]),
        )
        .unwrap();
        assert_eq!(migrated, entry);
    }

    #[test]
    fn v5_capability_defaults_to_at_least_once() {
        let v5 = v5_bytes_with_cap();
        let migrated = migrate_entry(&v5, 5, MoteDefHash::from_bytes([0u8; 32])).unwrap();
        match migrated {
            JournalEntry::RunVersionsResolved {
                capability: Some(cap),
                ..
            } => {
                assert_eq!(cap.idempotency_class, IdempotencyClassTag::AtLeastOnce);
                assert_eq!(cap.tool_id, "fs.read");
                assert_eq!(cap.tool_version, "1.2.3");
                assert_eq!(cap.resolved_kind, ResolvedKindTag::Builtin);
            }
            other => panic!("expected RunVersionsResolved with capability, got {other:?}"),
        }
    }

    #[test]
    fn v5_capability_matches_v6_with_explicit_default() {
        // Up-converting a v5 cap entry must equal the v6 entry that carries the
        // explicit default class — the migration adds nothing else.
        let v5 = v5_bytes_with_cap();
        let migrated = migrate_entry(&v5, 5, MoteDefHash::from_bytes([0u8; 32])).unwrap();
        let expected = v6_run_versions_with_cap(V5_ABSENT_IDEMPOTENCY_CLASS);
        assert_eq!(migrated, expected);
    }

    #[test]
    fn v5_no_capability_is_byte_identical() {
        // A capability-absent RunVersionsResolved is unchanged v5→v6: migrating its
        // bytes equals decoding them directly.
        let no_cap = JournalEntry::RunVersionsResolved {
            instance_id: [4u8; INSTANCE_ID_LEN],
            warrant_ref: ContentRef::from_bytes([5u8; 32]),
            model_id: "m".to_string(),
            capability: None,
            seq: 3,
        };
        let bytes = encode_entry(&no_cap).unwrap();
        let migrated = migrate_entry(&bytes, 5, MoteDefHash::from_bytes([0u8; 32])).unwrap();
        let direct =
            decode_entry_with_def_hash(&bytes, MoteDefHash::from_bytes([0u8; 32])).unwrap();
        assert_eq!(migrated, direct);
        assert_eq!(migrated, no_cap);
    }

    #[test]
    fn v5_other_kinds_pass_through_unchanged() {
        // A DigestSealed (kind 7) body is unchanged across v5→v6; migrating it must
        // equal decoding it directly (the sniff must not touch non-kind-6 bytes).
        let sealed = JournalEntry::DigestSealed {
            through_seq: 10,
            state_digest: [0xAB; 32],
            seq: 11,
        };
        let bytes = encode_entry(&sealed).unwrap();
        let migrated = migrate_entry(&bytes, 5, MoteDefHash::from_bytes([0u8; 32])).unwrap();
        assert_eq!(migrated, sealed);
        assert!(!v5_run_versions_has_capability(&bytes));
    }

    #[test]
    fn sniff_detects_capability_present_and_absent() {
        assert!(v5_run_versions_has_capability(&v5_bytes_with_cap()));
        let no_cap = JournalEntry::RunVersionsResolved {
            instance_id: [1u8; INSTANCE_ID_LEN],
            warrant_ref: ContentRef::from_bytes([2u8; 32]),
            model_id: "x".to_string(),
            capability: None,
            seq: 1,
        };
        assert!(!v5_run_versions_has_capability(
            &encode_entry(&no_cap).unwrap()
        ));
    }

    #[test]
    fn malformed_v5_bytes_fail_closed() {
        // Truncated garbage must never silently decode — it errors.
        let garbage = vec![KIND_RUN_VERSIONS_RESOLVED, 0u8, 1u8, 2u8];
        assert!(migrate_entry(&garbage, 5, MoteDefHash::from_bytes([0u8; 32])).is_err());
    }
}
