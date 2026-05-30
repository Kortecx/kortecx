//! End-to-end integration tests for the PR 9b-3 `StandardCommitProtocol`
//! `IdempotentByConstruction` path:
//! `broker.dispatch → R-11 verify → journal.append(Committed)` (D39 §a/§c).
//!
//! The tests wire a real `InMemoryContentStore` + `InMemoryJournal` + a
//! custom test `CapabilityBroker` that exercises both the happy path and
//! the R-11 enforcement edge cases (hostile broker returning a staged_ref
//! without actually staging the bytes).

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

// ============================================================================
// Test fixtures
// ============================================================================

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
    }
}

fn wm_idempotent_mote(seed: u8) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
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
        pattern: EffectPattern::IdempotentByConstruction,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

// ============================================================================
// Test broker — exercises happy path + R-11 hostile path + error paths.
// ============================================================================

/// Behavior switch for the test broker. The broker shares the same
/// `Arc<InMemoryContentStore>` as the protocol-under-test for the happy
/// path; the hostile / failing modes deliberately desync.
enum BrokerMode {
    /// `dispatch` puts `response_bytes` into the shared store + returns
    /// the resulting ref in the `BrokerHandle`.
    HappyPath {
        store: Arc<InMemoryContentStore>,
        response_bytes: Vec<u8>,
    },
    /// `dispatch` returns a `BrokerHandle` with a fabricated `staged_ref`
    /// that was NEVER put into the protocol's store. R-11 must fire.
    HostileStagedRefMissing,
    /// `dispatch` returns a `BrokerError` directly (e.g., the capability
    /// failed). Protocol must wrap as `BrokerDispatchFailed`.
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
                    capability: ToolName("test-capability".into()),
                    capability_version: ToolVersion("0.1.0".into()),
                })
            }
            BrokerMode::HostileStagedRefMissing => Ok(BrokerHandle {
                staged_ref: ContentRef::from_bytes([0xDD; 32]),
                capability: ToolName("hostile".into()),
                capability_version: ToolVersion("0".into()),
            }),
            BrokerMode::DispatchError => Err(BrokerError::SandboxRefused {
                capability: ToolName("test-capability".into()),
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
    }
}

// ============================================================================
// Happy path: IdempotentByConstruction commits successfully.
// ============================================================================

#[test]
fn idempotent_path_commits_and_appends_committed_entry() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"test-response-payload".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

    let mote = wm_idempotent_mote(0x01);
    let warrant = warrant();
    let seq = protocol
        .commit(input_for(&mote, &warrant, "happy-path"))
        .expect("idempotent commit must succeed");

    // The journal has one Committed entry at the returned seq.
    assert!(seq > 0, "journal must assign a non-zero seq");
    let entry = journal
        .read_committed(&mote.id)
        .expect("journal read")
        .expect("Committed entry must exist after commit");
    match entry {
        JournalEntry::Committed {
            mote_id, seq: s, ..
        } => {
            assert_eq!(mote_id, mote.id);
            assert_eq!(s, seq);
        }
        other => panic!("expected Committed entry, got {other:?}"),
    }

    // The content store has the response bytes at the result_ref.
    assert_eq!(store.len(), 1, "content store has the broker's response");
}

// ============================================================================
// R-11: hostile broker returns a staged_ref that's not in the store.
// ============================================================================

#[test]
fn r11_fires_when_broker_stages_a_ref_not_in_the_store() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HostileStagedRefMissing));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

    let mote = wm_idempotent_mote(0x02);
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "r11-test"))
        .expect_err("R-11 must fire when broker desyncs from store");
    match err {
        CommitProtocolError::R11ResultRefIncomplete {
            mote_id,
            result_ref,
        } => {
            assert_eq!(mote_id, mote.id);
            assert_eq!(result_ref, ContentRef::from_bytes([0xDD; 32]));
        }
        other => panic!("expected R-11, got {other:?}"),
    }

    // R-11 fired BEFORE the journal append; no Committed entry exists.
    assert!(
        journal.read_committed(&mote.id).unwrap().is_none(),
        "R-11 must short-circuit before journal.append"
    );
}

// ============================================================================
// BrokerDispatchFailed: broker.dispatch returns a typed error.
// ============================================================================

#[test]
fn broker_dispatch_error_wraps_as_broker_dispatch_failed() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::DispatchError));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

    let mote = wm_idempotent_mote(0x03);
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "dispatch-err"))
        .expect_err("dispatch error must surface");
    assert!(matches!(
        err,
        CommitProtocolError::BrokerDispatchFailed { mote_id, .. } if mote_id == mote.id
    ));
    assert!(
        journal.read_committed(&mote.id).unwrap().is_none(),
        "dispatch error must not write a Committed entry"
    );
}

