//! End-to-end integration tests for the PR 9b-6 `run_wm_mote` lifecycle
//! orchestrator. Exercises all three EffectPattern paths through the
//! full lifecycle: acquire → Proposed → commit_protocol → critic
//! scheduling (for ValidateThenCommit) → release.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{run_wm_mote, LifecycleError, LocalResourceManager, StandardCommitProtocol};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

// ============================================================================
// Fixtures
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

fn mote(seed: u8, pattern: EffectPattern, nd: NdClass, critic_for: Option<MoteId>) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: nd,
        config_subset: BTreeMap::new(),
        effect_pattern: pattern,
        critic_for,
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

fn empty_request(pattern: EffectPattern) -> EffectRequest {
    EffectRequest {
        payload: Vec::new(),
        pattern,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

struct HappyBroker {
    store: Arc<InMemoryContentStore>,
    response_bytes: Vec<u8>,
}
impl std::fmt::Debug for HappyBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HappyBroker").finish()
    }
}
impl CapabilityBroker for HappyBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        let r = self.store.put(&self.response_bytes).expect("put");
        Ok(BrokerHandle {
            staged_ref: r,
            capability: ToolName("happy".into()),
            capability_version: ToolVersion("0.1.0".into()),
        })
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

// ============================================================================
// IdempotentByConstruction lifecycle: acquire → Proposed → broker.dispatch →
// R-11 → Committed → release. NO critic; no EffectStaged.
// ============================================================================

#[test]
fn idempotent_by_construction_lifecycle_end_to_end() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        response_bytes: b"ibc-resp".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let producer = mote(
        0x01,
        EffectPattern::IdempotentByConstruction,
        NdClass::WorldMutating,
        None,
    );
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((producer.id, producer.clone())).collect();
    let w = warrant();

    let result = run_wm_mote(
        &producer,
        &w,
        ToolName("ibc-cap".into()),
        empty_request(EffectPattern::IdempotentByConstruction),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect("IdempotentByConstruction lifecycle must succeed");

    assert_eq!(result.mote_id, producer.id);
    assert!(result.critic_proposed_seq.is_none(), "no critic for IBC");

    // Journal: Proposed (producer) + Committed (producer). No EffectStaged.
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 2, "IBC writes Proposed + Committed only");
    assert!(matches!(&entries[0], JournalEntry::Proposed { .. }));
    assert!(matches!(&entries[1], JournalEntry::Committed { .. }));
}

// ============================================================================
// StageThenCommit lifecycle: acquire → Proposed → EffectStaged → broker.dispatch
// → R-11 → Committed → release. NO critic.
// ============================================================================

#[test]
fn stage_then_commit_lifecycle_writes_proposed_effect_staged_and_committed() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        response_bytes: b"stc-resp".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let producer = mote(
        0x02,
        EffectPattern::StageThenCommit,
        NdClass::WorldMutating,
        None,
    );
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((producer.id, producer.clone())).collect();
    let w = warrant();

    let result = run_wm_mote(
        &producer,
        &w,
        ToolName("stc-cap".into()),
        empty_request(EffectPattern::StageThenCommit),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect("StageThenCommit lifecycle must succeed");

    assert_eq!(result.mote_id, producer.id);
    assert!(result.critic_proposed_seq.is_none(), "no critic for STC");

    // Journal: Proposed → EffectStaged → Committed.
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 3);
    assert!(matches!(&entries[0], JournalEntry::Proposed { .. }));
    assert!(matches!(&entries[1], JournalEntry::EffectStaged { .. }));
    assert!(matches!(&entries[2], JournalEntry::Committed { .. }));
}

// ============================================================================
// ValidateThenCommit lifecycle: acquire → Proposed (producer) →
// broker.dispatch → R-11 → Committed → Proposed (critic) → release. NO
// EffectStaged.
// ============================================================================

#[test]
fn validate_then_commit_lifecycle_schedules_critic_proposed_entry() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        response_bytes: b"vtc-resp".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let producer = mote(
        0x03,
        EffectPattern::ValidateThenCommit,
        NdClass::WorldMutating,
        None,
    );
    let critic = mote(
        0x04,
        EffectPattern::IdempotentByConstruction,
        NdClass::Pure, // critics must be Pure-terminating per R-9
        Some(producer.id),
    );
    let mut submission_motes = BTreeMap::new();
    submission_motes.insert(producer.id, producer.clone());
    submission_motes.insert(critic.id, critic.clone());
    let w = warrant();

    let result = run_wm_mote(
        &producer,
        &w,
        ToolName("vtc-cap".into()),
        empty_request(EffectPattern::ValidateThenCommit),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect("ValidateThenCommit lifecycle must succeed");

    assert_eq!(result.mote_id, producer.id);
    let critic_seq = result
        .critic_proposed_seq
        .expect("critic must be scheduled");
    assert!(critic_seq > result.committed_seq);

    // Journal: Proposed(producer) → Committed(producer) → Proposed(critic).
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 3);
    match &entries[0] {
        JournalEntry::Proposed { mote_id, .. } => assert_eq!(*mote_id, producer.id),
        other => panic!("entry 0 should be producer Proposed; got {other:?}"),
    }
    match &entries[1] {
        JournalEntry::Committed { mote_id, .. } => assert_eq!(*mote_id, producer.id),
        other => panic!("entry 1 should be producer Committed; got {other:?}"),
    }
    match &entries[2] {
        JournalEntry::Proposed { mote_id, seq, .. } => {
            assert_eq!(*mote_id, critic.id);
            assert_eq!(*seq, critic_seq);
        }
        other => panic!("entry 2 should be critic Proposed; got {other:?}"),
    }
}

// ============================================================================
// ValidateThenCommit without a sibling critic in the submission yields
// `critic_proposed_seq = None`. (R-2 should refuse this submission earlier;
// this test verifies the lifecycle's defensive handling of an unexpected
// submission map.)
// ============================================================================

#[test]
fn validate_then_commit_without_critic_returns_none_critic_seq() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        response_bytes: b"vtc-no-critic".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let producer = mote(
        0x05,
        EffectPattern::ValidateThenCommit,
        NdClass::WorldMutating,
        None,
    );
    // submission_motes contains ONLY the producer; no sibling critic.
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((producer.id, producer.clone())).collect();
    let w = warrant();

    let result = run_wm_mote(
        &producer,
        &w,
        ToolName("vtc-no-critic-cap".into()),
        empty_request(EffectPattern::ValidateThenCommit),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect("commit must succeed even without critic in defensive path");

    assert_eq!(result.mote_id, producer.id);
    assert!(
        result.critic_proposed_seq.is_none(),
        "no sibling critic → no critic Proposed entry"
    );

    // Producer's Proposed + Committed land; no extra Proposed.
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 2);
}

// ============================================================================
// Refusing a PURE Mote: `run_wm_mote` must reject PURE Motes (caller is
// expected to use `run_pure_mote`).
// ============================================================================

#[test]
fn run_wm_mote_refuses_pure_motes() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        response_bytes: b"unused".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let pure_mote = mote(
        0x06,
        EffectPattern::IdempotentByConstruction,
        NdClass::Pure,
        None,
    );
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((pure_mote.id, pure_mote.clone())).collect();
    let w = warrant();

    let err = run_wm_mote(
        &pure_mote,
        &w,
        ToolName("never-called".into()),
        empty_request(EffectPattern::IdempotentByConstruction),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect_err("PURE Motes must be refused");
    assert!(matches!(err, LifecycleError::Internal(_)));
    // No journal entries — the refusal short-circuits before any append.
    assert_eq!(journal.count_entries().unwrap(), 0);
}
