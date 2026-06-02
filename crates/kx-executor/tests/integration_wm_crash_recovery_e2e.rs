//! **PR 9b-10 — WORLD-MUTATING Mote crash-recovery end-to-end.** The
//! runtime-promise demo: a WM Mote with `EffectPattern::StageThenCommit`
//! survives a mid-stage crash via the EffectStaged hint + R-13 recovery
//! + broker idempotency.
//!
//! ## The runtime promise
//!
//! Given a WORLD-MUTATING Mote dispatched under `StageThenCommit`:
//!
//! 1. **Pre-crash**: `run_wm_mote` writes `Proposed` + `EffectStaged`
//!    BEFORE `broker.dispatch`. If the broker fails (network drop,
//!    sandbox refused, etc.), the journal carries Proposed +
//!    EffectStaged but NO Committed. The WM effect MAY have happened
//!    at the remote tool's end — the broker doesn't know.
//!
//! 2. **Restart**: the executor folds the journal via
//!    `Projection::from_journal` and discovers the Mote is in cell 2
//!    of the recovery cross-product (EffectStaged alone). The oracle
//!    `can_redispatch_world_effect` returns `true` — re-dispatch is
//!    permitted.
//!
//! 3. **Recovery dispatch**: `redispatch_wm_mote` invokes
//!    `broker.dispatch` AGAIN. The Mote's identity is deterministic
//!    (per D38 §1: `idempotency_key = mote.id.to_hex()`); the remote
//!    tool dedupes on this key. The journal appends `Committed`.
//!
//! 4. **Final state**: journal has `Proposed → EffectStaged →
//!    Committed`. Broker was invoked TWICE (once pre-crash, once on
//!    recovery), but the deterministic `idempotency_key` ensures the
//!    remote tool produced the same effect (single observable
//!    side-effect). Recovery is SAFE.
//!
//! This test stitches together every PR 9b slice:
//! - 9b-1 R-10 submission refusals (not exercised here; PURE-path)
//! - 9b-2/3/4/5 commit_protocol + per-pattern paths
//! - 9b-6 lifecycle integration (`run_wm_mote`)
//! - 9b-7 R-13 recovery wiring (`redispatch_wm_mote` + Oracle)
//! - 9b-8 9-cell cross-product (cell 2 in particular)
//! - 9b-9 R-11 content-store trust
//!
//! Closes PR 9 / P1.9 entirely.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{
    redispatch_wm_mote, run_wm_mote, CommitProtocolError, LifecycleError, LocalResourceManager,
    StandardCommitProtocol, WmLifecycleCommit, WmRecoveryOutcome,
};

/// Unwrap a `Committed` recovery outcome (this e2e exercises the Staged-class
/// probe-then-redispatch path; class `None` → no compensate/quarantine).
fn into_commit(out: WmRecoveryOutcome) -> WmLifecycleCommit {
    match out {
        WmRecoveryOutcome::Committed { commit, .. } => commit,
        other => panic!("expected a Committed recovery outcome, got {other:?}"),
    }
}
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::Projection;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// CrashThenRecoverBroker: fails on the FIRST `dispatch` call (simulating a
// crash / network drop / sandbox refusal between EffectStaged write and
// Committed write), succeeds on the SECOND call. Tracks the count so the
// test can assert "exactly 2 dispatches with deterministic idempotency".
//
// Models the IdempotencyClass::Staged tool semantics (per D38 §2b):
// - The first dispatch may or may not have applied the world effect.
// - The second dispatch carries the SAME idempotency_key (deterministic
//   from mote.id per D38 §1); the remote tool dedupes.
// ---------------------------------------------------------------------------

struct CrashThenRecoverBroker {
    store: Arc<InMemoryContentStore>,
    response_bytes: Vec<u8>,
    dispatch_count: AtomicUsize,
}

impl std::fmt::Debug for CrashThenRecoverBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrashThenRecoverBroker").finish()
    }
}

