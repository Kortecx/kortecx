//! **PR 9b-9 — Test B**: executor commit-protocol trust under an
//! incomplete `ContentStore` impl.
//!
//! Per D39 §a/§c, the content store's contract is that `put` is atomic:
//! a returned `ref` MUST point at the full bytes. R-11 is the executor's
//! defense against a hostile / buggy `ContentStore` that violates this
//! contract — returning `Ok(ref)` from `put()` but failing to make the
//! bytes available via `get(ref)` later.
//!
//! Test B verifies the executor's commit-protocol catches this contract
//! violation via the `enforce_r11` check (`store.get(result_ref).is_err()
//! ` branch) and refuses `journal.append(Committed)` with
//! `CommitProtocolError::R11ResultRefIncomplete`.
//!
//! ## Test A reuse
//!
//! Test A (per D39 — content-store put atomicity) is already covered by
//! `crates/kx-content/src/local_fs.rs::tests::put_interrupted_between_sync_and_persist_leaves_no_observable_canonical_ref`
//! and exercises `LocalFsContentStore::put_with_interrupt_hook`. The
//! workspace test suite runs that test on every CI cycle; PR 9b-9 does
//! NOT duplicate the test here. Test A's regression-guard property is:
//! "if the put is interrupted, no observable canonical ref exists in
//! the store" — the SAME property R-11 relies on.
//!
//! Test A's content-side coverage + Test B's executor-side coverage
//! together close the put-atomicity / R-11 contract from both ends.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore, NotFound, StoreError};
use kx_executor::{
    run_wm_mote, CommitProtocolError, LifecycleError, LocalResourceManager, StandardCommitProtocol,
};
use kx_journal::{InMemoryJournal, Journal};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Hostile content stores: each variant violates a different piece of the
// `ContentStore` contract.
// ---------------------------------------------------------------------------

/// Mode 1: `put` succeeds, `contains` LIES (says yes), `get` returns
/// `NotFound`. Models "store accepted bytes then dropped them" — the
/// failure mode the R-11 get-check catches.
struct HostilePutThenLost {
    get_attempted: AtomicBool,
}

impl HostilePutThenLost {
    fn new() -> Self {
        Self {
            get_attempted: AtomicBool::new(false),
        }
    }
}

impl std::fmt::Debug for HostilePutThenLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostilePutThenLost").finish()
    }
}

impl ContentStore for HostilePutThenLost {
    type Payload = bytes::Bytes;

    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError> {
        // Compute the ref via the canonical content-addressing path; do
        // NOT actually store it.
        let r = ContentRef::of(bytes);
        Ok(r)
    }
    fn get(&self, _r: &ContentRef) -> Result<Self::Payload, NotFound> {
        self.get_attempted.store(true, Ordering::SeqCst);
        Err(NotFound)
    }
    fn delete(&self, _r: &ContentRef) -> Result<(), StoreError> {
        Ok(())
    }
    fn list_refs<'a>(&'a self) -> Box<dyn Iterator<Item = ContentRef> + 'a> {
        Box::new(std::iter::empty())
    }
    fn contains(&self, _r: &ContentRef) -> bool {
        // LIE: says the ref is present even though `get` will fail. R-11
        // must catch this via the get-check.
        true
    }
}

/// Mode 2: `put` succeeds + `contains` returns FALSE (honest about the
/// failure). R-11 catches via the contains-check (shorter path; never
/// reaches the get-check).
struct HostilePutThenAbsent;

impl HostilePutThenAbsent {
    fn new() -> Self {
        Self
    }
}

impl std::fmt::Debug for HostilePutThenAbsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostilePutThenAbsent").finish()
    }
}

impl ContentStore for HostilePutThenAbsent {
    type Payload = bytes::Bytes;

    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError> {
        Ok(ContentRef::of(bytes))
    }
    fn get(&self, _r: &ContentRef) -> Result<Self::Payload, NotFound> {
        Err(NotFound)
    }
    fn delete(&self, _r: &ContentRef) -> Result<(), StoreError> {
        Ok(())
    }
    fn list_refs<'a>(&'a self) -> Box<dyn Iterator<Item = ContentRef> + 'a> {
        Box::new(std::iter::empty())
    }
    fn contains(&self, _r: &ContentRef) -> bool {
        // HONEST: the ref is not actually present.
        false
    }
}

