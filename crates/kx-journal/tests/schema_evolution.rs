// Integration-test file: compiled as a separate crate from the host lib.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Schema-evolution replay corpus (IMP-2, carrier M2.x-E).
//!
//! Proves the forward-migration story: a journal written under an older,
//! still-supported schema version can be replayed read-only ([`ReplayJournal`])
//! and rewritten into a fresh current-version journal ([`migrate_to`]) that the
//! strict [`SqliteJournal::open`] accepts, resumes, and appends to — without
//! changing the run's committed facts.
//!
//! ## The frozen v5 representation
//!
//! No production journals exist in the wild yet, so the v5 fixture is built
//! deterministically by [`build_v5_journal`]: write a current-version (v6)
//! journal via the normal backend, then *downgrade* it with raw SQL to the exact
//! v5 byte shape — strip the trailing `idempotency_class` byte from every
//! capability-present `RunVersionsResolved` (the one and only v5→v6 delta) and
//! stamp `metadata.schema_version = 5`. The v5 shape is therefore defined, as the
//! corpus documents it, as "v6 minus the trailing capability class byte". The
//! `migrate_entry` unit tests pin that byte-level relationship independently.

use std::path::{Path, PathBuf};
use std::time::Instant;

use kx_content::ContentRef;
use kx_journal::{
    decode_entry_with_def_hash, migrate_to, FailureReason, IdempotencyClassTag, Journal,
    JournalEntry, JournalError, ParentEntry, ReactBranch, ReplayJournal, ResolvedCapabilityRecord,
    ResolvedKindTag, SqliteJournal, INSTANCE_ID_LEN, JOURNAL_SCHEMA_VERSION,
    MIN_SUPPORTED_SCHEMA_VERSION, V5_ABSENT_IDEMPOTENCY_CLASS,
};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use rusqlite::{params, Connection};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Fixture: a small, representative v5 journal
// ---------------------------------------------------------------------------

/// The curated v5 entry set (constructed at v6, downgraded in `build_v5_journal`):
/// a run registration, two committed Motes (B depends on A), a resolved-capability
/// record (the entry that actually up-converts), a zero-grant resolved-versions
/// record, a terminal failure, and a digest seal (kind 7, version-stable body).
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
            result_ref: ContentRef::from_bytes([110u8; 32]),
            parents: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([20u8; 32]),
        },
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes([11u8; 32]),
            idempotency_key: [11u8; 32],
            seq: 0,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: ContentRef::from_bytes([111u8; 32]),
            parents: {
                let mut p = SmallVec::new();
                p.push(ParentEntry {
                    parent_id: a,
                    edge_kind: 0, // Data
                    non_cascade: 0,
                });
                p
            },
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([21u8; 32]),
        },
        JournalEntry::RunVersionsResolved {
            instance_id,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            // Original resolved class was Token; v5 did not record it, so migration
            // must apply the SAFE default (AtLeastOnce), not recover Token.
            capability: Some(ResolvedCapabilityRecord {
                tool_id: "fs.read".to_string(),
                tool_version: "1.0.0".to_string(),
                resolved_kind: ResolvedKindTag::Builtin,
                resolved_def_hash: ContentRef::from_bytes([30u8; 32]),
                idempotency_class: IdempotencyClassTag::Token,
            }),
            seq: 0,
        },
        JournalEntry::RunVersionsResolved {
            instance_id,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            capability: None,
            seq: 0,
        },
        JournalEntry::Failed {
            mote_id: MoteId::from_bytes([12u8; 32]),
            idempotency_key: [12u8; 32],
            seq: 0,
            reason_class: FailureReason::TimedOut,
            reporter_id: 0,
        },
        JournalEntry::DigestSealed {
            through_seq: 6,
            state_digest: [0xEE; 32],
            seq: 0,
        },
    ]
}

/// Build the v5 fixture journal at `dir/sample_v5.kxjournal` and return its path.
fn build_v5_journal(dir: &Path) -> PathBuf {
    let path = dir.join("sample_v5.kxjournal");
    {
        let j = SqliteJournal::open(&path).unwrap();
        j.append_batch(curated_v6_entries()).unwrap();
    }
    downgrade_to_v5(&path);
    path
}

/// Raw-SQL downgrade of a v6 journal file to the v5 byte shape: strip the trailing
/// `idempotency_class` byte from each capability-present `RunVersionsResolved`, and
/// stamp `metadata.schema_version = 5`. Wrapped in one transaction.
fn downgrade_to_v5(path: &Path) {
    let mut conn = Connection::open(path).unwrap();
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
            let v5 = &bytes[..bytes.len() - 1]; // drop the trailing class byte
            txn.execute(
                "UPDATE entries SET entry_bytes = ?1 WHERE seq = ?2",
                params![v5, seq],
            )
            .unwrap();
        }
    }
    let v5_ver: [u8; 2] = 5u16.to_le_bytes();
    txn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![&v5_ver[..]],
    )
    .unwrap();
    txn.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
}

fn read_all<J: Journal>(j: &J) -> Vec<JournalEntry> {
    let head = j.current_seq().unwrap();
    j.read_entries_by_seq(0..head + 1).unwrap().collect()
}

// ---------------------------------------------------------------------------
// Read-side: ReplayJournal up-converts an old journal on the fly
// ---------------------------------------------------------------------------