impl CapabilityBroker for CrashThenRecoverBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        let n = self.dispatch_count.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            // Pre-crash: simulate broker failure after EffectStaged
            // was journaled. The WM effect MAY have happened remotely;
            // the broker can't tell.
            Err(BrokerError::SandboxRefused {
                capability: ToolName("staged-tool".into()),
                reason: "simulated mid-stage crash (network drop / sandbox refused)".into(),
            })
        } else {
            // Recovery: same idempotency_key (deterministic from
            // mote.id per D38 §1) → remote tool dedupes → returns same
            // staged_ref. Test the store dedup via put(same bytes).
            let r = self.store.put(&self.response_bytes).expect("put");
            Ok(BrokerHandle {
                staged_ref: r,
                capability: ToolName("staged-tool".into()),
                capability_version: ToolVersion("0.1.0".into()),
            })
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

fn wm_staged_mote(seed: u8) -> Mote {
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
    }
}

// ============================================================================
// THE RUNTIME-PROMISE DEMO
// ============================================================================

#[test]
fn wm_mote_crash_recovery_end_to_end_runtime_promise() {
    // Single shared state across pre-crash + recovery phases.
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(CrashThenRecoverBroker {
        store: store.clone(),
        response_bytes: b"runtime-promise-response".to_vec(),
        dispatch_count: AtomicUsize::new(0),
    });
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker.clone());
    let rm = LocalResourceManager::dev_defaults();

    let producer = wm_staged_mote(0xCC);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((producer.id, producer.clone())).collect();
    let w = warrant();

    // ---------------------------------------------------------------------
    // Phase 1 — pre-crash fresh dispatch. The broker fails on its first
    // call; the EffectStaged hint is in the journal AFTER this phase but
    // no Committed entry lands. The simulated crash here is the broker's
    // failure mid-stage; in production this could be network drop, sandbox
    // refusal, or a process kill after EffectStaged but before Committed.
    // ---------------------------------------------------------------------
    let phase1 = run_wm_mote(
        &producer,
        &w,
        ToolName("staged-tool".into()),
        empty_request(),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
    );
    assert!(
        matches!(
            phase1,
            Err(LifecycleError::CommitProtocol(
                CommitProtocolError::BrokerDispatchFailed { .. }
            ))
        ),
        "phase 1: simulated mid-stage crash (broker fails AFTER EffectStaged)"
    );
    assert_eq!(
        broker.dispatch_count.load(Ordering::SeqCst),
        1,
        "phase 1: broker called exactly once"
    );

    // Verify journal mid-crash state: Proposed + EffectStaged, NO Committed.
    let entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();
    assert_eq!(entries.len(), 2, "mid-crash: Proposed + EffectStaged only");
    assert!(
        matches!(&entries[0], JournalEntry::Proposed { mote_id, .. } if *mote_id == producer.id)
    );
    assert!(
        matches!(&entries[1], JournalEntry::EffectStaged { mote_id, .. } if *mote_id == producer.id)
    );
    assert!(journal.read_committed(&producer.id).unwrap().is_none());

    // ---------------------------------------------------------------------
    // Phase 2 — restart with recovery. Fold the journal into a Projection;
    // the projection's `can_redispatch_world_effect` returns true for cell
    // 2 (EffectStaged alone). Call `redispatch_wm_mote` to complete the
    // commit.
    // ---------------------------------------------------------------------
    let projection = Arc::new(Projection::from_journal(&*journal).expect("from_journal"));
    let phase2 = into_commit(
        redispatch_wm_mote(
            &producer,
            &w,
            ToolName("staged-tool".into()),
            empty_request(),
            None,
            &submission_motes,
            &*journal,
            &rm,
            &protocol,
            &*projection,
        )
        .expect("phase 2: recovery dispatch must succeed"),
    );

    assert_eq!(phase2.mote_id, producer.id);
    assert_eq!(
        broker.dispatch_count.load(Ordering::SeqCst),
        2,
        "phase 2: broker called exactly once more (idempotency_key dedup at remote tool boundary)"
    );

    // ---------------------------------------------------------------------
    // Final assertions — the runtime promise.
    // ---------------------------------------------------------------------
    let final_entries: Vec<JournalEntry> = journal
        .read_entries_by_seq(0..u64::MAX)
        .expect("scan")
        .collect();

    // The journal carries Proposed → EffectStaged → Committed. The
    // recovery path did NOT append a second Proposed (per the
    // redispatch_wm_mote contract; the previous Proposed is already in
    // the journal). The v2 dedup-index {1, 2, 4} would have made a
    // duplicate Committed a dedup hit, but in our cell-2 recovery the
    // first attempt never wrote Committed at all.
    assert_eq!(
        final_entries.len(),
        3,
        "final: Proposed → EffectStaged → Committed (no fresh Proposed on recovery)"
    );
    assert!(matches!(&final_entries[0], JournalEntry::Proposed { .. }));
    assert!(matches!(
        &final_entries[1],
        JournalEntry::EffectStaged { .. }
    ));
    let committed = match &final_entries[2] {
        JournalEntry::Committed {
            mote_id,
            result_ref,
            ..
        } => {
            assert_eq!(*mote_id, producer.id);
            *result_ref
        }
        other => panic!("final entry must be Committed; got {other:?}"),
    };

    // The committed result_ref resolves to the broker's response bytes.
    let bytes = store.get(&committed).expect("store get");
    assert_eq!(&*bytes, b"runtime-promise-response");

    // ---------------------------------------------------------------------
    // Sanity: re-running recovery a SECOND time after the Committed
    // landed must be refused — R-13 fires because the projection now
    // sees the Committed entry (cell 5 of the cross-product: done; never
    // re-dispatch).
    // ---------------------------------------------------------------------
    let projection_after = Arc::new(Projection::from_journal(&*journal).expect("from_journal"));
    let phase3 = redispatch_wm_mote(
        &producer,
        &w,
        ToolName("staged-tool".into()),
        empty_request(),
        None,
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &*projection_after,
    );
    assert!(
        matches!(
            phase3,
            Err(LifecycleError::CommitProtocol(
                CommitProtocolError::R13WmReDispatchRefused { .. }
            )),
        ),
        "phase 3 sanity: post-Committed re-dispatch refused (cell 5 of 9-cell cross-product)"
    );

    // The post-sanity dispatch count is still 2 (R-13 fired before any
    // broker call).
    assert_eq!(
        broker.dispatch_count.load(Ordering::SeqCst),
        2,
        "post-sanity: R-13 short-circuits before broker is consulted"
    );
}

