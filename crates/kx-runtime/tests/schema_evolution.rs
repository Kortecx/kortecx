// Integration-test: schema-migration identity preservation (IMP-2, M2.x-E).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! The headline durability guarantee: migrating a journal from an older schema
//! version preserves the run's PRODUCT IDENTITY — the committed-facts digest is
//! byte-identical before (read via the up-converting [`ReplayJournal`]) and after
//! ([`migrate_and_verify`] rewrites to the current version). The resume/state
//! digest is version-local and may change, but identity is the durability law.

use std::path::{Path, PathBuf};

use kx_journal::{
    decode_entry_with_def_hash, FailureReason, IdempotencyClassTag, Journal, JournalEntry,
    ParentEntry, ReplayJournal, ResolvedCapabilityRecord, ResolvedKindTag, SqliteJournal,
    INSTANCE_ID_LEN,
};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_runtime::{digest_journal, migrate_and_verify};
use rusqlite::{params, Connection};
use smallvec::SmallVec;

/// The frozen golden product digest for the `build_v5` fixture under current code.
/// Recorded once; the migrated and replayed folds must both equal it. (If a future
/// schema bump legitimately changes committed-fact shape this is re-baselined as
/// part of that bump's migration protocol; a lineage-only bump must NOT change it.)
const EXPECTED_PRODUCT_DIGEST: &str =
    "6bf5b52dddb4ef94f56947365de215261745208180192bd151bf9a8aeaf7abf3";

fn curated_v6_entries() -> Vec<JournalEntry> {
    let instance_id = [1u8; INSTANCE_ID_LEN];
    let a = MoteId::from_bytes([10u8; 32]);
    vec![
        JournalEntry::RunRegistered {
            instance_id,
            recipe_fingerprint: [2u8; 32],
            ts: 0,
            seq: 0,
        },
        JournalEntry::Committed {
            mote_id: a,
            idempotency_key: [10u8; 32],
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref: kx_content::ContentRef::from_bytes([110u8; 32]),
            parents: SmallVec::new(),
            warrant_ref: kx_content::ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([20u8; 32]),
        },
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes([11u8; 32]),
            idempotency_key: [11u8; 32],
            seq: 0,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: kx_content::ContentRef::from_bytes([111u8; 32]),
            parents: {
                let mut p: SmallVec<[ParentEntry; 4]> = SmallVec::new();
                p.push(ParentEntry {
                    parent_id: a,
                    edge_kind: 0,
                    non_cascade: 0,
                });
                p
            },
            warrant_ref: kx_content::ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([21u8; 32]),
        },
        JournalEntry::RunVersionsResolved {
            instance_id,
            warrant_ref: kx_content::ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            capability: Some(ResolvedCapabilityRecord {
                tool_id: "fs.read".to_string(),
                tool_version: "1.0.0".to_string(),
                resolved_kind: ResolvedKindTag::Builtin,
                resolved_def_hash: kx_content::ContentRef::from_bytes([30u8; 32]),
                idempotency_class: IdempotencyClassTag::Token,
            }),
            seq: 0,
        },
        JournalEntry::Failed {
            mote_id: MoteId::from_bytes([12u8; 32]),
            idempotency_key: [12u8; 32],
            seq: 0,
            reason_class: FailureReason::TimedOut,
            reporter_id: 0,
        },
    ]
}

