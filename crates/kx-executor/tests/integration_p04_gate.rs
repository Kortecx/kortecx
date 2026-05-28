//! P0.4 hard gate (P3.3): the executor never re-runs a Mote that is already
//! `Committed` — it serves the committed `result_ref`. For a non-deterministic Mote
//! (ReadOnlyNondet / WorldMutating) this is a correctness invariant (re-running would
//! re-sample a different observation, or fire a second world effect); for PURE it is a
//! wasted-compute optimization. The proof is a **counting** executor / broker: after
//! two `run_*` calls on the same Mote, the body / broker was invoked exactly once — the
//! second call was served from the committed journal entry, not re-run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{
    run_pure_mote, run_wm_mote, LocalResourceManager, StandardCommitProtocol, TestMoteExecutor,
};
use kx_journal::InMemoryJournal;
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

fn warrant(class: MoteClass) -> WarrantSpec {
    WarrantSpec {
        mote_class: class,
        nd_class: class,
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

fn mote(seed: u8, nd: NdClass) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: nd,
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

/// A broker that counts `dispatch` invocations — the witness that a committed
/// non-deterministic Mote is NOT re-dispatched on a second run (no second world effect).
struct CountingBroker {
    store: Arc<InMemoryContentStore>,
    dispatches: Arc<AtomicU32>,
}
impl std::fmt::Debug for CountingBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingBroker").finish()
    }
}
impl CapabilityBroker for CountingBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        // A fresh ref per dispatch — so a re-dispatch would produce a *different* ref,
        // making "served the committed ref" distinguishable from "re-ran".
        let n = self.dispatches.fetch_add(1, Ordering::Relaxed);
        let staged_ref = self.store.put(&[n as u8 + 1; 8]).expect("put");
        Ok(BrokerHandle {
            staged_ref,
            capability: ToolName("counting".into()),
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

#[test]
fn committed_pure_mote_is_served_not_rerun() {
    let journal = InMemoryJournal::new();
    let rm = LocalResourceManager::dev_defaults();
    let runs = Arc::new(AtomicU32::new(0));
    let r = runs.clone();
    // Each run returns a DIFFERENT ref, so re-running would be observable.
    let executor = TestMoteExecutor::new(move |_m, _w| {
        let n = r.fetch_add(1, Ordering::Relaxed);
        ContentRef::from_bytes([n as u8 + 1; 32])
    });
    let w = warrant(MoteClass::Pure);
    let m = mote(0x11, NdClass::Pure);

    let first = run_pure_mote(&m, &w, &journal, &rm, &executor).unwrap();
    let second = run_pure_mote(&m, &w, &journal, &rm, &executor).unwrap();

    assert_eq!(
        runs.load(Ordering::Relaxed),
        1,
        "P0.4 gate: body run exactly once; the second call served the committed result"
    );
    assert_eq!(
        first.result_ref, second.result_ref,
        "served the same committed result_ref"
    );
    assert_eq!(first.committed_seq, second.committed_seq);
}

#[test]
fn committed_world_mutating_mote_is_not_re_dispatched() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let dispatches = Arc::new(AtomicU32::new(0));
    let broker = Arc::new(CountingBroker {
        store: store.clone(),
        dispatches: dispatches.clone(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let w = warrant(MoteClass::WorldMutating);
    let m = mote(0x22, NdClass::WorldMutating);
    let submission: BTreeMap<MoteId, Mote> = std::iter::once((m.id, m.clone())).collect();

    let first = run_wm_mote(
        &m,
        &w,
        ToolName("c".into()),
        empty_request(),
        &submission,
        &*journal,
        &rm,
        &protocol,
    )
    .unwrap();
    let second = run_wm_mote(
        &m,
        &w,
        ToolName("c".into()),
        empty_request(),
        &submission,
        &*journal,
        &rm,
        &protocol,
    )
    .unwrap();

    assert_eq!(
        dispatches.load(Ordering::Relaxed),
        1,
        "P0.4 gate: a committed WORLD-MUTATING Mote is served, NEVER re-dispatched (no second world effect)"
    );
    assert_eq!(
        first.result_ref, second.result_ref,
        "served the committed effect result"
    );
    assert_eq!(first.committed_seq, second.committed_seq);
}

#[test]
fn committed_read_only_nondet_observation_is_not_resampled() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let dispatches = Arc::new(AtomicU32::new(0));
    let broker = Arc::new(CountingBroker {
        store: store.clone(),
        dispatches: dispatches.clone(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let w = warrant(MoteClass::ReadOnlyNondet);
    let m = mote(0x33, NdClass::ReadOnlyNondet);
    let submission: BTreeMap<MoteId, Mote> = std::iter::once((m.id, m.clone())).collect();

    let first = run_wm_mote(
        &m,
        &w,
        ToolName("c".into()),
        empty_request(),
        &submission,
        &*journal,
        &rm,
        &protocol,
    )
    .unwrap();
    let second = run_wm_mote(
        &m,
        &w,
        ToolName("c".into()),
        empty_request(),
        &submission,
        &*journal,
        &rm,
        &protocol,
    )
    .unwrap();

    assert_eq!(
        dispatches.load(Ordering::Relaxed),
        1,
        "P0.4 gate: a committed READ-ONLY-NONDET observation is served, NEVER re-sampled"
    );
    assert_eq!(
        first.result_ref, second.result_ref,
        "the durable observation is stable across recovery (not a fresh sample)"
    );
}
