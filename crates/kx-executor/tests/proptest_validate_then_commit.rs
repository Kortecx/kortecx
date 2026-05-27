//! Property tests on the PR 9b-5 `StandardCommitProtocol`
//! `ValidateThenCommit` path. SN-4 v2 mandate: ≥3 proptest properties ×
//! 64 cases.
//!
//! Properties:
//! 1. Single-entry commit — the happy path appends exactly one Committed
//!    entry; NO EffectStaged entry (load-bearing structural distinction
//!    from StageThenCommit).
//! 2. R-11 leaves journal empty (no EffectStaged hint to remain, unlike
//!    StageThenCommit).
//! 3. Determinism — same input → byte-identical Committed entry across
//!    two independent protocol instances.

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
use proptest::prelude::*;
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

fn wm_vtc_mote(seed: u8) -> Mote {
    let def = MoteDef {
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
            capability: ToolName("happy-vtc".into()),
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

struct HostileBroker {
    fake_ref: ContentRef,
}
impl std::fmt::Debug for HostileBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostileBroker").finish()
    }
}
impl CapabilityBroker for HostileBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        Ok(BrokerHandle {
            staged_ref: self.fake_ref,
            capability: ToolName("hostile".into()),
            capability_version: ToolVersion("0".into()),
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

fn input_for<'a>(
    mote: &'a Mote,
    warrant: &'a WarrantSpec,
    idempotency_key: [u8; 32],
) -> CommitInput<'a> {
    CommitInput {
        mote,
        warrant,
        capability: ToolName("test".into()),
        effect_request: empty_request(),
        warrant_ref: ContentRef::from_bytes([4; 32]),
        mote_def_hash: MoteDefHash::from_bytes([3; 32]),
        idempotency_key,
        parents: SmallVec::new(),
        diagnostic_context: "proptest",
    }
}

proptest! {
    /// Single-entry commit: the happy ValidateThenCommit path appends
    /// exactly one Committed entry; NO EffectStaged entry. This is the
    /// load-bearing structural distinction from StageThenCommit.
    #[test]
    fn prop_validate_then_commit_appends_exactly_one_committed(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
        response in any::<Vec<u8>>().prop_filter("non-empty", |v| !v.is_empty()),
    ) {
        let mote = wm_vtc_mote(seed);
        let w = warrant();
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HappyBroker {
            store: store.clone(),
            response_bytes: response,
        });
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let _ = protocol
            .commit(input_for(&mote, &w, idempotency_key))
            .expect("happy commit must succeed");

        let entries: Vec<JournalEntry> = journal
            .read_entries_by_seq(0..u64::MAX)
            .expect("scan").collect();
        prop_assert_eq!(entries.len(), 1);
        let is_committed = matches!(entries[0], JournalEntry::Committed { .. });
        let is_effect_staged = matches!(entries[0], JournalEntry::EffectStaged { .. });
        prop_assert!(is_committed);
        prop_assert!(!is_effect_staged);
    }

    /// R-11 leaves the journal empty (no EffectStaged hint, unlike the
    /// StageThenCommit path). The hostile broker fabricates a ref; R-11
    /// fires before any journal append.
    #[test]
    fn prop_r11_in_vtc_path_leaves_journal_empty(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
        fake_ref_bytes in any::<[u8; 32]>(),
    ) {
        let mote = wm_vtc_mote(seed);
        let w = warrant();
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HostileBroker {
            fake_ref: ContentRef::from_bytes(fake_ref_bytes),
        });
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let err = protocol.commit(input_for(&mote, &w, idempotency_key))
            .expect_err("R-11 must fire");
        let is_r11 = matches!(err, CommitProtocolError::R11ResultRefIncomplete { .. });
        prop_assert!(is_r11);

        prop_assert_eq!(journal.count_entries().unwrap(), 0);
    }

    /// Determinism: two independent protocol instances on the same
    /// ValidateThenCommit input produce byte-identical Committed
    /// entries.
    #[test]
    fn prop_validate_then_commit_is_deterministic(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
        response in any::<Vec<u8>>().prop_filter("non-empty", |v| !v.is_empty()),
    ) {
        let mote = wm_vtc_mote(seed);
        let w = warrant();

        let run = || -> JournalEntry {
            let store = Arc::new(InMemoryContentStore::new());
            let journal = Arc::new(InMemoryJournal::new());
            let broker = Arc::new(HappyBroker {
                store: store.clone(),
                response_bytes: response.clone(),
            });
            let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
            let _ = protocol.commit(input_for(&mote, &w, idempotency_key))
                .expect("commit must succeed");
            journal.read_committed(&mote.id).unwrap().unwrap()
        };

        let e1 = run();
        let e2 = run();
        prop_assert_eq!(&e1, &e2);
    }
}
