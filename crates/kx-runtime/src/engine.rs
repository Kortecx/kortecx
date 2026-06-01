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
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kx_capability::EffectRequest;
use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{
    redispatch_wm_mote, run_native_critic_mote, run_pure_mote, run_wm_mote, CommitProtocol,
    LocalResourceManager, MoteExecutor, StandardCommitProtocol, TestMoteExecutor,
};
use kx_journal::{Journal, SqliteJournal};
use kx_mote::{MoteId, NdClass, TopologyDecision};
use kx_projection::{
    CheckpointOutcome, ContentStoreVerdicts, MoteState, Projection, VerdictLookup,
};
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_warrant::{FsScope, NetScope};

use crate::broker::DemoBroker;
use crate::checkpoint_io;
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
// The shared `Run` prefix is intentional — every variant is a dispatch action.
#[allow(clippy::enum_variant_names)]
enum Action {
    RunPure(WorkflowMote),
    /// A native deterministic-critic Mote (`critic_check = Some`): evaluate the
    /// declared check in-process against the producer's committed bytes and
    /// commit a `CriticVerdict` (P4.2-2). Routed ahead of `RunPure` even though
    /// a critic is PURE, because its body is the check, not an executor spawn.
    RunNativeCritic(WorkflowMote),
    RunWm {
        wm: WorkflowMote,
        /// `true` ⇒ recover an in-flight WM Mote (re-dispatch); `false` ⇒ fresh.
        recover: bool,
    },
}

/// Run (or replay) the canonical demo workflow per `config`. Returns the final
/// outcome, or aborts the process at the configured crash point (which never
/// returns).
///
/// This is the thin demo-defaults wrapper over [`run_with_seams`]: it wires the
/// deterministic stub seams (`TestMoteExecutor::deterministic()` + `DemoBroker`)
/// and the canonical topology shaper, then drives the real orchestrator. Its
/// output is byte-identical to the pre-seam engine (digest `a6b5c679…`, 8/8) —
/// the seam injects, it does not change, the truth path.
pub fn run(config: &RuntimeConfig) -> Result<RunOutcome, RuntimeError> {
    let workflow = DemoWorkflow::canonical();

    let store = Arc::new(LocalFsContentStore::open(&config.content_root)?);
    let journal = Arc::new(SqliteJournal::open(&config.journal_path)?);
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();

    // The shaper's def + warrant drive topology materialization (resolved inside
    // `run_with_seams`). Find it up front so the broker can stage the exact
    // `TopologyDecision` bytes as the shaper's effect response.
    let shaper_wm = workflow
        .motes
        .iter()
        .find(|w| w.mote.id == workflow.shaper_id)
        .cloned()
        .ok_or_else(|| RuntimeError::Config("workflow is missing its shaper".into()))?;

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

    run_with_seams(
        config,
        &workflow,
        store,
        journal,
        &rm,
        &executor,
        &protocol,
        Some((&shaper_wm, &topology_decision)),
    )
}