#[test]
fn open_still_refuses_v5_loudly() {
    // Regression guard: the strict open() contract is UNCHANGED — it must still
    // refuse an older version loudly (migration is a separate, additive path).
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v5_journal(tmp.path());
    let err = SqliteJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found: 5, expected } if expected == JOURNAL_SCHEMA_VERSION
    ));
}

#[test]
fn replay_reads_v5_and_upconverts_capability() {
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v5_journal(tmp.path());

    let replay = ReplayJournal::open(&path).unwrap();
    assert_eq!(replay.from_version(), 5);
    assert_eq!(replay.count_entries().unwrap(), 7);

    let entries = read_all(&replay);
    let cap = entries
        .iter()
        .find_map(|e| match e {
            JournalEntry::RunVersionsResolved {
                capability: Some(c),
                ..
            } => Some(c),
            _ => None,
        })
        .expect("a capability-present RunVersionsResolved");
    // The original class was Token; v5 lost it; migration applies the safe default.
    assert_eq!(cap.idempotency_class, V5_ABSENT_IDEMPOTENCY_CLASS);
    assert_eq!(cap.idempotency_class, IdempotencyClassTag::AtLeastOnce);
    assert_eq!(cap.tool_id, "fs.read");
}

#[test]
fn replay_journal_is_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v5_journal(tmp.path());
    let replay = ReplayJournal::open(&path).unwrap();
    let attempt = replay.append(JournalEntry::Failed {
        mote_id: MoteId::from_bytes([99u8; 32]),
        idempotency_key: [99u8; 32],
        seq: 0,
        reason_class: FailureReason::TimedOut,
        reporter_id: 0,
    });
    assert!(matches!(attempt, Err(JournalError::Invariant(_))));
}

#[test]
fn open_for_replay_refuses_future_version() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("future.kxjournal");
    {
        let _ = SqliteJournal::open(&path).unwrap();
    }
    set_schema_version(&path, JOURNAL_SCHEMA_VERSION + 1);
    let err = ReplayJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found, .. } if found == JOURNAL_SCHEMA_VERSION + 1
    ));
}

#[test]
fn open_for_replay_refuses_too_old() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("ancient.kxjournal");
    {
        let _ = SqliteJournal::open(&path).unwrap();
    }
    set_schema_version(&path, MIN_SUPPORTED_SCHEMA_VERSION - 1);
    let err = ReplayJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found, .. } if found == MIN_SUPPORTED_SCHEMA_VERSION - 1
    ));
}

fn set_schema_version(path: &Path, v: u16) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![&v.to_le_bytes()[..]],
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// The frozen v7 representation (PR-2d-1): v7 = v8 minus ReactRound entries.
// Kinds 0..8 are byte-identical under v8; v7→v8 is a pure pass-through.
// ---------------------------------------------------------------------------

/// Build the v7 fixture journal: the curated entry set PLUS a `ReplanRound`
/// (kind 8, the v7 addition), with NO `ReactRound` (kind 9 — the v8 addition),
/// then stamp `metadata.schema_version = 7`. Since kinds 0..8 encode byte-
/// identically under v7 and v8, the stamp alone defines the v7 shape.
fn build_v7_journal(dir: &Path) -> PathBuf {
    let path = dir.join("sample_v7.kxjournal");
    {
        let j = SqliteJournal::open(&path).unwrap();
        let mut entries = curated_v6_entries();
        entries.push(JournalEntry::ReplanRound {
            round: 1,
            shaper_mote_id: MoteId::from_bytes([0x7c; 32]),
            base_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0x22; 32]),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            failed_steps: SmallVec::new(),
            escalation_reason_ref: None,
            seq: 0,
        });
        j.append_batch(entries).unwrap();
    }
    set_schema_version(&path, 7);
    path
}

#[test]
fn open_still_refuses_v7_loudly() {
    // The strict open() contract is unchanged by the v8 bump: a v7 journal is
    // refused loudly (migration is the separate, additive path) — the same
    // contract that makes an OLD binary refuse a v8 journal rather than
    // misread it (forward-compat = refusal, never corruption).
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v7_journal(tmp.path());
    let err = SqliteJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found: 7, expected } if expected == JOURNAL_SCHEMA_VERSION
    ));
}

#[test]
fn replay_reads_v7_as_pure_passthrough() {
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v7_journal(tmp.path());

    let replay = ReplayJournal::open(&path).unwrap();
    assert_eq!(replay.from_version(), 7);
    assert_eq!(replay.count_entries().unwrap(), 8);

    // The v7 ReplanRound (kind 8) decodes unchanged, capability classes are
    // PRESERVED (not defaulted — that is the v5 path), and no ReactRound exists.
    let entries = read_all(&replay);
    assert!(entries
        .iter()
        .any(|e| matches!(e, JournalEntry::ReplanRound { round: 1, .. })));
    let cap = entries
        .iter()
        .find_map(|e| match e {
            JournalEntry::RunVersionsResolved {
                capability: Some(c),
                ..
            } => Some(c),
            _ => None,
        })
        .expect("a capability-present RunVersionsResolved");
    assert_eq!(cap.idempotency_class, IdempotencyClassTag::Token);
    assert!(!entries
        .iter()
        .any(|e| matches!(e, JournalEntry::ReactRound { .. })));
}

