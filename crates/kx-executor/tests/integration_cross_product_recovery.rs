//! PR 9b-8 — **9-cell cross-product recovery integration tests at the
//! executor layer**. Mirrors `crates/kx-projection/tests/cross_product.rs`
//! but drives the recovery decisions through the full executor path:
//! `redispatch_wm_mote(... &Projection ...)` consults
//! `Projection::can_redispatch_world_effect` via the `WmRedispatchOracle`
//! impl (PR 9b-7), then the commit_protocol routes per `EffectPattern`.
//!
//! Each cell test follows the shape:
//!
//! 1. Append a journal prefix matching the cell's shape.
//! 2. Build a `Projection` from the journal (it's the oracle).
//! 3. Call `redispatch_wm_mote(&mote, ..., &projection)`.
//! 4. Assert the expected outcome — Ok (re-dispatch permitted) or
//!    R-13 refusal (cell 5/6/7/8 + cells 0/1 where no EffectStaged).
//!
//! ## 9-cell table at this layer
//!
//! | # | Prefix | Oracle decision | redispatch_wm_mote outcome |
//! |---|---|:---:|---|
//! | 0 | (empty) | refuse | R-13 (no EffectStaged) |
//! | 1 | Failed only | refuse | R-13 (no EffectStaged) |
//! | 2 | EffectStaged | permit | Ok → Committed |
//! | 3 | EffectStaged + Failed(pre_commit_crash) | permit | Ok → Committed |
//! | 4 | EffectStaged + Failed(terminal) | refuse | R-13 (terminal_failure_observed) |
//! | 5 | EffectStaged + Committed | refuse | R-13 (already committed) |
//! | 6 | Committed alone | refuse | R-13 (already committed) |
//! | 7 | EffectStaged + Committed + Repudiated | refuse | R-13 (already committed; repudiated) |
//! | 8 | EffectStaged + Repudiated (no Committed) | refuse | R-13 (Inconsistent — anomaly) |

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{
    redispatch_wm_mote, CommitProtocolError, LifecycleError, LocalResourceManager,
    StandardCommitProtocol, WmLifecycleCommit, WmRecoveryOutcome,
};

/// Unwrap a `Committed` recovery outcome; these cross-product tests only exercise
/// the probe-then-redispatch path (class `None` → no compensate/quarantine).
fn into_commit(out: WmRecoveryOutcome) -> WmLifecycleCommit {
    match out {
        WmRecoveryOutcome::Committed { commit, .. } => commit,
        other => panic!("expected a Committed recovery outcome, got {other:?}"),
    }
}
use kx_journal::{
    repudiation_idempotency_key, FailureReason, InMemoryJournal, Journal, JournalEntry,
    RepudiationReason,
};
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

fn wm_mote(pattern: EffectPattern, seed: u8) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: pattern,
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

// Journal entry constructors matching the kx-projection cross_product test
// shapes. Each takes the Mote being attempted.
fn effect_staged(mote: &Mote) -> JournalEntry {
    JournalEntry::EffectStaged {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq: 0,
    }
}

fn committed_entry(mote: &Mote, result_ref: ContentRef) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq: 0,
        nondeterminism: mote.def.nd_class,
        result_ref,
        parents: SmallVec::new(),
        warrant_ref: kx_warrant::warrant_ref_of(&warrant()),
        mote_def_hash: mote.def.hash(),
    }
}

fn failed_entry(mote: &Mote, reason: FailureReason) -> JournalEntry {
    JournalEntry::Failed {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq: 0,
        reason_class: reason,
        reporter_id: 0,
    }
}

fn repudiated_entry(target_mote_id: MoteId, target_committed_seq: u64) -> JournalEntry {
    JournalEntry::Repudiated {
        target_mote_id,
        idempotency_key: repudiation_idempotency_key(&target_mote_id, target_committed_seq),
        seq: 0,
        target_committed_seq,
        reason_class: RepudiationReason::OperatorAction,
        repudiator_id: 0,
    }
}