// ============================================================================
// StageThenCommit returns Internal { reason: "PR 9b-4 ..." } stub.
// ============================================================================

// Note: the StageThenCommit path now ships in PR 9b-4; this stub-shape
// test is superseded by `tests/integration_stage_then_commit.rs`.
#[test]
#[ignore = "superseded by integration_stage_then_commit::stage_then_commit_path_commits_correctly (PR 9b-4 ships the path)"]
fn stage_then_commit_returns_internal_pr_9b_4_placeholder() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"unused".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store, journal, broker);

    let mut mote = wm_idempotent_mote(0x04);
    let mut def = mote.def.clone();
    def.effect_pattern = EffectPattern::StageThenCommit;
    mote = Mote::new(
        def,
        mote.input_data_id,
        mote.graph_position.clone(),
        mote.parents.clone(),
    );
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "stage-then-commit"))
        .expect_err("StageThenCommit is unimplemented in PR 9b-3");
    match err {
        CommitProtocolError::Internal { mote_id, reason } => {
            assert_eq!(mote_id, mote.id);
            assert!(
                reason.contains("PR 9b-4"),
                "stub must reference PR 9b-4 for forward visibility; got: {reason}",
            );
        }
        other => panic!("expected Internal stub, got {other:?}"),
    }
}

// ============================================================================
// ValidateThenCommit returns Internal { reason: "PR 9b-5 ..." } stub.
// ============================================================================

// Note: the ValidateThenCommit path now ships in PR 9b-5; this stub-shape
// test is superseded by `tests/integration_validate_then_commit.rs`.
#[test]
#[ignore = "superseded by integration_validate_then_commit::validate_then_commit_path_commits_correctly (PR 9b-5 ships the path)"]
fn validate_then_commit_returns_internal_pr_9b_5_placeholder() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
        store: store.clone(),
        response_bytes: b"unused".to_vec(),
    }));
    let protocol = StandardCommitProtocol::new(store, journal, broker);

    let mut mote = wm_idempotent_mote(0x05);
    let mut def = mote.def.clone();
    def.effect_pattern = EffectPattern::ValidateThenCommit;
    mote = Mote::new(
        def,
        mote.input_data_id,
        mote.graph_position.clone(),
        mote.parents.clone(),
    );
    let warrant = warrant();
    let err = protocol
        .commit(input_for(&mote, &warrant, "validate-then-commit"))
        .expect_err("ValidateThenCommit is unimplemented in PR 9b-3");
    match err {
        CommitProtocolError::Internal { mote_id, reason } => {
            assert_eq!(mote_id, mote.id);
            assert!(
                reason.contains("PR 9b-5"),
                "stub must reference PR 9b-5 for forward visibility; got: {reason}",
            );
        }
        other => panic!("expected Internal stub, got {other:?}"),
    }
}

// ============================================================================
// Determinism: same input → same outcome (the journal-assigned seq may
// differ on re-run since journal is in-memory and seq is per-run-monotonic;
// the committed entry's mote_id + result_ref must be stable across
// independent protocol instances).
// ============================================================================

#[test]
fn two_independent_protocol_instances_produce_stable_committed_shape() {
    let mote = wm_idempotent_mote(0x06);
    let warrant = warrant();
    let response = b"stable-payload".to_vec();

    let do_commit = || -> JournalEntry {
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(TestBroker(BrokerMode::HappyPath {
            store: store.clone(),
            response_bytes: response.clone(),
        }));
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
        let _ = protocol
            .commit(input_for(&mote, &warrant, "determinism"))
            .expect("commit must succeed");
        journal.read_committed(&mote.id).unwrap().unwrap()
    };

    let e1 = do_commit();
    let e2 = do_commit();
    // mote_id + result_ref + warrant_ref + mote_def_hash + idempotency_key
    // are deterministic; seq is per-run-monotonic and starts at 1 here so
    // it should also match for two empty-journal-prefix scenarios.
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
            assert_eq!(r1, r2, "result_ref must be deterministic");
            assert_eq!(k1, k2, "idempotency_key must be deterministic");
            assert_eq!(w1, w2, "warrant_ref must be deterministic");
            assert_eq!(h1, h2, "mote_def_hash must be deterministic");
        }
        _ => panic!("both entries must be Committed"),
    }
}