#[test]
fn migrate_v7_to_current_upconverts_nothing_and_preserves_committed_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v7_journal(tmp.path());
    let dst = tmp.path().join("migrated_from_v7.kxjournal");

    let report = migrate_to(&src, &dst).unwrap();
    assert_eq!(report.from_version, 7);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.entries_migrated, 8);
    assert_eq!(report.entries_upconverted, 0); // pure pass-through

    // Strict open accepts the result; committed facts are byte-identical
    // (product identity invariant — the durability law).
    let j = SqliteJournal::open(&dst).unwrap();
    assert_eq!(j.count_entries().unwrap(), 8);
    let committed_bytes = |p: &Path| -> Vec<Vec<u8>> {
        let conn = Connection::open(p).unwrap();
        let mut stmt = conn
            .prepare("SELECT entry_bytes FROM entries WHERE kind = 1 ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(committed_bytes(&src), committed_bytes(&dst));
}

// ---------------------------------------------------------------------------
// The frozen v8 representation (PR-9b-2a): v8 = v9 minus the trailing ReactRound
// step_salt presence byte. Kinds 0..8 are byte-identical under v9; the lone v8→v9
// delta is a `0` (None) byte appended to each kind-9 `ReactRound` body.
// ---------------------------------------------------------------------------

/// Build the v8 fixture journal: the curated entry set PLUS a `ReplanRound`
/// (kind 8) AND two `ReactRound` facts (kind 9, the v8 addition — an anchor +
/// settle), then DOWNGRADE the kind-9 bodies to the v8 byte shape (strip the two
/// trailing additive bytes the current v11 encoder now writes — the v11
/// `is_agentic_launch` byte and the v9 `step_salt` presence byte) and stamp
/// `metadata.schema_version = 8`.
fn build_v8_journal(dir: &Path) -> PathBuf {
    let path = dir.join("sample_v8.kxjournal");
    {
        let j = SqliteJournal::open(&path).unwrap();
        let mut entries = curated_v6_entries();
        entries.push(JournalEntry::ReplanRound {
            round: 1,
            shaper_mote_id: MoteId::from_bytes([0x7c; 32]),
            base_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0x22; 32]),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            failed_steps: SmallVec::new(),
            escalation_reason_ref: None,
            seq: 0,
        });
        entries.push(JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "qwen2-0_5b".to_string(),
            branch: ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            seq: 0,
        });
        entries.push(JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "qwen2-0_5b".to_string(),
            branch: ReactBranch::Answer,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            seq: 0,
        });
        j.append_batch(entries).unwrap();
    }
    downgrade_to_v8(&path);
    path
}

/// Raw-SQL downgrade of a v11 journal file to the v8 byte shape: strip the two
/// trailing additive bytes from each `ReactRound` (kind 9) — the v11
/// `is_agentic_launch` byte AND the v9 `step_salt` presence byte — and stamp
/// `metadata.schema_version = 8`. Wrapped in one transaction.
fn downgrade_to_v8(path: &Path) {
    let mut conn = Connection::open(path).unwrap();
    let kind9: Vec<(i64, Vec<u8>)> = {
        let mut stmt = conn
            .prepare("SELECT seq, entry_bytes FROM entries WHERE kind = 9")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let txn = conn.transaction().unwrap();
    for (seq, bytes) in kind9 {
        // The fixture only writes step_salt: None + is_agentic_launch: false +
        // context_items_ref: None + image_ref: None, so the FOUR current tail bytes are
        // `[step_salt_present=0, is_agentic_launch=0, context_items_present=0, image_present=0]`;
        // dropping all four yields the exact v8 shape.
        assert_eq!(
            bytes[bytes.len() - 4..],
            [0u8, 0u8, 0u8, 0u8],
            "fixture ReactRound must be step_salt None + is_agentic_launch false + context None + image None"
        );
        let v8 = &bytes[..bytes.len() - 4];
        txn.execute(
            "UPDATE entries SET entry_bytes = ?1 WHERE seq = ?2",
            params![v8, seq],
        )
        .unwrap();
    }
    let v8_ver: [u8; 2] = 8u16.to_le_bytes();
    txn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![&v8_ver[..]],
    )
    .unwrap();
    txn.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
}

#[test]
fn open_still_refuses_v8_loudly() {
    // The strict open() contract is unchanged by the v9 bump: a v8 journal is
    // refused loudly (migration is the separate, additive path) — the same
    // contract that makes an OLD binary refuse a v9 journal rather than misread
    // its trailing step_salt byte (forward-compat = refusal, never corruption).
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v8_journal(tmp.path());
    let err = SqliteJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found: 8, expected } if expected == JOURNAL_SCHEMA_VERSION
    ));
}

#[test]
fn replay_reads_v8_and_upconverts_react_step_salt() {
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v8_journal(tmp.path());

    let replay = ReplayJournal::open(&path).unwrap();
    assert_eq!(replay.from_version(), 8);
    assert_eq!(replay.count_entries().unwrap(), 10);

    // The two v8 ReactRound facts up-convert to step_salt: None (the run-level
    // chain — every chain a v8 binary ever wrote); capability classes from the
    // curated set are PRESERVED (not defaulted — that is the v5 path).
    let entries = read_all(&replay);
    let react: Vec<_> = entries
        .iter()
        .filter_map(|e| match e {
            JournalEntry::ReactRound { step_salt, .. } => Some(*step_salt),
            _ => None,
        })
        .collect();
    assert_eq!(react, vec![None, None]);
    let cap = entries
        .iter()
        .find_map(|e| match e {
            JournalEntry::RunVersionsResolved {
                capability: Some(c),
                ..
            } => Some(c),
            _ => None,
        })
        .expect("a capability-present RunVersionsResolved");
    assert_eq!(cap.idempotency_class, IdempotencyClassTag::Token);
}