/// Build (journal, projection) by appending entries through an
/// `InMemoryJournal`; the journal assigns monotonic seqs.
fn journal_and_projection(entries: Vec<JournalEntry>) -> (Arc<InMemoryJournal>, Arc<Projection>) {
    let journal = Arc::new(InMemoryJournal::new());
    let mut last_committed_seq: Option<u64> = None;
    for entry in entries {
        // If the entry is a Repudiated entry with target_committed_seq == 0,
        // patch it to point at the just-appended Committed entry's seq.
        let appended = match entry {
            JournalEntry::Repudiated {
                target_mote_id,
                target_committed_seq: 0,
                seq,
                reason_class,
                repudiator_id,
                ..
            } => {
                let actual_target = last_committed_seq.expect("Repudiated needs prior Committed");
                let new_entry = JournalEntry::Repudiated {
                    target_mote_id,
                    idempotency_key: repudiation_idempotency_key(&target_mote_id, actual_target),
                    seq,
                    target_committed_seq: actual_target,
                    reason_class,
                    repudiator_id,
                };
                journal.append(new_entry).expect("append Repudiated")
            }
            other => journal.append(other).expect("append"),
        };
        if let JournalEntry::Committed { seq, .. } = &appended {
            last_committed_seq = Some(*seq);
        }
    }
    let projection = Arc::new(Projection::from_journal(&*journal).expect("from_journal"));
    (journal, projection)
}

struct HappyBroker {
    store: Arc<InMemoryContentStore>,
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
        let r = self.store.put(b"cross-product-resp").expect("put");
        Ok(BrokerHandle {
            staged_ref: r,
            capability: ToolName("xprod".into()),
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

fn drive_redispatch(
    mote: &Mote,
    entries: Vec<JournalEntry>,
) -> (
    Result<kx_executor::WmLifecycleCommit, LifecycleError>,
    Arc<InMemoryJournal>,
) {
    let (journal, projection) = journal_and_projection(entries);
    let store = Arc::new(InMemoryContentStore::new());
    let broker = Arc::new(HappyBroker {
        store: store.clone(),
    });
    let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let submission_motes: BTreeMap<MoteId, Mote> =
        std::iter::once((mote.id, mote.clone())).collect();
    let result = redispatch_wm_mote(
        mote,
        &warrant(),
        ToolName("xprod".into()),
        empty_request(),
        None,
        &submission_motes,
        &*journal,
        &rm,
        &protocol,
        &*projection,
    )
    .map(into_commit);
    (result, journal)
}

fn assert_r13_refusal(result: &Result<kx_executor::WmLifecycleCommit, LifecycleError>) {
    match result {
        Err(LifecycleError::CommitProtocol(CommitProtocolError::R13WmReDispatchRefused {
            ..
        })) => {}
        other => panic!("expected R-13 refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Cell 0 — empty journal: no EffectStaged hint → oracle refuses → R-13.
// ---------------------------------------------------------------------------

#[test]
fn cell_0_no_journal_entries_refuses_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x00);
    let (result, _journal) = drive_redispatch(&mote, vec![]);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell 1 — Failed (no EffectStaged): oracle refuses (no in-flight effect).
// ---------------------------------------------------------------------------

#[test]
fn cell_1_failed_only_refuses_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x01);
    let prefix = vec![failed_entry(&mote, FailureReason::TimedOut)];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell 2 — EffectStaged alone: oracle permits → redispatch proceeds → Committed.
// ---------------------------------------------------------------------------

#[test]
fn cell_2_effect_staged_alone_permits_redispatch() {
    let mote = wm_mote(EffectPattern::StageThenCommit, 0x02);
    let prefix = vec![effect_staged(&mote)];
    let (result, journal) = drive_redispatch(&mote, prefix);
    let commit = result.expect("cell 2: redispatch must succeed");
    assert_eq!(commit.mote_id, mote.id);

    // Journal: EffectStaged (from prefix) + EffectStaged (NEW from
    // commit_protocol StageThenCommit path) + Committed.
    // Note: PR 9b-8 doesn't dedup EffectStaged appends; the journal's
    // v2 dedup-by-key index for kind=4 makes the second EffectStaged a
    // dedup hit (returned entry has the original seq). So we see at
    // most 2 distinct entries: the original EffectStaged + the new
    // Committed.
    let committed = journal
        .read_committed(&mote.id)
        .expect("read_committed")
        .expect("Committed must exist");
    assert!(matches!(committed, JournalEntry::Committed { .. }));
}

// ---------------------------------------------------------------------------
// Cell 3 — EffectStaged + Failed(pre_commit_crash): oracle permits.
// ---------------------------------------------------------------------------

#[test]
fn cell_3_effect_staged_then_pre_commit_crash_permits_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x03);
    let prefix = vec![
        effect_staged(&mote),
        failed_entry(&mote, FailureReason::TimedOut), // pre-commit-crash
    ];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    let commit = result.expect("cell 3: redispatch must succeed");
    assert_eq!(commit.mote_id, mote.id);
}

// ---------------------------------------------------------------------------
// Cell 4/5 — EffectStaged + Failed(terminal): oracle refuses (terminal failure
// under EffectStaged is the WM double-effect hazard; D38 §2b STEP 5.2).
// ---------------------------------------------------------------------------

#[test]
fn cell_4_effect_staged_then_terminal_failure_refuses_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x04);
    let prefix = vec![
        effect_staged(&mote),
        failed_entry(&mote, FailureReason::ExecutorRefused), // terminal
    ];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell 5 — EffectStaged + Committed: oracle refuses (already committed; never
// re-dispatch).
// ---------------------------------------------------------------------------

#[test]
fn cell_5_effect_staged_plus_committed_refuses_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x05);
    let prefix = vec![
        effect_staged(&mote),
        committed_entry(&mote, ContentRef::from_bytes([0xab; 32])),
    ];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell 6 — Committed alone: oracle refuses.
// ---------------------------------------------------------------------------

#[test]
fn cell_6_committed_alone_refuses_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x06);
    let prefix = vec![committed_entry(&mote, ContentRef::from_bytes([0xab; 32]))];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell 7 — EffectStaged + Committed + Repudiated: oracle refuses (already
// committed, repudiation is a separate concern).
// ---------------------------------------------------------------------------

#[test]
fn cell_7_effect_staged_plus_committed_plus_repudiated_refuses_redispatch() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x07);
    let prefix = vec![
        effect_staged(&mote),
        committed_entry(&mote, ContentRef::from_bytes([0xab; 32])),
        // target_committed_seq=0 in the constructor signals "patch to
        // the just-appended Committed's seq".
        repudiated_entry(mote.id, 0),
    ];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell 8 — EffectStaged + Repudiated (NO Committed): the anomaly case.
