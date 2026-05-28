//! The drive loop: the single-process integration of scheduler + executor +
//! broker + resource manager, coordinating **only through the journal**.
//!
//! Each pass folds the journal into the projection, asks the projection which
//! Mote is actionable next, routes it to the right executor lifecycle entry
//! (`run_pure_mote` for PURE; `run_wm_mote` / `redispatch_wm_mote` for
//! WORLD-MUTATING + READ-ONLY-NONDET), then re-folds. The scheduler holds the
//! submitted Motes (which registers them + their edges in the projection); the
//! ready set comes from the journal-folded projection — so the scheduler and
//! executor never message each other directly, satisfying the P1 invariant
//! "scheduler/executor talk only through the log."

use std::collections::BTreeMap;
use std::sync::Arc;

use kx_capability::EffectRequest;
use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{
    redispatch_wm_mote, run_pure_mote, run_wm_mote, LocalResourceManager, StandardCommitProtocol,
    TestMoteExecutor,
};
use kx_journal::{Journal, SqliteJournal};
use kx_mote::{MoteId, NdClass};
use kx_projection::{MoteState, Projection};
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_warrant::{FsScope, NetScope};

use crate::broker::DemoBroker;
use crate::config::{Mode, RuntimeConfig};
use crate::crash::CrashPoint;
use crate::digest::{digest_projection, ProjectionDigest};
use crate::error::RuntimeError;
use crate::topology;
use crate::workflow::{DemoWorkflow, WorkflowMote};

/// The result of driving the workflow to a stopping point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    /// Number of workflow Motes in a `Committed` (non-repudiated) state.
    pub committed: usize,
    /// Total workflow Motes.
    pub total: usize,
    /// Deterministic digest of the committed-result set.
    pub digest: ProjectionDigest,
}

impl RunOutcome {
    /// Whether every workflow Mote committed.
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.committed == self.total
    }
}

/// What to do with the next actionable Mote. Owned (a cheap `WorkflowMote`
/// clone) so the runnable set can be extended with materialized shaper children
/// within the same loop iteration without a borrow conflict.
enum Action {
    RunPure(WorkflowMote),
    RunWm {
        wm: WorkflowMote,
        /// `true` ⇒ recover an in-flight WM Mote (re-dispatch); `false` ⇒ fresh.
        recover: bool,
    },
}

/// Run (or replay) the demo workflow per `config`. Returns the final outcome,
/// or aborts the process at the configured crash point (which never returns).
pub fn run(config: &RuntimeConfig) -> Result<RunOutcome, RuntimeError> {
    let workflow = DemoWorkflow::canonical();

    let store = Arc::new(LocalFsContentStore::open(&config.content_root)?);
    let journal = Arc::new(SqliteJournal::open(&config.journal_path)?);
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    let submission_motes = workflow.submission_motes();

    // The shaper's def + warrant drive topology materialization. Stage its
    // warrant bytes BEFORE building the projection: on replay,
    // `from_journal_with_materializer` folds the shaper's committed entry
    // immediately and the materializer must be able to fetch the warrant to
    // narrow each child (PR 11.5 / KG-1-close).
    let shaper_wm = workflow
        .motes
        .iter()
        .find(|w| w.mote.id == workflow.shaper_id)
        .cloned()
        .ok_or_else(|| RuntimeError::Config("workflow is missing its shaper".into()))?;
    store.put(&topology::encode_warrant(&shaper_wm.warrant)?)?;

    // The shaper's effect is its TopologyDecision: the broker stages those exact
    // canonical bytes, so the shaper's committed result_ref is the decision's
    // hash and the materializer can decode it back.
    let topology_decision = topology::demo_topology_decision();
    let mut responses = BTreeMap::new();
    responses.insert(
        workflow.shaper_id,
        topology::encode_topology_decision(&topology_decision)?,
    );
    let broker = Arc::new(DemoBroker::new(
        store.clone(),
        responses,
        config.crash_at,
        Some(workflow.stc_crash_target),
    ));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker.clone());

    // Build the projection THROUGH the materializer so every fold of a shaper's
    // Committed entry re-derives its children (deterministically, incl. replay).
    let materializer =
        topology::build_materializer(store.clone(), &shaper_wm.mote.def, &shaper_wm.warrant);
    let mut projection = Projection::from_journal_with_materializer(&*journal, materializer)?;
    // `from_journal_*` already folded the existing journal; record how far so the
    // incremental fold doesn't re-apply (and trip `DuplicateCommitted`).
    let mut folded_through: u64 = journal.current_seq()?;

    // Register every declared workflow Mote (its edges) in the projection.
    let mut scheduler = Scheduler::new(LocalPlacement);
    for w in &workflow.motes {
        scheduler.submit(w.mote.clone(), w.warrant.clone(), &mut projection)?;
    }

    // The runnable set grows when the shaper materializes children (which the
    // engine re-derives into runnable Motes, identity-matched to the projection).
    let mut runnable: Vec<WorkflowMote> = workflow.motes.clone();
    let mut children_derived = false;

    // The drive loop.
    loop {
        let Some(action) = pick_next(&runnable, &projection) else {
            break;
        };
        match action {
            Action::RunPure(w) => {
                run_pure_mote(&w.mote, &w.warrant, &*journal, &rm, &executor)?;
            }
            Action::RunWm { wm, recover } => {
                let request = effect_request_for(&wm);
                if recover {
                    redispatch_wm_mote(
                        &wm.mote,
                        &wm.warrant,
                        wm.capability.clone(),
                        request,
                        &submission_motes,
                        &*journal,
                        &rm,
                        &protocol,
                        &projection,
                    )?;
                } else {
                    run_wm_mote(
                        &wm.mote,
                        &wm.warrant,
                        wm.capability.clone(),
                        request,
                        &submission_motes,
                        &*journal,
                        &rm,
                        &protocol,
                    )?;
                }

                // Scenario-2 injection: a hard kill the instant M3's Committed
                // is durable (and the critic's Proposed has been recorded),
                // before the run finishes. Recovery must RE-READ M3, never
                // re-run its world effect.
                if config.crash_at == Some(CrashPoint::PostCommitVtc)
                    && wm.mote.id == workflow.vtc_crash_target
                {
                    fold_new(&journal, &mut projection, &mut folded_through)?;
                    if projection.state_of(&workflow.vtc_crash_target) == MoteState::Committed {
                        CrashPoint::PostCommitVtc.abort_now();
                    }
                }
            }
        }
        fold_new(&journal, &mut projection, &mut folded_through)?;

        // Once the shaper commits, re-derive its children as runnable Motes
        // (identity-matched to the materializer's registrations) so they
        // actually execute rather than merely materialize.
        if !children_derived && projection.state_of(&workflow.shaper_id) == MoteState::Committed {
            if let Some(shaper_result_ref) = projection.result_ref_of(&workflow.shaper_id) {
                let children = topology::derive_child_motes(
                    &shaper_wm.mote,
                    shaper_result_ref,
                    &topology_decision,
                    &shaper_wm.warrant,
                    &shaper_wm.capability,
                );
                runnable.extend(children);
                children_derived = true;
            }
        }
    }

    let outcome = outcome(&runnable, &projection);
    if config.mode == Mode::Run && !outcome.is_complete() {
        // A clean run that didn't finish means a Mote is stuck (e.g. a WM Mote
        // with no EffectStaged hint the oracle refuses to re-dispatch).
        return Err(RuntimeError::Stalled(outcome.total - outcome.committed));
    }
    Ok(outcome)
}