#[test]
fn migrate_v8_to_v9_upconverts_react_and_preserves_committed_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v8_journal(tmp.path());
    let dst = tmp.path().join("migrated_v9.kxjournal");

    let report = migrate_to(&src, &dst).unwrap();
    assert_eq!(report.from_version, 8);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.entries_migrated, 10);
    assert_eq!(report.entries_upconverted, 2); // exactly the two ReactRound facts

    // Strict open accepts the result; committed facts are byte-identical (product
    // identity invariant — the durability law; only kind-9 bodies grow, by the
    // appended trailing bytes: step_salt + is_agentic_launch + context_items + image).
    let j = SqliteJournal::open(&dst).unwrap();
    assert_eq!(j.count_entries().unwrap(), 10);
    let committed_bytes = |p: &Path| -> Vec<Vec<u8>> {
        let conn = Connection::open(p).unwrap();
        let mut stmt = conn
            .prepare("SELECT entry_bytes FROM entries WHERE kind = 1 ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(committed_bytes(&src), committed_bytes(&dst));

    // The migrated ReactRound facts now carry the explicit None step_salt.
    let via_migrate = read_all(&j);
    assert!(via_migrate
        .iter()
        .filter(|e| matches!(e, JournalEntry::ReactRound { .. }))
        .all(|e| matches!(
            e,
            JournalEntry::ReactRound {
                step_salt: None,
                ..
            }
        )));
}

#[test]
fn v9_react_round_persists_and_resumes() {
    // A fresh (v9) journal carrying ReactRound facts round-trips the strict
    // open + resume path: the anchor (run-level, step_salt None) and a settled
    // branch carrying a step_salt (the agentic-step shape) read back verbatim.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("react_v9.kxjournal");
    let anchor = JournalEntry::ReactRound {
        turn: 0,
        turn_mote_id: MoteId::from_bytes([0x8e; 32]),
        instance_id: [0x4d; INSTANCE_ID_LEN],
        base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
        warrant_ref: ContentRef::from_bytes([0x34; 32]),
        model_id: "qwen2-0_5b".to_string(),
        branch: ReactBranch::Pending,
        max_turns: 8,
        max_tool_calls: 8,
        step_salt: None,
        is_agentic_launch: false,
        context_items_ref: None,
        image_ref: None,
        seq: 0,
    };
    let settle = JournalEntry::ReactRound {
        turn: 0,
        turn_mote_id: MoteId::from_bytes([0x8e; 32]),
        instance_id: [0x4d; INSTANCE_ID_LEN],
        base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
        warrant_ref: ContentRef::from_bytes([0x34; 32]),
        model_id: "qwen2-0_5b".to_string(),
        branch: ReactBranch::Answer,
        max_turns: 8,
        max_tool_calls: 8,
        step_salt: Some([0x5a; 32]),
        is_agentic_launch: true,
        context_items_ref: None,
        image_ref: None,
        seq: 0,
    };
    {
        let j = SqliteJournal::open(&path).unwrap();
        j.append_batch(vec![anchor, settle]).unwrap();
    }
    let j = SqliteJournal::open(&path).unwrap(); // resume
    let entries = read_all(&j);
    assert_eq!(entries.len(), 2);
    assert!(matches!(
        &entries[0],
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::Pending,
            max_turns: 8,
            step_salt: None,
            ..
        }
    ));
    assert!(matches!(
        &entries[1],
        JournalEntry::ReactRound {
            branch: ReactBranch::Answer,
            step_salt: Some(salt),
            ..
        } if *salt == [0x5a; 32]
    ));
}

/// v13 (T-MULTI-ELEMENT-TOOLCALLS): a `ReactBranch::ToolBatch` fact (a turn that
/// proposed N≥2 tool calls) persists + resumes through the strict open path with
/// its ordered calls intact — including two calls to the SAME tool. Proves the
/// new branch survives a real SqliteJournal write + reopen, not just the in-memory
/// codec round-trip.
#[test]
fn v13_tool_batch_round_persists_and_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("react_v13_batch.kxjournal");
    let batch = JournalEntry::ReactRound {
        turn: 1,
        turn_mote_id: MoteId::from_bytes([0x8e; 32]),
        instance_id: [0x4d; INSTANCE_ID_LEN],
        base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
        warrant_ref: ContentRef::from_bytes([0x34; 32]),
        model_id: "kx-serve:gemma3-12b".to_string(),
        branch: ReactBranch::ToolBatch {
            calls: vec![
                ("mcp-echo".to_string(), "1".to_string()),
                ("mcp-echo".to_string(), "1".to_string()),
                ("fs-read".to_string(), "1".to_string()),
            ],
        },
        max_turns: 8,
        max_tool_calls: 20,
        step_salt: None,
        is_agentic_launch: false,
        context_items_ref: None,
        image_ref: None,
        seq: 0,
    };
    {
        let j = SqliteJournal::open(&path).unwrap();
        j.append_batch(vec![batch.clone()]).unwrap();
    }
    let j = SqliteJournal::open(&path).unwrap(); // resume
    let entries = read_all(&j);
    assert_eq!(entries.len(), 1);
    // `seq` is journal-assigned on append, so compare the durable fields (not the
    // whole struct) — exactly like `v9_react_round_persists_and_resumes`.
    let JournalEntry::ReactRound {
        turn,
        branch: ReactBranch::ToolBatch { calls },
        max_tool_calls,
        ..
    } = &entries[0]
    else {
        panic!("expected a resumed ToolBatch ReactRound");
    };
    assert_eq!(*turn, 1);
    assert_eq!(
        calls.len(),
        3,
        "all three calls survive the reopen in order"
    );
    assert_eq!(calls[0], ("mcp-echo".to_string(), "1".to_string()));
    assert_eq!(calls[1], ("mcp-echo".to_string(), "1".to_string()));
    assert_eq!(calls[2], ("fs-read".to_string(), "1".to_string()));
    assert_eq!(*max_tool_calls, 20, "the raised tool-call ceiling persists");
}