/// The real single-process orchestrator, parameterized over the injected seams.
///
/// `run()` calls this with the demo stubs + canonical shaper; the `kx-model-harness`
/// crate calls it with a real `InferenceBackend`-backed [`MoteExecutor`] + a
/// model/tool `CapabilityBroker` and its own (shaperless) workflows. The body —
/// projection fold, `pick_next`, the PURE / native-critic / WM / re-dispatch
/// routing, the P4.2-3 `ready_set_promoted` exit gate, exactly-once via
/// `serve_if_committed`, the crash-injection windows — is the SAME code on every
/// run; only the executor + commit protocol (broker) + workflow vary. This is the
/// thesis-test seam: distribution / real-model is wiring, not a rewrite.
///
/// `shaper` carries the topology shaper + its decision when the workflow has one
/// (the canonical demo); pass `None` for a flat DAG (the harness's A–J workflows),
/// in which case the topology materializer + child re-derivation are skipped.
// `store` / `journal` are taken by value: the orchestrator owns these `Arc`
// handles for the run (cloning them into the verdict lookup + materializer +
// commit calls), so `needless_pass_by_value` is intentional here — the caller
// hands off cheap `Arc` clones and keeps its own (matches `kx-executor`'s
// crate-level allow of the same lint).
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]
pub fn run_with_seams<S, E, CP>(
    config: &RuntimeConfig,
    workflow: &DemoWorkflow,
    store: Arc<S>,
    journal: Arc<SqliteJournal>,
    rm: &LocalResourceManager,
    executor: &E,
    protocol: &CP,
    shaper: Option<(&WorkflowMote, &TopologyDecision)>,
) -> Result<RunOutcome, RuntimeError>
where
    S: ContentStore + Send + Sync + 'static,
    E: MoteExecutor + ?Sized,
    CP: CommitProtocol + ?Sized,
{
    // Reads committed `CriticVerdict`s by content-address for the P4.2-3
    // promotion gate (shares the one store via Arc).
    let verdicts = ContentStoreVerdicts::new(store.clone());
    let submission_motes = workflow.submission_motes();

    // Build the projection. With a shaper, stage its warrant bytes BEFORE
    // building the projection (on replay `from_journal_with_materializer` folds
    // the shaper's committed entry immediately and the materializer must fetch
    // the warrant to narrow each child — PR 11.5 / KG-1-close), then fold
    // through the materializer so every fold of a shaper's Committed entry
    // re-derives its children deterministically (incl. replay). Without a
    // shaper (flat DAG), a plain journal fold suffices.
    // M2.2b — discardable-checkpoint live recovery. Load any sidecar next to the
    // journal and seed the cold fold from it (re-folding only the tail
    // `(offset, head]`). The checkpoint is NEVER authoritative: a missing /
    // corrupt / stale / wrong-run sidecar is silently discarded and the full
    // fold runs, so recovery is bit-identical with or without it. The trust
    // boundary: the sidecar lives in the journal's own data dir under the
    // journal's permissions — anyone who can forge it can already forge the
    // authoritative journal. The journaled digest seal (M2.2c) will anchor this
    // to the journal for unforgeability.
    let sidecar_path = checkpoint_io::sidecar_path(&config.journal_path);
    let checkpoint = checkpoint_io::read_checkpoint(&sidecar_path);
    let recovery_start = Instant::now();
    let (mut projection, recovery_outcome) = if let Some((shaper_wm, _)) = shaper {
        store.put(&topology::encode_warrant(&shaper_wm.warrant)?)?;
        let materializer =
            topology::build_materializer(store.clone(), &shaper_wm.mote.def, &shaper_wm.warrant);
        Projection::from_journal_with_checkpoint_with_materializer_reported(
            &*journal,
            materializer,
            checkpoint.as_ref(),
        )?
    } else {
        Projection::from_journal_with_checkpoint_reported(&*journal, checkpoint.as_ref())?
    };
    // `from_journal_*` already folded the existing journal; record how far so the
    // incremental fold doesn't re-apply (and trip `DuplicateCommitted`).
    let mut folded_through: u64 = journal.current_seq()?;
    log_recovery(recovery_outcome, folded_through, recovery_start.elapsed());
    // Seed the cadence counter from the recovered frontier so the first
    // post-recovery checkpoint fires after N *new* entries, not immediately.
    let mut last_checkpoint_at = folded_through;

    // Register every declared workflow Mote (its edges) in the projection.
    let mut scheduler = Scheduler::new(LocalPlacement);
    for w in &workflow.motes {
        scheduler.submit(w.mote.clone(), w.warrant.clone(), &mut projection)?;
    }

    // The runnable set grows when the shaper materializes children (which the
    // engine re-derives into runnable Motes, identity-matched to the projection).
    let mut runnable: Vec<WorkflowMote> = workflow.motes.clone();
    // A shaperless workflow never derives children — start "done".
    let mut children_derived = shaper.is_none();
    let mut child_ids: Vec<MoteId> = Vec::new();

    // The drive loop.
    loop {
        // Re-derive the committed shaper's materialized children into the runnable
        // set **before** deciding whether to stop (P0.6 / P3.4 — the hardest path):
        // on recovery the shaper's `Committed` is already folded, so a fresh process
        // re-materializes the SAME children and runs them, never re-running the shaper
        // to re-decide. Doing this after `pick_next` would let a recovery where only
        // the children remain break first, orphaning the shaper's decision.
        if !children_derived {
            if let Some((shaper_wm, topology_decision)) = shaper {
                children_derived = derive_shaper_children(
                    workflow,
                    shaper_wm,
                    topology_decision,
                    &projection,
                    &mut runnable,
                    &mut child_ids,
                );
            }
        }

        let Some(action) = pick_next(&runnable, &projection, &verdicts) else {
            break;
        };
        match action {
            Action::RunPure(w) => {
                crash_if_children_pending(config, &child_ids, w.mote.id);
                run_pure_mote(&w.mote, &w.warrant, &*journal, rm, executor)?;
            }
            Action::RunNativeCritic(w) => {
                // Evaluate the declared check in-process against the producer's
                // committed bytes and commit a CriticVerdict (P4.2-2). The
                // verdict drives the P4.2-3 promotion gate on the next fold.
                run_native_critic_mote(&w.mote, &w.warrant, &*journal, &*store)?;
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
                        rm,
                        protocol,
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
                        rm,
                        protocol,
                    )?;
                }

                // Scenario-2 injection: a hard kill the instant the VTC Mote's
                // Committed is durable (and the critic's Proposed has been
                // recorded), before the run finishes. Recovery must RE-READ it,
                // never re-run its world effect.
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
        // M2.2b cadence: persist a checkpoint every N folded entries. Fired ONLY
        // here, at the contiguously-drained loop-bottom frontier `[1, folded_through]`
        // (never at the inner fold_new in the crash window above), so the
        // `fold_checkpoint` precondition holds. Non-fatal on write failure.
        maybe_checkpoint(
            config,
            &projection,
            &sidecar_path,
            folded_through,
            &mut last_checkpoint_at,
        );
    }

    // Graceful-completion checkpoint: leave a fresh sidecar so a restart of a
    // completed/drained run seeds the full final state and folds an empty tail.
    // Skipped if the cadence already captured this exact frontier.
    if config.checkpoint_every.is_some() && folded_through > last_checkpoint_at {
        write_checkpoint(&projection, &sidecar_path, folded_through);
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

/// Emit recovery observability (M2.2b): one structured event recording whether
/// the discardable checkpoint seeded the fold (and the tail length) or the full
/// log was folded (and why), plus the recovery duration. Purely diagnostic — the
/// folded state is bit-identical either way.
fn log_recovery(outcome: CheckpointOutcome, head: u64, elapsed: Duration) {
    let elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    match outcome {
        CheckpointOutcome::Seeded {
            offset,
            tail_entries,
        } => tracing::info!(
            head,
            seed_offset = offset,
            tail_entries,
            elapsed_us,
            "recovery seeded from checkpoint",
        ),
        CheckpointOutcome::FullFold { reason } => tracing::info!(
            head,
            ?reason,
            elapsed_us,
            "recovery full-folded (no usable checkpoint)",
        ),
    }
}

/// Persist a checkpoint when the cadence is due (M2.2b). Fires iff checkpointing
/// is enabled and at least `N` entries were folded since the last checkpoint.
/// MUST be called only at a contiguously-drained frontier (the loop-bottom
/// `fold_new`), so the captured `fold_checkpoint` reflects a clean prefix.
fn maybe_checkpoint(
    config: &RuntimeConfig,
    projection: &Projection,
    sidecar_path: &Path,
    folded_through: u64,
    last_checkpoint_at: &mut u64,
) {
    let Some(cadence) = config.checkpoint_every else {
        return;
    };
    if folded_through == 0 || folded_through.saturating_sub(*last_checkpoint_at) < cadence {
        return;
    }
    write_checkpoint(projection, sidecar_path, folded_through);
    *last_checkpoint_at = folded_through;
}

/// Capture the projection's current `FoldCheckpoint` and persist it atomically.
/// A write failure is **non-fatal** — the checkpoint is never authoritative, so a
/// failed persist only means the next restart full-folds. Logged, never returned.
fn write_checkpoint(projection: &Projection, sidecar_path: &Path, folded_through: u64) {
    let bytes = projection.fold_checkpoint().to_bytes();
    match checkpoint_io::write_atomic(sidecar_path, &bytes) {
        Ok(()) => tracing::debug!(
            through = folded_through,
            bytes = bytes.len(),
            "checkpoint persisted"
        ),
        Err(error) => tracing::warn!(
            through = folded_through,
            %error,
            "checkpoint persist failed (continuing; recovery falls back to full fold)"
        ),
    }
}

/// Choose the next Mote to act on, in submission order: a `Pending` Mote whose
/// parents are committed (fresh run), or an in-flight Mote to recover.
/// Scenario-3 crash injection (P0.6 / P3.4): a hard kill the instant the first
/// materialized child is about to run — the shaper's decision + every declared Mote are
/// committed, the children are pending. Recovery must REPLAY the committed decision
/// (re-materialize + run the same children), never re-run the shaper to re-decide.
/// No-op unless that crash point is configured.
fn crash_if_children_pending(config: &RuntimeConfig, child_ids: &[MoteId], mote_id: MoteId) {
    if config.crash_at == Some(CrashPoint::ShaperChildrenPending) && child_ids.contains(&mote_id) {
        CrashPoint::ShaperChildrenPending.abort_now();
    }
}

/// If the shaper has committed, re-derive its materialized children (D49 — identity
/// from journal facts only) into `runnable` + `child_ids`, returning `true`. Pure
/// function of the committed shaper entry: a fresh recovery process derives the SAME
/// children (P0.6 / P3.4 — replay the decision, never re-decide). Returns `false` while
/// the shaper is uncommitted.
fn derive_shaper_children(
    workflow: &DemoWorkflow,
    shaper_wm: &WorkflowMote,
    topology_decision: &TopologyDecision,
    projection: &Projection,
    runnable: &mut Vec<WorkflowMote>,
    child_ids: &mut Vec<MoteId>,
) -> bool {
    if projection.state_of(&workflow.shaper_id) != MoteState::Committed {
        return false;
    }
    let Some(shaper_result_ref) = projection.result_ref_of(&workflow.shaper_id) else {
        return false;
    };
    let children = topology::derive_child_motes(
        &shaper_wm.mote,
        shaper_result_ref,
        topology_decision,
        &shaper_wm.warrant,
        &shaper_wm.capability,
    );
    child_ids.extend(children.iter().map(|c| c.mote.id));
    runnable.extend(children);
    true
}

fn pick_next(
    runnable: &[WorkflowMote],
    projection: &Projection,
    verdicts: &dyn VerdictLookup,
) -> Option<Action> {
    // The P4.2-3 exit gate: a WORLD-MUTATING producer's consumers are withheld
    // until its deterministic critic commits a `Valid` verdict. For workflows
    // without deterministic critics (e.g. the canonical demo) every producer is
    // `NotApplicable`, so this is byte-identical to the un-gated `ready_set()`.
    let ready = projection.ready_set_promoted(verdicts);
    for w in runnable {
        let id = w.mote.id;
        let state = projection.state_of(&id);
        if matches!(state, MoteState::Committed | MoteState::Repudiated) {
            continue;
        }
        let in_ready = ready.contains(&id);
        let scheduled = state == MoteState::Scheduled;

        if w.mote.def.critic_check.is_some() {
            // A native deterministic critic — PURE, but its body is the
            // in-process check, not an executor spawn (P4.2-2).
            if in_ready || scheduled {
                return Some(Action::RunNativeCritic(w.clone()));
            }
        } else if w.mote.nd_class() == NdClass::Pure {
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
///
/// **M2.2b invariant:** this path MUST stay a pure full fold (`from_journal`,
/// never `from_journal_with_checkpoint`). The product digest is the cross-process
/// comparison surface — seeding it from a discardable sidecar would make the
/// computed digest depend on whether a `.ckpt` happens to be present, even though
/// the value is identical. Keep the canonical digest a pure function of the log.
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
