//! M2.3b (D65 / D105.4) — the class-aware `AtLeastOnce` recovery arm of
//! `redispatch_wm_mote`. A staged-uncommitted at-most-once effect has no closing
//! mechanism, so recovery must NEVER blind-redispatch (that would double-fire).
//! Instead it either:
//!   - **Compensates** (the capability supports an undo) → terminal
//!     `Failed { CompensatedAtLeastOnce }`, NO broker dispatch, NO Committed; or
//!   - **Quarantines** (no undo support) → terminal `Failed { QuarantinedAtLeastOnce }`,
//!     surfaced via `anomaly_motes()`; or
//!   - **Refuses** (the undo itself errors) → `CompensateFailed`, fail-closed.
//!
//! The load-bearing invariant: the world effect is NEVER re-applied, and a second
//! recovery pass is a no-op (the terminal `Failed` makes the oracle refuse).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{
    redispatch_wm_mote, CommitProtocolError, LifecycleError, LocalResourceManager, RecoveryAction,
    StandardCommitProtocol, WmRecoveryOutcome, WmRedispatchOracle,
};
use kx_journal::{FailureReason, InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::{AnomalyKind, MoteState, Projection};
use kx_tool_registry::IdempotencyClass;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

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

/// Oracle stub. `can_redispatch_world_effect` is consulted at Step 0; the
/// AtLeastOnce arm runs only when it returns `true` (a real staged-uncommitted
/// Mote), so we set `true` here and additionally verify the SECOND pass (after a
/// terminal Failed lands) refuses via the real projection oracle.
struct StubOracle {
    can_redispatch: bool,
}
impl WmRedispatchOracle for StubOracle {
    fn can_redispatch_world_effect(&self, _mote_id: &MoteId) -> bool {
        self.can_redispatch
    }
}

/// How the broker's `compensate` behaves.
#[derive(Clone, Copy)]
enum CompensateMode {
    /// The capability supports an undo → broker stages the undo's result.
    Supported,
    /// The capability does NOT support compensation (default `Ok(None)`).
    Unsupported,
    /// The undo itself errors.
    Errors,
}

/// A broker that counts world effects. `dispatch` is the load-bearing
/// double-fire witness: it MUST stay at 0 on the AtLeastOnce recovery arm.
struct CompensatingBroker {
    store: Arc<InMemoryContentStore>,
    mode: CompensateMode,
    dispatched: AtomicUsize,
    compensated: AtomicUsize,
}
impl std::fmt::Debug for CompensatingBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompensatingBroker").finish()
    }
}
impl CapabilityBroker for CompensatingBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        // A blind re-dispatch of an AtLeastOnce effect — the bug M2.3b closes.
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        let r = self.store.put(b"REDISPATCH-double-fire").expect("put");
        Ok(BrokerHandle {
            staged_ref: r,
            capability: ToolName("wm".into()),
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
    fn compensate(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        match self.mode {
            CompensateMode::Supported => {
                self.compensated.fetch_add(1, Ordering::SeqCst);
                let r = self.store.put(b"UNDO-result").expect("put");
                Ok(Some(BrokerHandle {
                    staged_ref: r,
                    capability: ToolName("wm".into()),
                    capability_version: ToolVersion("0.1.0".into()),
                }))
            }
            CompensateMode::Unsupported => Ok(None),
            CompensateMode::Errors => Err(BrokerError::CapabilityFailure {
                capability: ToolName("wm".into()),
                reason: kx_capability::CapabilityFailureReason::Other("undo blew up".into()),
            }),
        }
    }
}

/// Drive the AtLeastOnce recovery arm once. Returns the outcome + the broker so
/// callers can assert the dispatch/compensate counters + the journal.
fn drive_at_least_once(
    mode: CompensateMode,
) -> (
    Result<WmRecoveryOutcome, LifecycleError>,
    Arc<InMemoryJournal>,
    Arc<CompensatingBroker>,
) {
    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let broker = Arc::new(CompensatingBroker {
        store: store.clone(),
        mode,
        dispatched: AtomicUsize::new(0),
        compensated: AtomicUsize::new(0),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker.clone());
    let rm = LocalResourceManager::dev_defaults();
    let oracle = StubOracle {
        can_redispatch: true,
    };
    let mote = wm_mote(0x4a);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();
    let out = redispatch_wm_mote(
        &mote,
        &warrant(),
        ToolName("wm".into()),
        empty_request(),
        Some(IdempotencyClass::AtLeastOnce),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &oracle,
    );
    (out, journal, broker)
}

fn only_entry(journal: &InMemoryJournal) -> JournalEntry {
    let entries: Vec<JournalEntry> = journal.read_entries_by_seq(0..u64::MAX).unwrap().collect();
    assert_eq!(entries.len(), 1, "expected exactly one journal entry");
    entries.into_iter().next().unwrap()
}

// ---------------------------------------------------------------------------
// Compensate arm
// ---------------------------------------------------------------------------

#[test]
fn at_least_once_with_compensation_undoes_and_terminally_fails() {
    let (out, journal, broker) = drive_at_least_once(CompensateMode::Supported);
    let outcome = out.expect("compensation must succeed");

    match outcome {
        WmRecoveryOutcome::TerminallyFailed {
            action, mote_id, ..
        } => {
            assert_eq!(action, RecoveryAction::Compensate);
            assert_eq!(mote_id, wm_mote(0x4a).id);
        }
        other => panic!("expected TerminallyFailed{{Compensate}}, got {other:?}"),
    }

    // The effect was NEVER re-dispatched (no double-fire); the undo ran once.
    assert_eq!(broker.dispatched.load(Ordering::SeqCst), 0);
    assert_eq!(broker.compensated.load(Ordering::SeqCst), 1);

    // The journal carries a terminal Failed{Compensated} — no Committed.
    match only_entry(&journal) {
        JournalEntry::Failed { reason_class, .. } => {
            assert_eq!(reason_class, FailureReason::CompensatedAtLeastOnce);
        }
        other => panic!("expected Failed{{Compensated}}, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Quarantine arm
// ---------------------------------------------------------------------------

#[test]
fn at_least_once_without_compensation_quarantines() {
    let (out, journal, broker) = drive_at_least_once(CompensateMode::Unsupported);
    let outcome = out.expect("quarantine is a successful (non-error) recovery outcome");

    match outcome {
        WmRecoveryOutcome::TerminallyFailed { action, .. } => {
            assert_eq!(action, RecoveryAction::Quarantine);
        }
        other => panic!("expected TerminallyFailed{{Quarantine}}, got {other:?}"),
    }

    // Never re-dispatched, never compensated.
    assert_eq!(broker.dispatched.load(Ordering::SeqCst), 0);
    assert_eq!(broker.compensated.load(Ordering::SeqCst), 0);

    // Terminal Failed{Quarantined}, and the fold surfaces it as a quarantine anomaly.
    match only_entry(&journal) {
        JournalEntry::Failed { reason_class, .. } => {
            assert_eq!(reason_class, FailureReason::QuarantinedAtLeastOnce);
        }
        other => panic!("expected Failed{{Quarantined}}, got {other:?}"),
    }
    let p = Projection::from_journal(&*journal).unwrap();
    let mid = wm_mote(0x4a).id;
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    assert!(!p.can_redispatch_world_effect(&mid));
    assert!(p
        .anomaly_motes()
        .contains(&(mid, AnomalyKind::QuarantinedAtLeastOnceEffect)));
}

// ---------------------------------------------------------------------------
// CompensateFailed — fail-closed
// ---------------------------------------------------------------------------

#[test]
fn at_least_once_compensation_error_refuses_fail_closed() {
    let (out, journal, broker) = drive_at_least_once(CompensateMode::Errors);
    let err = out.expect_err("a failing undo must refuse, not commit");

    match err {
        LifecycleError::CommitProtocol(e @ CommitProtocolError::CompensateFailed { .. }) => {
            assert!(
                e.is_recovery_refusal(),
                "CompensateFailed is a recovery refusal"
            );
        }
        other => panic!("expected CompensateFailed, got {other:?}"),
    }

    // Fail-closed: no re-dispatch, and NO journal mutation (no Committed, no Failed).
    assert_eq!(broker.dispatched.load(Ordering::SeqCst), 0);
    assert_eq!(journal.count_entries().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Prefix-monotonic terminality: a SECOND recovery pass is refused.
// ---------------------------------------------------------------------------

#[test]
fn second_recovery_pass_after_quarantine_is_refused_by_the_oracle() {
    // Pass 1: quarantine an AtLeastOnce Mote (terminal Failed{Quarantined}).
    let (out, journal, _broker) = drive_at_least_once(CompensateMode::Unsupported);
    out.expect("pass 1 quarantine");

    // Pass 2: fold the journal into a REAL projection oracle (which now sees the
    // terminal Failed) and re-run recovery. The oracle must refuse at Step 0 —
    // the quarantine is durable + prefix-monotonic, so no double-fire ever.
    let store = Arc::new(InMemoryContentStore::new());
    let broker = Arc::new(CompensatingBroker {
        store: store.clone(),
        mode: CompensateMode::Supported,
        dispatched: AtomicUsize::new(0),
        compensated: AtomicUsize::new(0),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker.clone());
    let rm = LocalResourceManager::dev_defaults();
    let projection = Projection::from_journal(&*journal).unwrap();
    let mote = wm_mote(0x4a);
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();

    let result = redispatch_wm_mote(
        &mote,
        &warrant(),
        ToolName("wm".into()),
        empty_request(),
        Some(IdempotencyClass::AtLeastOnce),
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &projection,
    );
    assert!(matches!(
        result,
        Err(LifecycleError::CommitProtocol(
            CommitProtocolError::R13WmReDispatchRefused { .. }
        ))
    ));
    // No undo, no dispatch on the refused second pass.
    assert_eq!(broker.dispatched.load(Ordering::SeqCst), 0);
    assert_eq!(broker.compensated.load(Ordering::SeqCst), 0);
}