// ---------------------------------------------------------------------------
// The frozen v9 representation (PR-3, A2). v9 = v10 minus the brand-new
// `ReactBranch::Rejected` (branch tag 4). Every existing kind/tag (including the
// kind-9 `ReactRound` bodies for tags 0..=3) is byte-identical under v9 and v10,
// and no v9 journal can contain a tag-4 body — so v9 → v10 is a PURE pass-through
// (exactly the v7/v6 → current shape, but proving the ReactRound facts survive).
// ---------------------------------------------------------------------------

/// Build the v9 fixture journal: the curated entry set PLUS a `ReplanRound`
/// (kind 8) AND two `ReactRound` facts (kind 9 — an anchor + a settled answer),
/// then DOWNGRADE the kind-9 bodies to the v9 byte shape (strip the trailing v11
/// `is_agentic_launch` byte the current encoder now writes — a v9 `ReactRound`
/// body carries the v9 `step_salt` presence byte but NOT the launch byte) and
/// stamp `metadata.schema_version = 9`. A v9 body carrying a tag 0..=3 branch is
/// otherwise byte-identical to its v10 encoding (the lone v9→v10 delta is the new
/// tag-4 reason slot, which no v9 journal exercises).
fn build_v9_journal(dir: &Path) -> PathBuf {
    let path = dir.join("sample_v9.kxjournal");
    {
        let j = SqliteJournal::open(&path).unwrap();
        let mut entries = curated_v6_entries();
        entries.push(JournalEntry::ReplanRound {
            round: 1,
            shaper_mote_id: MoteId::from_bytes([0x7c; 32]),
            base_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0x22; 32]),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            failed_steps: SmallVec::new(),
            escalation_reason_ref: None,
            seq: 0,
        });
        entries.push(JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "qwen2-0_5b".to_string(),
            branch: ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            seq: 0,
        });
        entries.push(JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "qwen2-0_5b".to_string(),
            branch: ReactBranch::Answer,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            seq: 0,
        });
        j.append_batch(entries).unwrap();
    }
    downgrade_to_v9(&path);
    path
}

/// Raw-SQL downgrade of a v11 journal file to the v9 byte shape: strip ONLY the
/// trailing v11 `is_agentic_launch` byte from each `ReactRound` (kind 9), leaving
/// the v9 `step_salt` presence byte in place, and stamp `metadata.schema_version
/// = 9`. Wrapped in one transaction.
fn downgrade_to_v9(path: &Path) {
    let mut conn = Connection::open(path).unwrap();
    let kind9: Vec<(i64, Vec<u8>)> = {
        let mut stmt = conn
            .prepare("SELECT seq, entry_bytes FROM entries WHERE kind = 9")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let txn = conn.transaction().unwrap();
    for (seq, bytes) in kind9 {
        // The fixture only writes is_agentic_launch: false + context_items_ref: None +
        // image_ref: None (the run-level chain), so the three current tail bytes are
        // `[is_agentic=0, context_present=0, image_present=0]`; dropping all three
        // (leaving step_salt) yields the v9 shape.
        assert_eq!(
            bytes[bytes.len() - 3..],
            [0u8, 0u8, 0u8],
            "fixture ReactRound must be is_agentic_launch false + context None + image None"
        );
        let v9 = &bytes[..bytes.len() - 3];
        txn.execute(
            "UPDATE entries SET entry_bytes = ?1 WHERE seq = ?2",
            params![v9, seq],
        )
        .unwrap();
    }
    let v9_ver: [u8; 2] = 9u16.to_le_bytes();
    txn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![&v9_ver[..]],
    )
    .unwrap();
    txn.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
}

#[test]
fn open_still_refuses_v9_loudly() {
    // The strict open() contract is unchanged by the v10 bump: a v9 journal is
    // refused loudly (migration is the separate, additive path) — the same
    // contract that makes an OLD binary refuse a v10 journal (carrying a Rejected
    // branch it cannot decode) rather than misread it.
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v9_journal(tmp.path());
    let err = SqliteJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found: 9, expected } if expected == JOURNAL_SCHEMA_VERSION
    ));
}

