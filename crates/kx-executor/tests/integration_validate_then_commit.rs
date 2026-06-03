//! End-to-end integration tests for the PR 9b-5 `StandardCommitProtocol`
//! `ValidateThenCommit` path:
//! `broker.dispatch → R-11 verify → journal.append(Committed)` (D39
//! §a/§c + D20).
//!
//! The commit-step semantics are identical to `IdempotentByConstruction`
//! — the producer Mote's `Committed` entry lands the same way. The
//! distinction is at scheduling: the producer requires a sibling critic
//! Mote whose own commit (or repudiation) gates downstream consumers per
//! D20. **Critic-Mote child scheduling is the lifecycle layer's
//! responsibility** (lands in PR 9b-6+); these tests verify only the
//! commit-step behavior.

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

fn wm_validate_then_commit_mote(seed: u8) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::ValidateThenCommit,
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
        pattern: EffectPattern::ValidateThenCommit,
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
                staged_ref: ContentRef::from_bytes([0xFE; 32]),
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
// Happy path: ValidateThenCommit commits the producer; no EffectStaged
// entry (unlike StageThenCommit); exactly one Committed entry lands.
// ============================================================================

#[test]
fn validate_then_commit_path_commits_correctly() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"vtc-response".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

    let mote = wm_validate_then_commit_mote(0x10);
    let warrant = warrant();
    let seq = protocol
        .commit(input_for(&mote, &warrant, "happy-vtc"))
        .expect("ValidateThenCommit must succeed");

    // Exactly one entry — the Committed for the producer. No EffectStaged
    // (per the path's contract; unlike StageThenCommit).
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "ValidateThenCommit appends exactly one Committed (no EffectStaged); critic scheduling is lifecycle's job",
    );
    match &entries[0] {
        JournalEntry::Committed {
            mote_id,
            seq: s,
            result_ref,
            ..
        } => {
            assert_eq!(*mote_id, mote.id);
            assert_eq!(*s, seq);
            let bytes = store.get(result_ref).expect("store get");
            assert_eq!(&*bytes, b"vtc-response");
        }
        other => panic!("expected Committed, got {other:?}"),
    }
}

// ============================================================================
// R-11 fires same as IdempotentByConstruction / StageThenCommit.
// ============================================================================

#[test]
fn r11_fires_on_validate_then_commit_when_broker_stages_missing_ref() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HostileStagedRefMissing));
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

    let mote = wm_validate_then_commit_mote(0x11);
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "r11-vtc"))
        .expect_err("R-11 must fire");
    match err {
        CommitProtocolError::R11ResultRefIncomplete {
            mote_id,
            result_ref,
        } => {
            assert_eq!(mote_id, mote.id);
            assert_eq!(result_ref, ContentRef::from_bytes([0xFE; 32]));
        }
        other => panic!("expected R-11, got {other:?}"),
    }

    // No Committed entry — and no EffectStaged either (unlike StageThenCommit
    // where R-11 leaves the EffectStaged hint behind).
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(
        entries.len(),
        0,
        "ValidateThenCommit R-11 must leave the journal empty (no EffectStaged in this pattern)",
    );
}

// ============================================================================
// BrokerDispatchFailed wraps broker errors.
// ============================================================================

#[test]
fn broker_dispatch_error_wraps_as_broker_dispatch_failed_on_validate_then_commit() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::DispatchError));
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

    let mote = wm_validate_then_commit_mote(0x12);
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "dispatch-err-vtc"))
        .expect_err("dispatch error must surface");
    assert!(matches!(
        err,
        CommitProtocolError::BrokerDispatchFailed { mote_id, .. } if mote_id == mote.id
    ));
    assert!(journal.read_committed(&mote.id).unwrap().is_none());
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(
        entries.len(),
        0,
        "ValidateThenCommit dispatch error must leave the journal empty",
    );
}

// ============================================================================
// Determinism: same input → byte-identical Committed entry across two
// independent protocol instances.
// ============================================================================

#[test]
fn validate_then_commit_is_deterministic_across_independent_runs() {
    let mote = wm_validate_then_commit_mote(0x13);
    let warrant = warrant();
    let response = b"stable-vtc".to_vec();

    let do_commit = || -> JournalEntry {
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
            store: store.clone(),
            response_bytes: response.clone(),
        }));
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
        let _ = protocol
            .commit(input_for(&mote, &warrant, "det-vtc"))
            .expect("commit must succeed");
        journal.read_committed(&mote.id).unwrap().unwrap()
    };

    let e1 = do_commit();
    let e2 = do_commit();
    match (&e1, &e2) {
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
        _ => panic!("both entries must be Committed"),
    }
}

// ============================================================================
// Structural difference from StageThenCommit: NO EffectStaged entry on
// the happy path. This is the load-bearing distinction at the journal
// layer between the two patterns. (D20: the critic-Mote relationship
// supersedes the EffectStaged hint for ValidateThenCommit's recovery
// semantics; downstream consumers consult the critic's commit state, not
// the producer's alone.)
// ============================================================================

#[test]
fn validate_then_commit_does_not_append_effect_staged() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"no-staged".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

    let mote = wm_validate_then_commit_mote(0x14);
    let warrant = warrant();
    let _ = protocol
        .commit(input_for(&mote, &warrant, "no-staged-vtc"))
        .expect("commit must succeed");

    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(
        !matches!(&entries[0], JournalEntry::EffectStaged { .. }),
        "ValidateThenCommit must NOT append EffectStaged (that's StageThenCommit-only)",
    );
    assert!(
        matches!(&entries[0], JournalEntry::Committed { .. }),
        "the one entry must be Committed",
    );
}
