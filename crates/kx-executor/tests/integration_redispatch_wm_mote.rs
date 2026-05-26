//! End-to-end integration tests for the PR 9b-7 `redispatch_wm_mote`
//! recovery-time orchestrator. Verifies that R-13's oracle consultation
//! fires BEFORE any broker dispatch, that the journal stays unchanged
//! on refusal, and that the approve-path produces the same Committed
//! shape as the fresh-dispatch `run_wm_mote`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{
    redispatch_wm_mote, CommitProtocolError, LifecycleError, LocalResourceManager,
    StandardCommitProtocol, WmRedispatchOracle,
};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
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
    }
}

fn wm_mote(seed: u8, pattern: EffectPattern) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: pattern,
        critic_for: None,
        is_topology_shaper: false,
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

/// Test oracle with a fixed answer.
struct StubOracle {
    can_redispatch: bool,
    /// Tracks whether the oracle was actually consulted (PR 9b-7's
    /// load-bearing assertion: R-13 fires BEFORE broker.dispatch).
    consulted: AtomicBool,
}

impl WmRedispatchOracle for StubOracle {
    fn can_redispatch_world_effect(&self, _mote_id: &MoteId) -> bool {
        self.consulted.store(true, Ordering::SeqCst);
        self.can_redispatch
    }
}

struct HappyBroker {
    store: Arc<InMemoryContentStore>,
    /// Tracks whether dispatch was actually called (must be false on
    /// the refusal path; R-13 must short-circuit before the broker).
    dispatched: AtomicBool,
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
        self.dispatched.store(true, Ordering::SeqCst);
        let r = self.store.put(b"redispatch-resp").expect("put");
        Ok(BrokerHandle {
            staged_ref: r,
            capability: ToolName("redispatch".into()),
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
// R-13: oracle refuses → R13WmReDispatchRefused; broker not consulted;
// journal unchanged.
// ============================================================================

#[test]
fn r13_fires_before_broker_dispatch_when_oracle_refuses() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        dispatched: AtomicBool::new(false),
    });
    let broker_ref = broker.clone();
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let oracle = StubOracle {
        can_redispatch: false,
        consulted: AtomicBool::new(false),
    };

    let mote = wm_mote(0x01, EffectPattern::StageThenCommit);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();

    let err = redispatch_wm_mote(
        &mote,
        &warrant(),
        ToolName("test".into()),
        empty_request(EffectPattern::StageThenCommit),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &oracle,
    )
    .expect_err("R-13 must fire");

    // R-13 surfaces through LifecycleError::CommitProtocol(R13...)
    match err {
        LifecycleError::CommitProtocol(CommitProtocolError::R13WmReDispatchRefused {
            mote_id,
            reason,
        }) => {
            assert_eq!(mote_id, mote.id);
            assert!(reason.contains("Oracle") || reason.contains("oracle"));
        }
        other => panic!("expected R-13 CommitProtocol error, got {other:?}"),
    }

    // Load-bearing: oracle WAS consulted; broker WAS NOT.
    assert!(oracle.consulted.load(Ordering::SeqCst));
    assert!(
        !broker_ref.dispatched.load(Ordering::SeqCst),
        "R-13 must short-circuit BEFORE broker.dispatch",
    );

    // Journal is unchanged — R-13 fires before any append.
    assert_eq!(journal.count_entries().unwrap(), 0);
    // Content store is unchanged too.
    assert_eq!(store.len(), 0);
}

// ============================================================================
// Oracle approves → commit_protocol proceeds → Committed entry lands.
// No fresh Proposed entry on the recovery path (the previous attempt's
// Proposed is in the journal already; this test starts from a
// fresh-state journal for simplicity, asserting only the no-fresh-Proposed
// invariant for THIS function's contract).
// ============================================================================

#[test]
fn oracle_approves_then_commit_protocol_proceeds_no_fresh_proposed() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        dispatched: AtomicBool::new(false),
    });
    let broker_ref = broker.clone();
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let oracle = StubOracle {
        can_redispatch: true,
        consulted: AtomicBool::new(false),
    };

    let mote = wm_mote(0x02, EffectPattern::IdempotentByConstruction);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();

    let result = redispatch_wm_mote(
        &mote,
        &warrant(),
        ToolName("test".into()),
        empty_request(EffectPattern::IdempotentByConstruction),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &oracle,
    )
    .expect("oracle-approved re-dispatch must succeed");

    assert_eq!(result.mote_id, mote.id);
    assert!(oracle.consulted.load(Ordering::SeqCst));
    assert!(broker_ref.dispatched.load(Ordering::SeqCst));

    // The journal has exactly one entry: the Committed produced by the
    // commit_protocol. NO fresh Proposed (PR 9b-7's contract: recovery
    // path trusts the existing Proposed from the previous attempt).
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 1, "recovery path appends Committed only");
    assert!(matches!(&entries[0], JournalEntry::Committed { .. }));
}

// ============================================================================
// PURE Mote refused on recovery path same as fresh path.
// ============================================================================

#[test]
fn redispatch_wm_mote_refuses_pure_motes() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        dispatched: AtomicBool::new(false),
    });
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let oracle = StubOracle {
        can_redispatch: true,
        consulted: AtomicBool::new(false),
    };

    let mut pure = wm_mote(0x03, EffectPattern::IdempotentByConstruction);
    let mut def = pure.def.clone();
    def.nd_class = NdClass::Pure;
    pure = Mote::new(
        def,
        pure.input_data_id,
        pure.graph_position.clone(),
        pure.parents.clone(),
    );
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((pure.id, pure.clone())).collect();

    let err = redispatch_wm_mote(
        &pure,
        &warrant(),
        ToolName("never-called".into()),
        empty_request(EffectPattern::IdempotentByConstruction),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &oracle,
    )
    .expect_err("PURE Mote must be refused on recovery path");
    assert!(matches!(err, LifecycleError::Internal(_)));
    assert_eq!(journal.count_entries().unwrap(), 0);
    // Oracle not consulted — PURE refusal short-circuits earlier.
    assert!(!oracle.consulted.load(Ordering::SeqCst));
}

// ============================================================================
// Oracle approval + ValidateThenCommit producer → critic Proposed entry
// lands (same as run_wm_mote).
// ============================================================================

#[test]
fn redispatch_validate_then_commit_schedules_critic_proposed() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
        dispatched: AtomicBool::new(false),
    });
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let oracle = StubOracle {
        can_redispatch: true,
        consulted: AtomicBool::new(false),
    };

    let producer = wm_mote(0x04, EffectPattern::ValidateThenCommit);
    // Critic Mote with critic_for = producer.id, Pure-class per R-9.
    let critic_def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: Some(producer.id),
        is_topology_shaper: false,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    let critic = Mote::new(
        critic_def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![0x05]),
        SmallVec::new(),
    );
    let mut submission_motes = BTreeMap::new();
    submission_motes.insert(producer.id, producer.clone());
    submission_motes.insert(critic.id, critic.clone());

    let result = redispatch_wm_mote(
        &producer,
        &warrant(),
        ToolName("test".into()),
        empty_request(EffectPattern::ValidateThenCommit),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &oracle,
    )
    .expect("re-dispatch must succeed");

    let critic_seq = result
        .critic_proposed_seq
        .expect("critic must be scheduled");
    assert!(critic_seq > result.committed_seq);

    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(
        entries.len(),
        2,
        "recovery path: Committed(producer) + Proposed(critic)"
    );
    assert!(matches!(&entries[0], JournalEntry::Committed { .. }));
    assert!(matches!(&entries[1], JournalEntry::Proposed { .. }));
}
