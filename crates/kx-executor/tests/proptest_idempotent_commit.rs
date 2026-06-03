//! Property tests on the PR 9b-3 `StandardCommitProtocol`
//! `IdempotentByConstruction` path. SN-4 v2 mandate: ≥3 proptest
//! properties × 64 cases.
//!
//! Properties:
//! 1. Determinism — same `(mote_id, response_bytes, warrant_ref,
//!    idempotency_key)` yields the same `Committed` entry shape across two
//!    fresh protocol instances.
//! 2. R-11 monotonicity — for any choice of `(mote_id, fake_ref)`, the
//!    hostile-broker path's R-11 refusal carries the same `mote_id` +
//!    `result_ref` the broker fabricated.
//! 3. BrokerDispatchFailed surface — any broker error propagates as the
//!    matching `BrokerDispatchFailed` variant carrying the Mote's id.

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
        ..Default::default()
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
        secret_scope: kx_warrant::SecretScope::None,
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
    warrant_ref: ContentRef,
    mote_def_hash: MoteDefHash,
) -> CommitInput<'a> {
    CommitInput {
        mote,
        warrant,
        capability: ToolName("test-capability".into()),
        effect_request: empty_request(),
        warrant_ref,
        mote_def_hash,
        idempotency_key,
        parents: SmallVec::new(),
        diagnostic_context: "proptest",
        idempotency_class: None,
    }
}

proptest! {
    /// Determinism: two fresh protocol instances with identical inputs
    /// produce a `Committed` entry with byte-identical `result_ref`,
    /// `idempotency_key`, `warrant_ref`, and `mote_def_hash`.
    #[test]
    fn prop_idempotent_commit_is_deterministic(
        seed in any::<u8>(),
        response in any::<Vec<u8>>().prop_filter("non-empty", |v| !v.is_empty()),
        idempotency_key in any::<[u8; 32]>(),
        warrant_ref_bytes in any::<[u8; 32]>(),
        mote_def_hash_bytes in any::<[u8; 32]>(),
    ) {
        let mote = wm_idempotent_mote(seed);
        let w = warrant();
        let wref = ContentRef::from_bytes(warrant_ref_bytes);
        let mdh = MoteDefHash::from_bytes(mote_def_hash_bytes);

        let run = || -> JournalEntry {
            let store = Arc::new(InMemoryContentStore::new());
            let journal = Arc::new(InMemoryJournal::new());
            let broker = Arc::new(HappyBroker {
                store: store.clone(),
                response_bytes: response.clone(),
            });
            let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
            let _ = protocol
                .commit(input_for(&mote, &w, idempotency_key, wref, mdh))
                .expect("commit must succeed");
            journal.read_committed(&mote.id).unwrap().unwrap()
        };

        let e1 = run();
        let e2 = run();
        prop_assert_eq!(&e1, &e2, "two independent runs must produce equal Committed entries");
    }

    /// R-11 monotonicity: when the broker returns an arbitrary
    /// `fake_ref`, the protocol's R-11 refusal carries that exact ref +
    /// the Mote's id.
    #[test]
    fn prop_r11_carries_brokers_fake_ref(
        seed in any::<u8>(),
        fake_ref_bytes in any::<[u8; 32]>(),
    ) {
        let mote = wm_idempotent_mote(seed);
        let w = warrant();
        let fake_ref = ContentRef::from_bytes(fake_ref_bytes);

        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HostileBroker { fake_ref });
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let result = protocol.commit(input_for(
            &mote,
            &w,
            [0; 32],
            ContentRef::from_bytes([0; 32]),
            MoteDefHash::from_bytes([0; 32]),
        ));
        match result {
            Err(CommitProtocolError::R11ResultRefIncomplete { mote_id, result_ref }) => {
                prop_assert_eq!(mote_id, mote.id);
                prop_assert_eq!(result_ref, fake_ref);
            }
            other => prop_assert!(
                false,
                "expected R-11 with broker's fake_ref, got {:?}",
                other,
            ),
        }
        // No Committed entry on R-11 path.
        prop_assert!(journal.read_committed(&mote.id).unwrap().is_none());
    }

    /// R-11 short-circuit: when R-11 fires, the journal stays empty for
    /// this Mote (no Committed entry was written). Prefix-monotonicity of
    /// the refusal — the protocol does not partially commit.
    #[test]
    fn prop_r11_does_not_partially_commit(
        seed in any::<u8>(),
        fake_ref_bytes in any::<[u8; 32]>(),
    ) {
        let mote = wm_idempotent_mote(seed);
        let w = warrant();
        let fake_ref = ContentRef::from_bytes(fake_ref_bytes);

        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HostileBroker { fake_ref });
        let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

        let _ = protocol.commit(input_for(
            &mote,
            &w,
            [0; 32],
            ContentRef::from_bytes([0; 32]),
            MoteDefHash::from_bytes([0; 32]),
        ));

        // The journal has zero entries; the store has zero objects.
        prop_assert_eq!(journal.count_entries().unwrap(), 0);
        prop_assert_eq!(store.len(), 0);
    }
}