#[test]
fn migrate_v9_to_current_upconverts_react_launch_flag_and_preserves_committed_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v9_journal(tmp.path());
    let dst = tmp.path().join("migrated_from_v9.kxjournal");

    let report = migrate_to(&src, &dst).unwrap();
    assert_eq!(report.from_version, 9);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.entries_migrated, 10);
    // The two ReactRound facts grow by the trailing is_agentic_launch byte.
    assert_eq!(report.entries_upconverted, 2);

    // Strict open accepts the result; committed facts are byte-identical (product
    // identity invariant — the durability law). The kind-9 ReactRound bodies grow
    // by three trailing `0` bytes: the up-converted run-level launch flag
    // (is_agentic_launch == step_salt.is_some() == false) + the absent context_items_ref
    // (None) + the absent image_ref (None).
    let j = SqliteJournal::open(&dst).unwrap();
    assert_eq!(j.count_entries().unwrap(), 10);
    let kind_bytes = |p: &Path, kind: i64| -> Vec<Vec<u8>> {
        let conn = Connection::open(p).unwrap();
        let mut stmt = conn
            .prepare("SELECT entry_bytes FROM entries WHERE kind = ?1 ORDER BY seq")
            .unwrap();
        stmt.query_map(params![kind], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(kind_bytes(&src, 1), kind_bytes(&dst, 1)); // committed facts byte-identical
    let src9 = kind_bytes(&src, 9);
    let dst9 = kind_bytes(&dst, 9);
    assert_eq!(src9.len(), dst9.len());
    for (src_body, dst_body) in src9.iter().zip(dst9.iter()) {
        // Each dst body = the src v9 body + three trailing `0` bytes: the up-converted
        // run-level is_agentic_launch flag (step_salt.is_some() == false) + the absent
        // context_items_ref (None) + the absent image_ref (None).
        assert_eq!(*dst_body, [src_body.clone(), vec![0u8, 0u8, 0u8]].concat());
    }

    // The migrated ReactRound facts decode with the run-level launch flag.
    let via_migrate = read_all(&j);
    assert!(via_migrate
        .iter()
        .filter(|e| matches!(e, JournalEntry::ReactRound { .. }))
        .all(|e| matches!(
            e,
            JournalEntry::ReactRound {
                is_agentic_launch: false,
                ..
            }
        )));
}

// ---------------------------------------------------------------------------
// The frozen v10 representation (PR-R1): v10 = v11 minus the trailing ReactRound
// `is_agentic_launch` byte. Kinds 0..9 (and the ReactRound branch tags 0..=4) are
// byte-identical under v10 and v11; the lone v10→v11 delta is the `is_agentic_launch`
// byte appended to each kind-9 `ReactRound` body, which DECODE-time up-converts to
// `step_salt.is_some()` (the OLD Some-means-agentic semantics).
// ---------------------------------------------------------------------------

/// Raw-SQL downgrade of a v11 journal file to the v10 byte shape: strip ONLY the
/// trailing v11 `is_agentic_launch` byte from each `ReactRound` (kind 9), and stamp
/// `metadata.schema_version = 10`. Wrapped in one transaction. Unlike the v8/v9
/// downgrades, the stripped byte is NOT asserted to be `0`: the v10 fixture
/// includes an agentic-step settle (step_salt Some ⇒ is_agentic_launch true ⇒
/// launch byte `1`), and dropping it correctly yields the v10 shape (which then
/// up-converts on migration to `is_agentic_launch == step_salt.is_some()`).
fn downgrade_to_v10(path: &Path) {
    let mut conn = Connection::open(path).unwrap();
    let kind9: Vec<(i64, Vec<u8>)> = {
        let mut stmt = conn
            .prepare("SELECT seq, entry_bytes FROM entries WHERE kind = 9")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let txn = conn.transaction().unwrap();
    for (seq, bytes) in kind9 {
        // Drop the final THREE bytes unconditionally — the v14 image_ref present byte
        // (`0`, the fixture attaches no image), the v12 context_items_ref present byte
        // (`0`, no context) and the v11 is_agentic_launch byte (which may be `1` for the
        // agentic-step settle). v9 and v10 ReactRound bodies are byte-identical (the
        // v9→v10 delta is the Rejected branch tag, not a trailing byte), so this strips
        // to the v10 shape.
        let v10 = &bytes[..bytes.len() - 3];
        txn.execute(
            "UPDATE entries SET entry_bytes = ?1 WHERE seq = ?2",
            params![v10, seq],
        )
        .unwrap();
    }
    let v10_ver: [u8; 2] = 10u16.to_le_bytes();
    txn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![&v10_ver[..]],
    )
    .unwrap();
    txn.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
}

/// Build the v10 fixture journal: the curated entry set PLUS a `ReplanRound`
/// (kind 8) AND two `ReactRound` facts (kind 9 — a run-level anchor [step_salt
/// None] and an agentic-step settle [step_salt Some]), then DOWNGRADE the kind-9
/// bodies to the v10 byte shape (strip the trailing v11 `is_agentic_launch` byte)
/// and stamp `metadata.schema_version = 10`. On migration the run-level anchor
/// up-converts to `is_agentic_launch == false` and the agentic settle to
/// `is_agentic_launch == true` (the OLD `step_salt.is_some()` semantics).
fn build_v10_journal(dir: &Path) -> PathBuf {
    let path = dir.join("sample_v10.kxjournal");
    {
        let j = SqliteJournal::open(&path).unwrap();
        let mut entries = curated_v6_entries();
        entries.push(JournalEntry::ReplanRound {
            round: 1,
            shaper_mote_id: MoteId::from_bytes([0x7c; 32]),
            base_prompt_ref: ContentRef::from_bytes([0x11; 32]),
            corrected_prompt_ref: ContentRef::from_bytes([0x22; 32]),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "qwen2-0_5b".to_string(),
            failed_steps: SmallVec::new(),
            escalation_reason_ref: None,
            seq: 0,
        });
        entries.push(JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "qwen2-0_5b".to_string(),
            branch: ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            seq: 0,
        });
        entries.push(JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([0x8e; 32]),
            instance_id: [0x4d; INSTANCE_ID_LEN],
            base_prompt_ref: ContentRef::from_bytes([0x12; 32]),
            warrant_ref: ContentRef::from_bytes([0x34; 32]),
            model_id: "qwen2-0_5b".to_string(),
            branch: ReactBranch::Answer,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: Some([0x77; 32]),
            is_agentic_launch: true,
            context_items_ref: None,
            image_ref: None,
            seq: 0,
        });
        j.append_batch(entries).unwrap();
    }
    downgrade_to_v10(&path);
    path
}