// ============================================================================
// Bonus: verify that recovery from cell-4 state (EffectStaged + terminal
// Failed) is REFUSED — the WM double-effect hazard guard. This is the
// dual of the happy-path runtime promise: the protocol must REFUSE to
// re-dispatch when a terminal failure was observed.
// ============================================================================

#[test]
fn wm_mote_terminal_failure_recovery_refused_cell_4_hazard_guard() {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(CrashThenRecoverBroker {
        store: store.clone(),
        response_bytes: b"never-dispatched".to_vec(),
        dispatch_count: AtomicUsize::new(0),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker.clone());
    let rm = LocalResourceManager::dev_defaults();

    let producer = wm_staged_mote(0xDD);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((producer.id, producer.clone())).collect();

    // Construct journal prefix: Proposed + EffectStaged + Failed(terminal)
    // (cell 4 of the 9-cell cross-product).
    let warrant_ref = kx_warrant::warrant_ref_of(&warrant());
    let proposed = JournalEntry::Proposed {
        mote_id: producer.id,
        idempotency_key: *producer.id.as_bytes(),
        seq: 0,
        nondeterminism: NdClass::WorldMutating,
        placement_hint: 0,
        warrant_ref,
    };
    journal.append(proposed).unwrap();
    journal
        .append(JournalEntry::EffectStaged {
            mote_id: producer.id,
            idempotency_key: *producer.id.as_bytes(),
            seq: 0,
        })
        .unwrap();
    journal
        .append(JournalEntry::Failed {
            mote_id: producer.id,
            idempotency_key: *producer.id.as_bytes(),
            seq: 0,
            reason_class: kx_journal::FailureReason::ExecutorRefused, // terminal
            reporter_id: 0,
        })
        .unwrap();

    // Restart with recovery — must REFUSE re-dispatch.
    let projection = Arc::new(Projection::from_journal(&*journal).expect("from_journal"));
    let result = redispatch_wm_mote(
        &producer,
        &warrant(),
        ToolName("staged-tool".into()),
        empty_request(),
        None,
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &*projection,
    );
    assert!(matches!(
        result,
        Err(LifecycleError::CommitProtocol(
            CommitProtocolError::R13WmReDispatchRefused { .. }
        ))
    ));
    // Broker was NEVER called — R-13 short-circuits the WM double-effect
    // hazard.
    assert_eq!(
        broker.dispatch_count.load(Ordering::SeqCst),
        0,
        "cell 4: R-13 must short-circuit BEFORE broker.dispatch"
    );
}
