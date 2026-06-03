//! End-to-end integration tests for the PR 9b-4 `StandardCommitProtocol`
//! `StageThenCommit` path:
//! `journal.append(EffectStaged) → broker.dispatch → R-11 verify →
//! journal.append(Committed)` (D38 §2b).
//!
//! The EffectStaged entry is appended BEFORE broker.dispatch so the
//! recovery fold (per `journal-txn.md` 9-cell cross-product) sees the
//! dispatch intent durably recorded. The tests cover the happy path
//! (both entries land in order) + the failure modes that leave only the
//! staged entry (recovery scenario where re-dispatch may or may not be
//! permitted depending on the subsequent Failed entry's classification).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{CommitInput, CommitProtocol, CommitProtocolError, StandardCommitProtocol};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteDefHash,
    NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

fn wm_stage_then_commit_mote(seed: u8) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

fn empty_request() -> EffectRequest {
    EffectRequest {
        payload: Vec::new(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
        secret_scope: kx_warrant::SecretScope::None,
    }
}

enum BrokerMode {
    HappyPath {
        store: Arc<InMemoryContentStore>,
        response_bytes: Vec<u8>,
    },
    HostileStagedRefMissing,
    DispatchError,
}

struct TestBroker(BrokerMode);

impl std::fmt::Debug for TestBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestBroker").finish()
    }
}

impl CapabilityBroker for TestBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        match &self.0 {
            BrokerMode::HappyPath {
                store,
                response_bytes,
            } => {
                let r = store.put(response_bytes).expect("put");
                Ok(BrokerHandle {
                    staged_ref: r,
                    capability: ToolName("test".into()),
                    capability_version: ToolVersion("0.1.0".into()),
                })
            }
            BrokerMode::HostileStagedRefMissing => Ok(BrokerHandle {
                staged_ref: ContentRef::from_bytes([0xEE; 32]),
                capability: ToolName("hostile".into()),
                capability_version: ToolVersion("0".into()),
            }),
            BrokerMode::DispatchError => Err(BrokerError::SandboxRefused {
                capability: ToolName("test".into()),
                reason: "test-induced dispatch failure".into(),
            }),
        }
    }

    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

fn input_for<'a>(mote: &'a Mote, warrant: &'a WarrantSpec, diagnostic: &'a str) -> CommitInput<'a> {
    CommitInput {
        mote,
        warrant,
        capability: ToolName("test-capability".into()),
        effect_request: empty_request(),
        warrant_ref: ContentRef::from_bytes([4; 32]),
        mote_def_hash: MoteDefHash::from_bytes([3; 32]),
        idempotency_key: [5; 32],
        parents: SmallVec::new(),
        diagnostic_context: diagnostic,
        idempotency_class: None,
    }
}

// ============================================================================
// Happy path: EffectStaged + Committed land in order; broker stages bytes;
// content store has the response.
// ============================================================================

#[test]
fn stage_then_commit_path_commits_correctly() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"stc-response".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

    let mote = wm_stage_then_commit_mote(0x01);
    let warrant = warrant();
    let committed_seq = protocol
        .commit(input_for(&mote, &warrant, "happy-stc"))
        .expect("StageThenCommit must succeed");

    // Two entries: EffectStaged at seq 1, Committed at seq 2 (or whatever
    // the journal assigned, but EffectStaged MUST precede Committed).
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("journal scan")
        .collect();
    assert_eq!(entries.len(), 2, "EffectStaged + Committed must both land");

    let staged_seq = match &entries[0] {
        JournalEntry::EffectStaged { mote_id, seq, .. } => {
            assert_eq!(*mote_id, mote.id);
            *seq
        }
        other => panic!("first entry must be EffectStaged, got {other:?}"),
    };

    match &entries[1] {
        JournalEntry::Committed {
            mote_id,
            seq,
            result_ref,
            ..
        } => {
            assert_eq!(*mote_id, mote.id);
            assert_eq!(*seq, committed_seq);
            assert!(
                *seq > staged_seq,
                "Committed seq ({}) must be > EffectStaged seq ({})",
                seq,
                staged_seq,
            );
            // result_ref points at "stc-response" in the store.
            let bytes = store.get(result_ref).expect("store get");
            assert_eq!(&*bytes, b"stc-response");
        }
        other => panic!("second entry must be Committed, got {other:?}"),
    }
}

// ============================================================================
// EffectStaged entry IS recorded even when broker.dispatch subsequently
// fails. Recovery uses this hint to decide re-dispatch safety (cell 3 vs
// cell 5 of the 9-cell cross-product).
// ============================================================================