#[test]
fn open_still_refuses_v10_loudly() {
    // The strict open() contract is unchanged by the v11 bump: a v10 journal is
    // refused loudly (migration is the separate, additive path) — the same
    // contract that makes an OLD binary refuse a v11 journal (carrying the new
    // is_agentic_launch byte) rather than misread it.
    let tmp = tempfile::tempdir().unwrap();
    let path = build_v10_journal(tmp.path());
    let err = SqliteJournal::open(&path).unwrap_err();
    assert!(matches!(
        err,
        JournalError::SchemaVersionMismatch { found: 10, expected } if expected == JOURNAL_SCHEMA_VERSION
    ));
}

#[test]
fn migrate_v10_to_current_upconverts_react_launch_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v10_journal(tmp.path());
    let dst = tmp.path().join("migrated_from_v10.kxjournal");

    let report = migrate_to(&src, &dst).unwrap();
    assert_eq!(report.from_version, 10);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.entries_migrated, 10);
    // The two ReactRound facts grow by the trailing is_agentic_launch byte.
    assert_eq!(report.entries_upconverted, 2);

    // Strict open accepts the result; committed facts are byte-identical.
    let j = SqliteJournal::open(&dst).unwrap();
    assert_eq!(j.count_entries().unwrap(), 10);
    let kind_bytes = |p: &Path, kind: i64| -> Vec<Vec<u8>> {
        let conn = Connection::open(p).unwrap();
        let mut stmt = conn
            .prepare("SELECT entry_bytes FROM entries WHERE kind = ?1 ORDER BY seq")
            .unwrap();
        stmt.query_map(params![kind], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(kind_bytes(&src, 1), kind_bytes(&dst, 1)); // committed facts byte-identical

    // The run-level anchor up-converts to is_agentic_launch == false; the agentic
    // settle (step_salt Some) up-converts to is_agentic_launch == true (the OLD
    // step_salt.is_some() discriminator).
    let via_migrate = read_all(&j);
    let react: Vec<(Option<[u8; 32]>, bool)> = via_migrate
        .iter()
        .filter_map(|e| match e {
            JournalEntry::ReactRound {
                step_salt,
                is_agentic_launch,
                ..
            } => Some((*step_salt, *is_agentic_launch)),
            _ => None,
        })
        .collect();
    assert_eq!(react, vec![(None, false), (Some([0x77; 32]), true)]);
}

// ---------------------------------------------------------------------------
// Write-side: migrate_to rewrites a v5 journal into a strict v6 journal
// ---------------------------------------------------------------------------

#[test]
fn migrate_to_produces_strict_v6_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5_journal(tmp.path());
    let dst = tmp.path().join("migrated_v6.kxjournal");

    let report = migrate_to(&src, &dst).unwrap();
    assert_eq!(report.from_version, 5);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.entries_migrated, 7);
    assert_eq!(report.entries_upconverted, 1); // exactly the one cap record

    // The migrated journal is accepted by the STRICT open() (its version is v6).
    let j = SqliteJournal::open(&dst).unwrap();
    assert_eq!(j.count_entries().unwrap(), 7);
}

#[test]
fn migrate_to_matches_replay_readback() {
    // The whole point: rewriting via migrate_to yields exactly the same logical
    // entries as reading the source via the up-converting ReplayJournal.
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5_journal(tmp.path());
    let dst = tmp.path().join("migrated_v6.kxjournal");
    migrate_to(&src, &dst).unwrap();

    let via_replay = read_all(&ReplayJournal::open(&src).unwrap());
    let via_migrate = read_all(&SqliteJournal::open(&dst).unwrap());
    assert_eq!(via_replay, via_migrate);
}