// ---------------------------------------------------------------------------
// Test broker — delegates puts to the store-under-test so the hostile
// store's put gets exercised, not a separate InMemoryContentStore.
// ---------------------------------------------------------------------------

struct BrokerOverStore<S: ContentStore + Send + Sync> {
    store: Arc<S>,
    response_bytes: Vec<u8>,
}

impl<S: ContentStore + Send + Sync> std::fmt::Debug for BrokerOverStore<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerOverStore").finish()
    }
}

impl<S: ContentStore + Send + Sync> CapabilityBroker for BrokerOverStore<S> {
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
            capability: ToolName("test-b".into()),
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

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

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

fn wm_mote(seed: u8) -> Mote {
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
// Test B — Mode 1: store lies (contains=true) + get fails. R-11 catches via
// the get-check.
// ============================================================================

#[test]
fn test_b_r11_catches_store_that_lies_about_contains_then_loses_bytes() {
    let store = Arc::new(HostilePutThenLost::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(BrokerOverStore {
        store: store.clone(),
        response_bytes: b"test-b-bytes".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let mote = wm_mote(0xB1);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();

    let err = run_wm_mote(
        &mote,
        &warrant(),
        ToolName("test-b".into()),
        empty_request(),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect_err("R-11 must fire");
    match err {
        LifecycleError::CommitProtocol(CommitProtocolError::R11ResultRefIncomplete {
            mote_id,
            ..
        }) => assert_eq!(mote_id, mote.id),
        other => panic!("expected R-11, got {other:?}"),
    }

    // Load-bearing: enforce_r11 actually called get (the contains-check
    // returned true so the get-check must have been reached).
    assert!(store.get_attempted.load(Ordering::SeqCst));
    // No Committed entry.
    assert!(journal.read_committed(&mote.id).unwrap().is_none());
}

// ============================================================================
// Test B — Mode 2: store is honest about absence (contains=false).
// R-11 catches via the contains-check (shorter path).
// ============================================================================

#[test]
fn test_b_r11_catches_store_that_honestly_reports_absence_after_put() {
    let store = Arc::new(HostilePutThenAbsent::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(BrokerOverStore {
        store: store.clone(),
        response_bytes: b"test-b-absent".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let mote = wm_mote(0xB2);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();

    let err = run_wm_mote(
        &mote,
        &warrant(),
        ToolName("test-b".into()),
        empty_request(),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    )
    .expect_err("R-11 must fire");
    assert!(matches!(
        err,
        LifecycleError::CommitProtocol(CommitProtocolError::R11ResultRefIncomplete { .. })
    ));
    assert!(journal.read_committed(&mote.id).unwrap().is_none());
}

// ============================================================================
// Sanity: with an honest InMemoryContentStore (the production path),
// the commit completes normally. This pins that Test B's hostile-store
// behavior is the specific failure mode caught — not a regression in
// the honest path.
// ============================================================================

#[test]
fn test_b_honest_in_memory_store_succeeds() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(BrokerOverStore {
        store: store.clone(),
        response_bytes: b"honest-path".to_vec(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();

    let mote = wm_mote(0xB3);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();

    let result = run_wm_mote(
        &mote,
        &warrant(),
        ToolName("test-b".into()),
        empty_request(),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    );
    let commit = result.expect("honest path must succeed");
    assert_eq!(commit.mote_id, mote.id);
    assert!(journal.read_committed(&mote.id).unwrap().is_some());
}

// ============================================================================
// Documentation pin: Test A is covered by kx-content's in-crate test.
// This compile-time check imports the public `StoreError` + `NotFound`
// types Test A indirectly depends on, ensuring those API surfaces remain
// stable. (The actual Test A test is in
// `crates/kx-content/src/local_fs.rs::tests`.)
// ============================================================================

#[test]
fn test_a_kx_content_api_surface_remains_stable() {
    fn _typecheck(_e: StoreError, _n: NotFound) {}
    // No runtime assertion — the compile-time import check is the test.
}