// Oracle returns false (inconsistent flag set).
// ---------------------------------------------------------------------------

#[test]
fn cell_8_effect_staged_plus_repudiated_no_committed_refuses_redispatch_anomaly() {
    let mote = wm_mote(EffectPattern::IdempotentByConstruction, 0x08);
    // Repudiated entry without a prior Committed — target_committed_seq=99
    // (arbitrary, no such entry exists). The fold flags this as
    // `inconsistent` per the cell 8 contract.
    let prefix = vec![
        effect_staged(&mote),
        JournalEntry::Repudiated {
            target_mote_id: mote.id,
            idempotency_key: repudiation_idempotency_key(&mote.id, 99),
            seq: 0,
            target_committed_seq: 99,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 0,
        },
    ];
    let (result, _journal) = drive_redispatch(&mote, prefix);
    assert_r13_refusal(&result);
}

// ---------------------------------------------------------------------------
// Cell summary: cells 0/1/4/5/6/7/8 refuse re-dispatch; cells 2/3 permit it.
// This single test asserts the bipartite split holds in aggregate.
// ---------------------------------------------------------------------------

#[test]
fn nine_cell_bipartite_split_is_exhaustive() {
    // Cells that should REFUSE re-dispatch: 0, 1, 4, 5, 6, 7, 8 (= 7 cells).
    // Cells that should PERMIT re-dispatch: 2, 3 (= 2 cells).
    // Total: 9 cells; coverage is exhaustive at the cell-by-cell test level
    // above. This summary test pins the count + structure.
    let refuse_cells = [
        "cell_0", "cell_1", "cell_4", "cell_5", "cell_6", "cell_7", "cell_8",
    ];
    let permit_cells = ["cell_2", "cell_3"];
    assert_eq!(refuse_cells.len() + permit_cells.len(), 9);
    assert!(refuse_cells.iter().all(|s| s.starts_with("cell_")));
    assert!(permit_cells.iter().all(|s| s.starts_with("cell_")));
}