#[test]
fn migrate_to_preserves_committed_facts_byte_identical() {
    // Product identity (committed facts) is invariant across migration: the
    // Committed entries are byte-for-byte unchanged (only kind-6 cap bodies grow).
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5_journal(tmp.path());
    let dst = tmp.path().join("migrated_v6.kxjournal");
    migrate_to(&src, &dst).unwrap();

    let committed_bytes = |p: &Path| -> Vec<Vec<u8>> {
        let conn = Connection::open(p).unwrap();
        let mut stmt = conn
            .prepare("SELECT entry_bytes FROM entries WHERE kind = 1 ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(committed_bytes(&src), committed_bytes(&dst));
}

#[test]
fn migrate_to_preserves_seqs() {
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5_journal(tmp.path());
    let dst = tmp.path().join("migrated_v6.kxjournal");
    migrate_to(&src, &dst).unwrap();

    let src_seqs: Vec<u64> = read_all(&ReplayJournal::open(&src).unwrap())
        .iter()
        .map(JournalEntry::seq)
        .collect();
    let dst_seqs: Vec<u64> = read_all(&SqliteJournal::open(&dst).unwrap())
        .iter()
        .map(JournalEntry::seq)
        .collect();
    assert_eq!(src_seqs, dst_seqs);
    assert_eq!(src_seqs, vec![1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn migrate_to_enables_resume_and_append() {
    // The upgrade story: after migrating, the v6 journal resumes and appends.
    let tmp = tempfile::tempdir().unwrap();
    let src = build_v5_journal(tmp.path());
    let dst = tmp.path().join("migrated_v6.kxjournal");
    migrate_to(&src, &dst).unwrap();

    let j = SqliteJournal::open(&dst).unwrap();
    let appended = j
        .append(JournalEntry::Failed {
            mote_id: MoteId::from_bytes([200u8; 32]),
            idempotency_key: [200u8; 32],
            seq: 0,
            reason_class: FailureReason::WorkerCrashed,
            reporter_id: 7,
        })
        .unwrap();
    assert_eq!(appended.seq(), 8); // continues the sequence after the 7 migrated
    assert_eq!(j.count_entries().unwrap(), 8);
}

#[test]
fn migrate_to_is_idempotent_on_current_version() {
    // Migrating an already-current journal yields a logically-equivalent current
    // journal with nothing up-converted.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("already_v6.kxjournal");
    {
        let j = SqliteJournal::open(&src).unwrap();
        j.append_batch(curated_v6_entries()).unwrap();
    }
    let dst = tmp.path().join("recopied_v6.kxjournal");
    let report = migrate_to(&src, &dst).unwrap();
    assert_eq!(report.from_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);
    assert_eq!(report.entries_upconverted, 0);

    let before = read_all(&SqliteJournal::open(&src).unwrap());
    let after = read_all(&SqliteJournal::open(&dst).unwrap());
    assert_eq!(before, after);
}

// ---------------------------------------------------------------------------
// Scale: migration is O(entries) — resume after upgrade is not an outage
// ---------------------------------------------------------------------------

/// Build a v5 journal of `n` capability-present `RunVersionsResolved` entries
/// (every entry up-converts — the worst case for migration cost).
fn build_v5_cap_journal(dir: &Path, n: u32) -> PathBuf {
    let path = dir.join(format!("scale_{n}.kxjournal"));
    let instance_id = [1u8; INSTANCE_ID_LEN];
    let entries: Vec<JournalEntry> = (0..n)
        .map(|i| JournalEntry::RunVersionsResolved {
            instance_id,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            model_id: "m".to_string(),
            capability: Some(ResolvedCapabilityRecord {
                tool_id: format!("tool-{i}"),
                tool_version: "1".to_string(),
                resolved_kind: ResolvedKindTag::Builtin,
                resolved_def_hash: ContentRef::from_bytes([7u8; 32]),
                idempotency_class: IdempotencyClassTag::Token,
            }),
            seq: 0,
        })
        .collect();
    {
        let j = SqliteJournal::open(&path).unwrap();
        j.append_batch(entries).unwrap();
    }
    downgrade_to_v5(&path);
    path
}

#[test]
#[ignore = "scale: run --release --test schema_evolution -- --ignored --nocapture"]
fn migrate_25k_is_linear() {
    const SIZES: &[u32] = &[1_000, 5_000, 10_000, 25_000];
    let tmp = tempfile::tempdir().unwrap();
    let mut per_entry_us: Vec<f64> = Vec::with_capacity(SIZES.len());

    for &n in SIZES {
        let src = build_v5_cap_journal(tmp.path(), n);
        let dst = tmp.path().join(format!("scale_{n}_v6.kxjournal"));
        let start = Instant::now();
        let report = migrate_to(&src, &dst).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(report.entries_migrated, u64::from(n));
        assert_eq!(report.entries_upconverted, u64::from(n));
        let us = elapsed.as_secs_f64() * 1e6;
        let per = us / f64::from(n);
        per_entry_us.push(per);
        eprintln!(
            "n={n:>6}  migrate={:>9.2}ms  per_entry={per:>7.3}us",
            us / 1e3
        );
    }

    let ratio = per_entry_us.last().unwrap() / per_entry_us.first().unwrap();
    eprintln!("per-entry migrate cost ratio (25k/1k) = {ratio:.2}  (quadratic would be ~25x)");
    if !cfg!(debug_assertions) {
        assert!(
            ratio < 8.0,
            "migrate_to per-entry cost grew {ratio:.1}x (1k->25k) — super-linear; \
             resume-after-upgrade must stay O(entries)"
        );
    }
}