/// Fold the journal entries appended since `folded_through` into `projection`,
/// advancing `folded_through` to the journal's current seq.
///
/// Incremental by design: an append-only log only ever grows, so we read the
/// bounded range `(folded_through, current_seq]` rather than re-scanning the
/// whole journal each pass. (Re-scanning was both O(n²) and — because SQLite
/// binds the range as signed `i64` — a correctness trap: a `u64::MAX` upper
/// bound wraps to `-1` and silently returns no rows.)
fn fold_new(
    journal: &SqliteJournal,
    projection: &mut Projection,
    folded_through: &mut u64,
) -> Result<(), RuntimeError> {
    let current = journal.current_seq()?;
    if current <= *folded_through {
        return Ok(());
    }
    for entry in journal.read_entries_by_seq((*folded_through + 1)..(current + 1))? {
        projection.fold(&entry)?;
    }
    *folded_through = current;
    Ok(())
}

/// Choose the next Mote to act on, in submission order: a `Pending` Mote whose
/// parents are committed (fresh run), or an in-flight Mote to recover.
fn pick_next(runnable: &[WorkflowMote], projection: &Projection) -> Option<Action> {
    let ready = projection.ready_set();
    for w in runnable {
        let id = w.mote.id;
        let state = projection.state_of(&id);
        if matches!(state, MoteState::Committed | MoteState::Repudiated) {
            continue;
        }
        let in_ready = ready.contains(&id);
        let scheduled = state == MoteState::Scheduled;

        if w.mote.nd_class() == NdClass::Pure {
            // PURE is recomputable: run when ready, or re-run if it was left
            // in-flight by a crash (or proposed as a critic by the VTC path).
            if in_ready || scheduled {
                return Some(Action::RunPure(w.clone()));
            }
        } else if in_ready {
            return Some(Action::RunWm {
                wm: w.clone(),
                recover: false,
            });
        } else if scheduled && projection.can_redispatch_world_effect(&id) {
            // In-flight WM/ROND with an EffectStaged hint — safe to re-dispatch
            // (the broker's idempotency-key dedup makes the external effect
            // exactly-once). Without the hint the oracle refuses, and the Mote
            // is correctly left stuck rather than risking a double effect.
            return Some(Action::RunWm {
                wm: w.clone(),
                recover: true,
            });
        }
    }
    None
}

/// Build the [`EffectRequest`] for a WORLD-MUTATING / READ-ONLY-NONDET Mote.
fn effect_request_for(w: &WorkflowMote) -> EffectRequest {
    EffectRequest {
        payload: Vec::new(),
        pattern: w.mote.def.effect_pattern,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

fn outcome(runnable: &[WorkflowMote], projection: &Projection) -> RunOutcome {
    let committed = runnable
        .iter()
        .filter(|w| projection.state_of(&w.mote.id) == MoteState::Committed)
        .count();
    RunOutcome {
        committed,
        total: runnable.len(),
        digest: digest_projection(projection),
    }
}

/// Compute the digest of the on-disk journal in a fresh projection — the
/// "different machine replays to a bit-identical projection" surface.
pub fn digest_only(config: &RuntimeConfig) -> Result<ProjectionDigest, RuntimeError> {
    let journal = SqliteJournal::open(&config.journal_path)?;
    crate::digest::digest_journal(&journal)
}

/// Convenience accessor: the canonical workflow's Mote ids by role, for tests
/// and the kill-and-replay harness.
#[must_use]
pub fn canonical_targets() -> (MoteId, MoteId) {
    let w = DemoWorkflow::canonical();
    (w.stc_crash_target, w.vtc_crash_target)
}

/// All canonical workflow Mote ids (submission order) — for assertions.
#[must_use]
pub fn canonical_mote_ids() -> Vec<MoteId> {
    DemoWorkflow::canonical()
        .motes
        .iter()
        .map(|w: &WorkflowMote| w.mote.id)
        .collect()
}