/// Build a v5 fixture: write v6, then downgrade with raw SQL (strip the trailing
/// capability class byte; stamp schema_version = 5).
fn build_v5(dir: &Path) -> PathBuf {
    let path = dir.join("run_v5.kxjournal");
    {
        let j = SqliteJournal::open(&path).unwrap();
        j.append_batch(curated_v6_entries()).unwrap();
    }
    let mut conn = Connection::open(&path).unwrap();
    let kind6: Vec<(i64, Vec<u8>)> = {
        let mut stmt = conn
            .prepare("SELECT seq, entry_bytes FROM entries WHERE kind = 6")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let txn = conn.transaction().unwrap();
    for (seq, bytes) in kind6 {
        let entry = decode_entry_with_def_hash(&bytes, MoteDefHash::from_bytes([0u8; 32])).unwrap();
        if matches!(
            entry,
            JournalEntry::RunVersionsResolved {
                capability: Some(_),
                ..
            }
        ) {
            let v5 = &bytes[..bytes.len() - 1];
            txn.execute(
                "UPDATE entries SET entry_bytes = ?1 WHERE seq = ?2",
                params![v5, seq],
            )
            .unwrap();
        }
    }
    txn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![&5u16.to_le_bytes()[..]],
    )
    .unwrap();
    txn.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    path
}

#[test]
fn migrate_and_verify_preserves_product_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5(tmp.path());
    let dst = tmp.path().join("run_v6.kxjournal");

    let report = migrate_and_verify(&src, &dst).unwrap();
    assert_eq!(report.from_version, 5);
    // The migration target tracks the current schema (v13 as of T-MULTI-ELEMENT-
    // TOOLCALLS's additive `ReactBranch::ToolBatch`; v12 was PR-9d's additive
    // `ReactRound.context_items_ref`; v11 was PR-R1's `ReactRound.is_agentic_launch`;
    // v10 was PR-3's `ReactBranch::Rejected`; v9 was PR-9b-2a's `ReactRound.step_salt`;
    // v8 was PR-2d-1's `ReactRound`; v7 was PR-2c-2's `ReplanRound`); the v5→current
    // up-conversion still appends exactly the lone `idempotency_class` byte (this v5
    // fixture has no kind-9 ReactRound, so none of the step_salt / is_agentic_launch /
    // context_items_ref trailing bytes is added, and the v9→v10..v12→v13 deltas only
    // touch kind-9 bodies no v5 entry can carry). Pinned as a reviewable change — the
    // PRODUCT identity digest below is the real invariant (unchanged across the bump).
    assert_eq!(report.to_version, 13);
    assert_eq!(report.entries_upconverted, 1);

    // The up-converted source and the migrated destination fold to the same
    // committed-facts digest (migrate_and_verify already enforced this, but assert
    // it explicitly + pin the frozen golden).
    let src_digest = digest_journal(&ReplayJournal::open(&src).unwrap()).unwrap();
    let dst_digest = digest_journal(&SqliteJournal::open(&dst).unwrap()).unwrap();
    assert_eq!(src_digest, dst_digest, "migration must preserve identity");
    eprintln!("PRODUCT DIGEST (record as golden): {}", dst_digest.to_hex());
    assert_eq!(
        dst_digest.to_hex(),
        EXPECTED_PRODUCT_DIGEST,
        "product identity digest drifted from the frozen golden"
    );
}

#[test]
fn migrated_journal_resumes_and_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5(tmp.path());
    let dst = tmp.path().join("run_v6.kxjournal");
    migrate_and_verify(&src, &dst).unwrap();

    // The migrated v6 journal opens with the STRICT backend and accepts a new
    // committed Mote — the run resumes after the binary upgrade.
    let j = SqliteJournal::open(&dst).unwrap();
    let before = j.count_entries().unwrap();
    j.append(JournalEntry::Committed {
        mote_id: MoteId::from_bytes([13u8; 32]),
        idempotency_key: [13u8; 32],
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: kx_content::ContentRef::from_bytes([113u8; 32]),
        parents: SmallVec::new(),
        warrant_ref: kx_content::ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([23u8; 32]),
    })
    .unwrap();
    assert_eq!(j.count_entries().unwrap(), before + 1);

    // The new committed fact changes the product digest (a genuinely new run state).
    let resumed_digest = digest_journal(&j).unwrap();
    assert_ne!(resumed_digest.to_hex(), EXPECTED_PRODUCT_DIGEST);
}