#[test]
fn effect_staged_entry_persists_when_broker_dispatch_fails() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::DispatchError));
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

    let mote = wm_stage_then_commit_mote(0x02);
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "broker-fail-after-stage"))
        .expect_err("broker dispatch error must surface");
    assert!(matches!(
        err,
        CommitProtocolError::BrokerDispatchFailed { mote_id, .. } if mote_id == mote.id
    ));

    // The EffectStaged entry must remain — that's the recovery hint.
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("journal scan")
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "EffectStaged must remain; no Committed entry on broker failure",
    );
    assert!(matches!(
        &entries[0],
        JournalEntry::EffectStaged { mote_id, .. } if *mote_id == mote.id,
    ));
    // No Committed yet.
    assert!(journal.read_committed(&mote.id).unwrap().is_none());
}

// ============================================================================
// R-11 fires AFTER EffectStaged has been recorded. The EffectStaged entry
// remains; the Committed entry does not land. Recovery sees this as cell 3
// or cell 5 depending on the subsequent Failed entry's classification (the
// lifecycle layer is responsible for emitting Failed).
// ============================================================================

#[test]
fn r11_fires_after_effect_staged_when_broker_stages_missing_ref() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HostileStagedRefMissing));
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

    let mote = wm_stage_then_commit_mote(0x03);
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "r11-after-stage"))
        .expect_err("R-11 must fire");
    match err {
        CommitProtocolError::R11ResultRefIncomplete {
            mote_id,
            result_ref,
        } => {
            assert_eq!(mote_id, mote.id);
            assert_eq!(result_ref, ContentRef::from_bytes([0xEE; 32]));
        }
        other => panic!("expected R-11, got {other:?}"),
    }

    // EffectStaged remains; no Committed.
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("journal scan")
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0], JournalEntry::EffectStaged { .. }));
    assert!(journal.read_committed(&mote.id).unwrap().is_none());
}

// ============================================================================
// Determinism: two independent protocol instances on a StageThenCommit
// Mote produce stable EffectStaged + Committed shapes.
// ============================================================================

#[test]
fn stage_then_commit_is_deterministic_across_independent_runs() {
    let mote = wm_stage_then_commit_mote(0x04);
    let warrant = warrant();
    let response = b"stable-stc".to_vec();

    let do_commit = || -> (JournalEntry, JournalEntry) {
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
            store: store.clone(),
            response_bytes: response.clone(),
        }));
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
        let _ = protocol
            .commit(input_for(&mote, &warrant, "det-stc"))
            .expect("commit must succeed");
        let entries: Vec<JournalEntry> = journal
            .read_entries_by_seq(0..u64::MAX)
            .expect("scan")
            .collect();
        assert_eq!(entries.len(), 2);
        (entries[0].clone(), entries[1].clone())
    };

    let (s1, c1) = do_commit();
    let (s2, c2) = do_commit();

    // EffectStaged: idempotency_key + mote_id must be stable.
    match (&s1, &s2) {
        (
            JournalEntry::EffectStaged {
                mote_id: m1,
                idempotency_key: k1,
                ..
            },
            JournalEntry::EffectStaged {
                mote_id: m2,
                idempotency_key: k2,
                ..
            },
        ) => {
            assert_eq!(m1, m2);
            assert_eq!(k1, k2);
        }
        _ => panic!("both first entries must be EffectStaged"),
    }

    // Committed: result_ref + idempotency_key + warrant_ref + mote_def_hash
    // must be stable across two independent runs.
    match (&c1, &c2) {
        (
            JournalEntry::Committed {
                result_ref: r1,
                idempotency_key: k1,
                warrant_ref: w1,
                mote_def_hash: h1,
                ..
            },
            JournalEntry::Committed {
                result_ref: r2,
                idempotency_key: k2,
                warrant_ref: w2,
                mote_def_hash: h2,
                ..
            },
        ) => {
            assert_eq!(r1, r2);
            assert_eq!(k1, k2);
            assert_eq!(w1, w2);
            assert_eq!(h1, h2);
        }
        _ => panic!("both second entries must be Committed"),
    }
}

// ============================================================================
// EffectStaged + Committed share the same idempotency_key (v2 dedup index
// {1, 2, 4} treats them as distinct kinds so both land).
// ============================================================================

#[test]
fn effect_staged_and_committed_share_idempotency_key() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"key-share".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

    let mote = wm_stage_then_commit_mote(0x05);
    let warrant = warrant();
    let _ = protocol
        .commit(input_for(&mote, &warrant, "key-share"))
        .expect("commit must succeed");

    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 2);
    let staged_key = match &entries[0] {
        JournalEntry::EffectStaged {
            idempotency_key, ..
        } => *idempotency_key,
        _ => panic!("first must be EffectStaged"),
    };
    let committed_key = match &entries[1] {
        JournalEntry::Committed {
            idempotency_key, ..
        } => *idempotency_key,
        _ => panic!("second must be Committed"),
    };
    assert_eq!(
        staged_key, committed_key,
        "EffectStaged + Committed must share idempotency_key for dedup index {{1,2,4}}",
    );
}
