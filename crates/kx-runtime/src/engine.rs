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

use kx_audit::{AuditEvent, DispatchKind};
use kx_capability::EffectRequest;
use kx_capture::StepRecord;
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_executor::{
    redispatch_wm_mote, run_native_critic_mote, run_pure_mote, run_wm_mote, CommitProtocol,
    LifecycleError, LocalResourceManager, MoteExecutor, StandardCommitProtocol, TestMoteExecutor,
};
use kx_journal::{Journal, JournalEntry, SqliteJournal};
use kx_mote::{MoteId, NdClass, TopologyDecision};
use kx_projection::{
    CheckpointOutcome, ContentStoreVerdicts, MoteState, Projection, VerdictLookup,
};
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_warrant::{FsScope, NetScope};

use crate::audit_sink::RuntimeAuditSink;
use crate::broker::DemoBroker;
use crate::capture_sink::CaptureSink;
use crate::checkpoint_io;
use crate::config::{Mode, RuntimeConfig};
use crate::crash::CrashPoint;
use crate::digest::{digest_projection, ProjectionDigest};
use crate::error::RuntimeError;
use crate::failure_policy::{classify_lifecycle_error, reason_for, FailureClass, FailurePolicy};
use crate::snapshot_sink::SnapshotSink;
use crate::topology::{self, TopologyProvider};
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

impl Action {
    /// The Mote being dispatched (R4 audit echo — already-derived, no recompute).
    fn mote_id(&self) -> MoteId {
        match self {
            Action::RunPure(w) | Action::RunNativeCritic(w) => w.mote.id,
            Action::RunWm { wm, .. } => wm.mote.id,
        }
    }

    /// Its non-determinism class.
    fn nd_class(&self) -> NdClass {
        match self {
            Action::RunPure(w) | Action::RunNativeCritic(w) => w.mote.nd_class(),
            Action::RunWm { wm, .. } => wm.mote.nd_class(),
        }
    }

    /// Which dispatch path this action takes (for the R4 audit trail).
    fn dispatch_kind(&self) -> DispatchKind {
        match self {
            Action::RunPure(_) => DispatchKind::Pure,
            Action::RunNativeCritic(_) => DispatchKind::Critic,
            Action::RunWm { recover: false, .. } => DispatchKind::WmFresh,
            Action::RunWm { recover: true, .. } => DispatchKind::WmRecovery,
        }
    }
}

/// Run (or replay) the canonical demo workflow per `config`. Returns the final
/// outcome, or aborts the process at the configured crash point (which never
/// returns).
///
/// This is the thin demo-defaults wrapper over [`run_with_capture`] /
/// [`run_with_seams`]: it wires the deterministic stub seams
/// (`TestMoteExecutor::deterministic()` + `DemoBroker`) and the canonical topology
/// shaper, then drives the real orchestrator. Its output is byte-identical to the
/// pre-seam engine (digest `a6b5c679…`, 8/8) — the seam injects, it does not
/// change, the truth path.
///
/// **D67:** capture is ON by default (`CaptureScope::ActionsOnly`). The ledger is
/// internal here (the demo discards it); a caller that wants the captured
/// `MoteId → action result_ref` ledger drives [`run_with_capture`] with its own
/// [`CaptureSink`]. Capture is OFF the truth path — it is never journaled — so the
/// digest is byte-unchanged whether capture is on, off, or inspected.
pub fn run(config: &RuntimeConfig) -> Result<RunOutcome, RuntimeError> {
    let capture = CaptureSink::actions_only();
    run_with_capture(config, Some(&capture))
}

/// As [`run`], but drives the canonical demo with the caller's `capture_sink`, so
/// the captured action ledger (D67) is inspectable after the run. `None` disables
/// capture entirely (the byte-identity-without-overhead path). Because capture is
/// OFF the truth path (never journaled, never a `MoteId` input, never a gate), the
/// reported digest is identical for any `capture_sink` — `None`, `ActionsOnly`, or
/// `Full`.
pub fn run_with_capture(
    config: &RuntimeConfig,
    capture_sink: Option<&CaptureSink>,
) -> Result<RunOutcome, RuntimeError> {
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

    // R4: enable the off-truth-path audit log iff the operator passed
    // `--audit-log <path>` (a `kx run --audit-log` JSONL trail, truncated fresh per
    // run). Open failure is surfaced here (fail-fast on a bad path), never mid-run.
    // Audit is OFF the truth path, so the digest `a6b5c679…` is byte-unchanged
    // whether the log is on or off.
    let audit = match &config.audit_log {
        Some(path) => Some(RuntimeAuditSink::jsonl(path)?),
        None => None,
    };

    run_with_seams(
        config,
        &workflow,
        store,
        journal,
        &rm,
        &executor,
        &protocol,
        Some((&shaper_wm, &topology_decision)),
        // No model drives the demo's topology — the decision is the hardcoded
        // `demo_topology_decision()`, supplied directly. `None` keeps arg-based
        // child derivation (byte-identical truth path).
        None,
        // The deterministic demo assembles no context: `None` ⇒ no snapshot is
        // ever published ⇒ the truth path (digest `a6b5c679…`) is byte-unchanged.
        None,
        capture_sink,
        audit.as_ref(),
        // The canonical demo never fails a Mote, so no policy is needed; `None`
        // keeps the legacy abort-on-failure semantics (byte-identical truth path).
        None,
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
///
/// `topology_provider` is the PR-2 (F-4) "model drives the loop" seam. When `Some`,
/// the shaper's [`TopologyDecision`] was produced by a model (a
/// [`TopologyProvider`], lowered + committed as the shaper's `result_ref`), so the
/// engine derives the shaper's runnable children from that **committed fact**
/// (decoding the `result_ref`) rather than the caller-supplied `shaper` decision —
/// guaranteeing the runnable set equals the materializer-registered set (one source
/// of truth). When `None` (the canonical demo + every existing caller), children are
/// derived from the supplied decision exactly as before — the deterministic truth
/// path (digest `a6b5c679…`) is byte-unchanged. (The provider is invoked eagerly by
/// the harness in PR-2; PR-3 re-plan will invoke it lazily per round — same seam.)
///
/// `snapshot_sink` is the D78 context-publishing seam: when `Some`, the
/// orchestrator publishes the current committed-state [`kx_projection::Snapshot`]
/// to it immediately before each dispatch, so a real-model executor/broker can
/// assemble the Mote's upstream context + tool menu
/// (`kx_context_assembler::assemble`). The canonical demo passes `None` — no
/// snapshot is published and the deterministic truth path is byte-unchanged
/// (the published snapshot is model input only; it never enters identity or the
/// journal, D64).
///
/// `failure_policy` is the PR-1 bounded-retry + dead-letter seam. When `Some`, a
/// Mote dispatch error no longer aborts the run: a transient infrastructure
/// failure is retried (bounded by `max_attempts`), and a terminal failure (or an
/// exhausted transient) is journaled as a `Failed` fact so the drive loop continues
/// **past** the dead-lettered Mote. The canonical demo + every existing caller pass
/// `None`, so a dispatch error propagates exactly as the pre-PR-1 `?` did — the
/// deterministic truth path (digest `a6b5c679…`) is byte-unchanged. (A dead-lettered
/// `Failed` is the durable, auditable record the model-driven re-plan loop (AL2)
/// later reads — kortecx never blindly re-runs a failing Mote.)
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
    topology_provider: Option<&dyn TopologyProvider>,
    snapshot_sink: Option<&SnapshotSink>,
    capture_sink: Option<&CaptureSink>,
    audit_sink: Option<&RuntimeAuditSink>,
    failure_policy: Option<&FailurePolicy>,
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
        // PR-2 (F-4): a model-driven loop supplies its own materializer (resolving
        // the model's proposed roles); the demo uses the hardcoded `demo-worker`
        // registry. Both read `store` (the provider's materializer is contracted to
        // use the run's store). `None` ⇒ byte-identical demo path.
        let materializer = match topology_provider {
            Some(provider) => provider.materializer(&shaper_wm.mote.def, &shaper_wm.warrant),
            None => {
                topology::build_materializer(store.clone(), &shaper_wm.mote.def, &shaper_wm.warrant)
            }
        };
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
    // R4: a resume folded an existing journal before the drive loop ran. Record
    // the recovered frontier off the truth path. `folded_through` (the journal
    // seq) is already in hand; the committed count is a ONE-TIME scan of the
    // just-folded projection, run only when auditing is enabled — zero cost when
    // `None`. (Per-Mote `MoteCommitted` events for the recovered set are emitted by
    // the terminal sweep at run end, so they cover recovery-committed Motes too.)
    if folded_through > 0 {
        if let Some(a) = audit_sink {
            let committed_through = projection
                .iter_motes()
                .filter(|(_, s)| *s == MoteState::Committed)
                .count();
            a.record(AuditEvent::Recovered {
                committed_through: as_u32(committed_through),
                folded_through,
            });
        }
    }
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
    // PR-1: per-Mote transient-retry counter (in-memory, off the truth path — a
    // crash simply re-folds the committed prefix and the budget restarts). Only
    // consulted when `failure_policy` is `Some`; empty + untouched otherwise.
    let mut attempts: BTreeMap<MoteId, u32> = BTreeMap::new();

    // R4: the drive loop is starting. Off the truth path — `None` ⇒ no event.
    if let Some(a) = audit_sink {
        a.record(AuditEvent::RunStarted {
            runnable: as_u32(runnable.len()),
        });
    }

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
                // PR-2 (F-4): a model-driven shaper derives children from the
                // COMMITTED decision fact (`topology_provider` is `Some`); the demo
                // + every existing caller (`None`) derive from the supplied decision,
                // byte-for-byte as before — the digest `a6b5c679…` path is untouched.
                children_derived = if topology_provider.is_some() {
                    derive_shaper_children_from_fact(
                        workflow,
                        shaper_wm,
                        &projection,
                        &*store,
                        &mut runnable,
                        &mut child_ids,
                    )?
                } else {
                    derive_shaper_children(
                        workflow,
                        shaper_wm,
                        topology_decision,
                        &projection,
                        &mut runnable,
                        &mut child_ids,
                    )
                };
                // R4: the committed shaper just materialized its children into the
                // runnable set. Echo the shaper id + child count off the truth path.
                if children_derived {
                    if let Some(a) = audit_sink {
                        a.record(AuditEvent::ChildrenDerived {
                            shaper: shaper_wm.mote.id,
                            children: as_u32(child_ids.len()),
                        });
                    }
                }
            }
        }

        let Some(action) = pick_next(&runnable, &projection, &verdicts) else {
            break;
        };
        // D78: publish the current committed-state snapshot so a real-model
        // executor/broker can assemble this Mote's upstream context + tool menu
        // before dispatch. No-op for the demo (`None`) — keeps the truth path
        // byte-identical. The Mote picked here has its Data parents committed
        // (it is in the ready set), so the snapshot carries their `result_ref`s.
        if let Some(sink) = snapshot_sink {
            sink.publish(projection.snapshot());
        }
        // R4: record the dispatch off the truth path (echoes the picked Mote's id +
        // nd_class + which dispatch path was taken; no recomputation, SN-8).
        if let Some(a) = audit_sink {
            a.record(AuditEvent::MoteDispatched {
                mote_id: action.mote_id(),
                nd_class: action.nd_class(),
                kind: action.dispatch_kind(),
            });
        }
        // Dispatch the picked Mote. Each lifecycle entry returns
        // `Result<_, LifecycleError>`; unify to `Result<(), LifecycleError>` so a
        // failure can be routed through the PR-1 `failure_policy` (or propagated
        // verbatim when there is none). `mote_id` + `wm_vtc_target` are captured
        // before the `match` consumes `action`.
        let mote_id = action.mote_id();
        let wm_vtc_target = matches!(
            &action,
            Action::RunWm { wm, .. } if wm.mote.id == workflow.vtc_crash_target
        );
        let dispatch: Result<(), LifecycleError> = match action {
            Action::RunPure(w) => {
                crash_if_children_pending(config, &child_ids, w.mote.id);
                run_pure_mote(&w.mote, &w.warrant, &*journal, rm, executor).map(|_| ())
            }
            Action::RunNativeCritic(w) => {
                // Evaluate the declared check in-process against the producer's
                // committed bytes and commit a CriticVerdict (P4.2-2). The
                // verdict drives the P4.2-3 promotion gate on the next fold.
                run_native_critic_mote(&w.mote, &w.warrant, &*journal, &*store).map(|_| ())
            }
            Action::RunWm { wm, recover } => {
                let request = effect_request_for(&wm);
                if recover {
                    redispatch_wm_mote(
                        &wm.mote,
                        &wm.warrant,
                        wm.capability.clone(),
                        request,
                        // Single-node demo: vacuous tool contract → no durable
                        // resolved class, so recovery uses today's probe-then-
                        // redispatch path. M2.3b class-aware routing is exercised
                        // on the coordinator path + the executor integration tests.
                        None,
                        &submission_motes,
                        &*journal,
                        rm,
                        protocol,
                        &projection,
                    )
                    .map(|_| ())
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
                    )
                    .map(|_| ())
                }
            }
        };

        // PR-1: a dispatch failure no longer aborts the run when a policy is set.
        // With no policy (the demo + every existing caller) the error propagates
        // exactly as the pre-PR-1 `?` did — byte-identical truth path.
        if let Err(e) = dispatch {
            match failure_policy {
                None => return Err(RuntimeError::Lifecycle(e)),
                Some(policy) => {
                    handle_failure(
                        &e,
                        mote_id,
                        policy,
                        &mut attempts,
                        &journal,
                        &mut projection,
                        &mut folded_through,
                    )?;
                    // Retry → `pick_next` re-selects the still-ready Mote; dead-letter
                    // → it is now `Failed` and `pick_next` skips it. Either way, re-loop.
                    continue;
                }
            }
        }

        // Scenario-2 injection (success path only): a hard kill the instant the VTC
        // Mote's Committed is durable (and the critic's Proposed has been recorded),
        // before the run finishes. Recovery must RE-READ it, never re-run its world
        // effect.
        if wm_vtc_target && config.crash_at == Some(CrashPoint::PostCommitVtc) {
            fold_new(&journal, &mut projection, &mut folded_through)?;
            if projection.state_of(&workflow.vtc_crash_target) == MoteState::Committed {
                CrashPoint::PostCommitVtc.abort_now();
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
            &journal,
            &sidecar_path,
            folded_through,
            &mut last_checkpoint_at,
        );
    }

    // Graceful-completion checkpoint: leave a fresh sidecar + seal so a restart of
    // a completed/drained run seeds the full final state and folds the (1-entry:
    // the seal) tail. Skipped if the cadence already captured this exact frontier.
    if config.checkpoint_every.is_some() && folded_through > last_checkpoint_at {
        write_checkpoint(&projection, &journal, &sidecar_path, folded_through);
    }

    // D67: capture each committed Mote's ACTION (its `result_ref`) off the truth
    // path, once, at return. A single O(committed·log) sweep over the final
    // committed projection — flat per-Mote, no super-linear per-iteration rescan.
    // The returned ledger is identical to a per-step capture because there is no
    // concurrent observer in M3.1 (live streaming is M11); a crash loses the
    // in-memory ledger anyway and recovery re-derives it from the journal. No-op
    // for `None` (the byte-identity-without-overhead path). A pure `result_ref_of`
    // read + an in-memory insert: NEVER touches the journal/projection fold, so the
    // product digest `a6b5c679…` is byte-unchanged.
    if let Some(sink) = capture_sink {
        capture_committed_actions(&runnable, &projection, sink);
    }

    let outcome = outcome(&runnable, &projection);

    // R4: the run finished. Emit the per-Mote terminal-state trail + the run
    // summary off the truth path, then flush best-effort. A single
    // O(committed·log) sweep over the FINAL projection — flat per-Mote, and
    // (unlike a per-dispatch state check) COMPLETE: it covers Motes committed by
    // recovery before the loop ran. `RunCompleted.digest` is the product digest
    // bytes (`a6b5c679…` for the canonical demo) — a tamper-evident receipt.
    // No-op for `None` (the byte-identity-without-overhead path).
    if let Some(a) = audit_sink {
        audit_terminal_states(&runnable, &projection, a);
        a.record(AuditEvent::RunCompleted {
            committed: as_u32(outcome.committed),
            total: as_u32(outcome.total),
            digest: outcome.digest.0,
        });
        a.flush();
    }

    if config.mode == Mode::Run {
        // "Stalled" means a Mote is genuinely stuck — non-terminal with no way
        // forward (e.g. a WM Mote with no EffectStaged hint the oracle refuses to
        // re-dispatch). A Mote dead-lettered by the failure policy is *terminal*
        // (`Failed`), not stuck, so the run completes rather than aborting. On the
        // demo path no Mote is non-committed, so `stuck == 0` exactly as before.
        let stuck = runnable
            .iter()
            .filter(|w| !is_terminal(projection.state_of(&w.mote.id)))
            .count();
        if stuck > 0 {
            return Err(RuntimeError::Stalled(stuck));
        }
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

/// Reporter id stamped on the single-process engine's dead-letter `Failed` entries
/// (the distributed coordinator uses its own `COORDINATOR_REPORTER_ID = 0`). A
/// distinct, fixed, UUID-shaped marker — never part of identity or dedup (a
/// `Failed` entry is not deduped, D19), purely a provenance hint for audit.
const RUNTIME_REPORTER_ID: u128 = 0x6b78_5f72_756e_7469_6d65_0000_0000_0001;

/// Whether a Mote has reached a terminal state — committed or dead-lettered. The
/// drive loop never re-picks a terminal Mote, and a `Mode::Run` that leaves only
/// terminal Motes is *complete*, not *stalled*.
fn is_terminal(state: MoteState) -> bool {
    matches!(
        state,
        MoteState::Committed | MoteState::Failed | MoteState::Repudiated | MoteState::Inconsistent
    )
}

/// PR-1 failure handling (only reached when `failure_policy` is `Some`): retry a
/// transient infrastructure error within budget, else dead-letter the Mote by
/// journaling a terminal `Failed` fact so the drive loop can continue **past** it.
///
/// First re-folds: a commit-protocol path may have already journaled its own
/// terminal `Failed` (e.g. an R-13 re-dispatch refusal), in which case the Mote is
/// already terminal and we must NOT double-write. Otherwise, a transient class is
/// retried (in-memory attempt counter, backoff) until `max_attempts`, then a single
/// `Failed{reason_class}` is appended. A terminal-logic class dead-letters at once.
///
/// Returns `Ok(())` in every handled case — the caller re-loops: a retry re-selects
/// the still-ready Mote, a dead-letter is now `MoteState::Failed` and is skipped.
fn handle_failure(
    err: &LifecycleError,
    mote_id: MoteId,
    policy: &FailurePolicy,
    attempts: &mut BTreeMap<MoteId, u32>,
    journal: &SqliteJournal,
    projection: &mut Projection,
    folded_through: &mut u64,
) -> Result<(), RuntimeError> {
    // Pick up anything the failed dispatch already journaled (e.g. a commit-protocol
    // terminal `Failed`). If the Mote is already terminal, don't double-write.
    fold_new(journal, projection, folded_through)?;
    if is_terminal(projection.state_of(&mote_id)) {
        tracing::warn!(mote = ?mote_id, error = %err, "dispatch failed; Mote already terminal — not re-writing");
        return Ok(());
    }

    let class = classify_lifecycle_error(err);
    if class == FailureClass::TransientInfra {
        let n = attempts.entry(mote_id).or_insert(0);
        *n += 1;
        if *n < policy.max_attempts {
            if !policy.backoff.is_zero() {
                std::thread::sleep(policy.backoff);
            }
            tracing::warn!(mote = ?mote_id, attempt = *n, max = policy.max_attempts, error = %err, "transient dispatch failure — retrying");
            return Ok(());
        }
    }

    // Terminal, or transient budget exhausted → dead-letter with a terminal `Failed`
    // fact. NOT re-running a failing Mote is the whole point (the user's "no point
    // re-running it"); the durable `Failed` is what a later model re-plan (AL2) reads.
    let reason_class = reason_for(class);
    let failed = JournalEntry::Failed {
        mote_id,
        idempotency_key: *mote_id.as_bytes(),
        seq: 0, // journal assigns
        reason_class,
        reporter_id: RUNTIME_REPORTER_ID,
    };
    journal.append(failed)?;
    fold_new(journal, projection, folded_through)?;
    tracing::warn!(mote = ?mote_id, ?reason_class, error = %err, "dead-lettering Mote (failsafe: a failing Mote is recorded, never blindly re-run)");
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
    journal: &SqliteJournal,
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
    write_checkpoint(projection, journal, sidecar_path, folded_through);
    *last_checkpoint_at = folded_through;
}

/// Capture the projection's current `FoldCheckpoint`, persist it atomically, then
/// (M2.2c) co-commit a journaled `DigestSealed` seal at the same frontier.
///
/// **Order matters: sidecar first, then seal.** A crash between them leaves a
/// sidecar with no anchoring seal → recovery discards it (`SealMissing`) and
/// full-folds — never the reverse. Both writes are **non-fatal**: the checkpoint
/// is never authoritative, and an orphan/absent seal only costs a slower restart.
///
/// The seal records `projection.state_digest()` (== `blake3(checkpoint payload)`)
/// at frontier `folded_through`, committed *in* the journal (the trust root), so a
/// forged-but-self-consistent sidecar cannot seed a wrong base state on restart
/// (D103.1 → unforgeable; M2.2c). The single-writer discipline lands the seal at
/// `seq = folded_through + 1`, where recovery's `journal_seal_at` looks for it.
fn write_checkpoint(
    projection: &Projection,
    journal: &SqliteJournal,
    sidecar_path: &Path,
    folded_through: u64,
) {
    let bytes = projection.fold_checkpoint().to_bytes();
    match checkpoint_io::write_atomic(sidecar_path, &bytes) {
        Ok(()) => tracing::debug!(
            through = folded_through,
            bytes = bytes.len(),
            "checkpoint persisted"
        ),
        Err(error) => {
            tracing::warn!(
                through = folded_through,
                %error,
                "checkpoint persist failed (continuing; recovery falls back to full fold)"
            );
            // No sidecar → a seal would anchor nothing. Skip it.
            return;
        }
    }
    // M2.2c: anchor the sidecar to the trust root.
    let seal = JournalEntry::DigestSealed {
        through_seq: folded_through,
        state_digest: projection.state_digest(),
        seq: 0,
    };
    match journal.append(seal) {
        Ok(sealed) => tracing::debug!(
            through = folded_through,
            seq = sealed.seq(),
            "digest seal committed"
        ),
        Err(error) => tracing::warn!(
            through = folded_through,
            %error,
            "digest seal append failed (continuing; recovery falls back to full fold)"
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

/// Like [`derive_shaper_children`], but the [`TopologyDecision`] comes from the
/// shaper's **committed `result_ref`** (the model's lowered decision, decoded from
/// the content store) rather than a caller-supplied value. This is the PR-2 (F-4)
/// fact-driven path, selected when a [`topology::TopologyProvider`] produced the
/// topology: the engine trusts the journal fact, so its runnable children are
/// provably the SAME set the `DefaultTopologyMaterializer` registered (both decode
/// one committed payload — a single source of truth; no pre-staged-arg vs fact
/// divergence). Pure over the committed entry, so a fresh recovery re-derives the
/// same children (R49 — replay the model's decision, never re-decide).
///
/// `Ok(false)` while the shaper is uncommitted; `Ok(true)` once children derive;
/// `Err` only if the committed payload is missing or not a decodable
/// `TopologyDecision` — corruption, since the model-proposal decode boundary
/// (`kx_planner::decode_loop_proposal`) ran BEFORE the decision was committed.
fn derive_shaper_children_from_fact<S>(
    workflow: &DemoWorkflow,
    shaper_wm: &WorkflowMote,
    projection: &Projection,
    store: &S,
    runnable: &mut Vec<WorkflowMote>,
    child_ids: &mut Vec<MoteId>,
) -> Result<bool, RuntimeError>
where
    S: ContentStore + ?Sized,
{
    if projection.state_of(&workflow.shaper_id) != MoteState::Committed {
        return Ok(false);
    }
    let Some(shaper_result_ref) = projection.result_ref_of(&workflow.shaper_id) else {
        return Ok(false);
    };
    let payload = store.get(&shaper_result_ref).map_err(|_| {
        RuntimeError::Decode(format!(
            "shaper {:?} committed result_ref {shaper_result_ref:?} has no payload",
            workflow.shaper_id
        ))
    })?;
    let decision = topology::decode_topology_decision(&payload)?;
    let children = topology::derive_child_motes(
        &shaper_wm.mote,
        shaper_result_ref,
        &decision,
        &shaper_wm.warrant,
        &shaper_wm.capability,
    );
    child_ids.extend(children.iter().map(|c| c.mote.id));
    runnable.extend(children);
    Ok(true)
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
        // Skip any terminal Mote — committed, or dead-lettered (`Failed`/
        // `Inconsistent`) by the PR-1 failure policy. On the demo path no Mote is
        // ever `Failed`/`Inconsistent`, so this is byte-identical to skipping only
        // `Committed | Repudiated`.
        if is_terminal(state) {
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
        secret_scope: kx_warrant::SecretScope::None,
    }
}

/// Record the committed action (`result_ref`) of every committed runnable Mote
/// into the off-truth-path capture sink (D67).
///
/// `runnable` is the authoritative full Mote set (materialized shaper children are
/// extended into it — the same set [`outcome`] counts over), so iterating it once
/// is complete. Reads only `result_ref_of` (in-memory projection state) — no
/// content-store read, no journal write. `O(committed·log)`, flat per-Mote.
/// Recording is idempotent (overwrite by `MoteId`). Capture is `ActionsOnly`
/// exhaust; `Full` reasoning/thinking enrichment is the real-model harness's job.
fn capture_committed_actions(
    runnable: &[WorkflowMote],
    projection: &Projection,
    sink: &CaptureSink,
) {
    for w in runnable {
        if let Some(result_ref) = projection.result_ref_of(&w.mote.id) {
            sink.record(StepRecord::action(w.mote.id, result_ref));
        }
    }
}

/// Saturating `usize → u32` for off-truth-path audit counts (avoids a cast lint;
/// audit counts never approach `u32::MAX` in practice).
fn as_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// R4: emit one terminal-state [`AuditEvent`] per runnable Mote off the truth path.
///
/// A single O(committed·log) sweep over the FINAL projection — flat per-Mote and
/// COMPLETE (it reads the same `(result_ref, nd_class)` the digest folds, so the
/// emitted `MoteCommitted` set is exactly the digest's committed set, and it covers
/// Motes committed by recovery before the loop ran). Non-terminal Motes
/// (`Pending`/`Scheduled`) emit nothing. Echoes already-derived projection state —
/// it NEVER recomputes a `MoteId` (SN-8) and NEVER reads payload bytes.
fn audit_terminal_states(
    runnable: &[WorkflowMote],
    projection: &Projection,
    sink: &RuntimeAuditSink,
) {
    for w in runnable {
        let id = w.mote.id;
        if let Some(event) = terminal_event_for(
            id,
            projection.state_of(&id),
            projection.result_ref_of(&id),
            projection.nondeterminism_of(&id),
        ) {
            sink.record(event);
        }
    }
}

/// Pure mapping from a Mote's FINAL projection state to its terminal audit event
/// (R4). Extracted so the per-state mapping — including the rarely-reached
/// `Failed`/`Repudiated`/`Inconsistent` cases — is exhaustively unit-testable
/// without standing up a run. `Committed` echoes the SAME `(result_ref, nd_class)`
/// the digest folds (so the emitted `MoteCommitted` set is exactly the digest's
/// committed set); a `Committed` missing either is skipped (it would not be in the
/// digest either). Non-terminal states (`Pending`/`Scheduled`) emit nothing.
fn terminal_event_for(
    id: MoteId,
    state: MoteState,
    result_ref: Option<ContentRef>,
    nd_class: Option<NdClass>,
) -> Option<AuditEvent> {
    match state {
        MoteState::Committed => match (result_ref, nd_class) {
            (Some(result_ref), Some(nd_class)) => Some(AuditEvent::MoteCommitted {
                mote_id: id,
                result_ref,
                nd_class,
            }),
            _ => None,
        },
        MoteState::Failed => Some(AuditEvent::MoteFailed { mote_id: id }),
        MoteState::Repudiated => Some(AuditEvent::MoteRepudiated { mote_id: id }),
        MoteState::Inconsistent => Some(AuditEvent::MoteInconsistent { mote_id: id }),
        MoteState::Pending | MoteState::Scheduled => None,
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

#[cfg(test)]
mod audit_tests {
    //! R4 audit-seam unit tests over the engine internals: the pure terminal-state
    //! mapping (exhaustive over every `MoteState`) and the terminal sweep driven by
    //! a REAL projection folded into each terminal state (the "producing" test for
    //! the `Failed`/`Repudiated`/`Inconsistent` variants the canonical demo never
    //! reaches on its own).

    use std::sync::Arc;

    use kx_audit::{AuditEvent, DispatchKind, InMemoryAuditSink};
    use kx_content::ContentRef;
    use kx_journal::{FailureReason, JournalEntry, RepudiationReason};
    use kx_mote::{MoteDefHash, MoteId, NdClass};
    use smallvec::SmallVec;

    use super::*;

    fn cref(b: u8) -> ContentRef {
        ContentRef::from_bytes([b; 32])
    }

    // `key` is a distinct per-Mote idempotency byte so different Motes' entries
    // never collide in the dedup index `(idempotency_key, kind)`.
    fn committed(
        mote_id: MoteId,
        seq: u64,
        key: u8,
        nd: NdClass,
        result_ref: ContentRef,
    ) -> JournalEntry {
        JournalEntry::Committed {
            mote_id,
            idempotency_key: [key; 32],
            seq,
            nondeterminism: nd,
            result_ref,
            parents: SmallVec::new(),
            warrant_ref: cref(0xaa),
            mote_def_hash: MoteDefHash::from_bytes([key; 32]),
        }
    }

    fn failed(mote_id: MoteId, seq: u64, key: u8) -> JournalEntry {
        JournalEntry::Failed {
            mote_id,
            idempotency_key: [key; 32],
            seq,
            reason_class: FailureReason::TimedOut,
            reporter_id: 0,
        }
    }

    fn effect_staged(mote_id: MoteId, seq: u64, key: u8) -> JournalEntry {
        JournalEntry::EffectStaged {
            mote_id,
            idempotency_key: [key; 32],
            seq,
        }
    }

    fn repudiated(
        target_mote_id: MoteId,
        target_committed_seq: u64,
        seq: u64,
        key: u8,
    ) -> JournalEntry {
        JournalEntry::Repudiated {
            target_mote_id,
            idempotency_key: [key; 32],
            seq,
            target_committed_seq,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 0,
        }
    }

    #[test]
    fn terminal_event_maps_every_state() {
        let id = MoteId::from_bytes([1u8; 32]);
        let r = cref(2);

        // Committed with both refs → MoteCommitted echoing the digest's tuple.
        assert_eq!(
            terminal_event_for(
                id,
                MoteState::Committed,
                Some(r),
                Some(NdClass::WorldMutating)
            ),
            Some(AuditEvent::MoteCommitted {
                mote_id: id,
                result_ref: r,
                nd_class: NdClass::WorldMutating,
            })
        );
        // Committed missing a ref is NOT emitted (it would not be in the digest either).
        assert_eq!(
            terminal_event_for(id, MoteState::Committed, None, Some(NdClass::Pure)),
            None
        );
        assert_eq!(
            terminal_event_for(id, MoteState::Committed, Some(r), None),
            None
        );
        // The terminal failure / anomaly states each map to their event.
        assert_eq!(
            terminal_event_for(id, MoteState::Failed, None, None),
            Some(AuditEvent::MoteFailed { mote_id: id })
        );
        assert_eq!(
            terminal_event_for(id, MoteState::Repudiated, None, None),
            Some(AuditEvent::MoteRepudiated { mote_id: id })
        );
        assert_eq!(
            terminal_event_for(id, MoteState::Inconsistent, None, None),
            Some(AuditEvent::MoteInconsistent { mote_id: id })
        );
        // Non-terminal states emit nothing.
        assert_eq!(terminal_event_for(id, MoteState::Pending, None, None), None);
        assert_eq!(
            terminal_event_for(id, MoteState::Scheduled, None, None),
            None
        );
    }

    #[test]
    fn sweep_emits_each_terminal_state_from_a_real_projection() {
        // Real demo Motes give real ids; we fold crafted journal entries to drive
        // four of them into Committed / Failed / Repudiated / Inconsistent and leave
        // the rest Pending, then assert the sweep emits exactly the right trail.
        let wf = DemoWorkflow::canonical();
        assert!(
            wf.motes.len() >= 5,
            "demo declares enough motes for the matrix"
        );
        let ids: Vec<MoteId> = wf.motes.iter().map(|w| w.mote.id).collect();

        let mut p = Projection::new();
        // motes[0] → Committed (nd = ReadOnlyNondet, result_ref = cref(9)).
        p.fold(&committed(
            ids[0],
            1,
            0xa0,
            NdClass::ReadOnlyNondet,
            cref(9),
        ))
        .unwrap();
        // motes[1] → Failed.
        p.fold(&failed(ids[1], 2, 0xa1)).unwrap();
        // motes[2] → Committed(seq 3) then Repudiated(target 3).
        p.fold(&committed(ids[2], 3, 0xa2, NdClass::Pure, cref(3)))
            .unwrap();
        p.fold(&repudiated(ids[2], 3, 4, 0xa2)).unwrap();
        // motes[3] → EffectStaged then Repudiated-without-Committed (cell-8 anomaly).
        p.fold(&effect_staged(ids[3], 5, 0xa3)).unwrap();
        p.fold(&repudiated(ids[3], 0, 6, 0xa3)).unwrap();
        // motes[4..] left Pending (never folded).

        assert_eq!(p.state_of(&ids[0]), MoteState::Committed);
        assert_eq!(p.state_of(&ids[1]), MoteState::Failed);
        assert_eq!(p.state_of(&ids[2]), MoteState::Repudiated);
        assert_eq!(p.state_of(&ids[3]), MoteState::Inconsistent);

        let mem = InMemoryAuditSink::new();
        let sink = RuntimeAuditSink::from_arc(Arc::new(mem.clone()));
        audit_terminal_states(&wf.motes, &p, &sink);

        let events = mem.events();
        assert_eq!(
            events,
            vec![
                AuditEvent::MoteCommitted {
                    mote_id: ids[0],
                    result_ref: cref(9),
                    nd_class: NdClass::ReadOnlyNondet,
                },
                AuditEvent::MoteFailed { mote_id: ids[1] },
                AuditEvent::MoteRepudiated { mote_id: ids[2] },
                AuditEvent::MoteInconsistent { mote_id: ids[3] },
            ],
            "the sweep emits one terminal event per non-pending Mote, in runnable order"
        );
    }

    #[test]
    fn action_accessors_echo_the_picked_mote() {
        // The dispatch-kind mapping the MoteDispatched event relies on.
        let wf = DemoWorkflow::canonical();
        let w = wf.motes[0].clone();
        let pure = Action::RunPure(w.clone());
        assert_eq!(pure.dispatch_kind(), DispatchKind::Pure);
        assert_eq!(pure.mote_id(), w.mote.id);
        let critic = Action::RunNativeCritic(w.clone());
        assert_eq!(critic.dispatch_kind(), DispatchKind::Critic);
        let fresh = Action::RunWm {
            wm: w.clone(),
            recover: false,
        };
        assert_eq!(fresh.dispatch_kind(), DispatchKind::WmFresh);
        let recovery = Action::RunWm {
            wm: w.clone(),
            recover: true,
        };
        assert_eq!(recovery.dispatch_kind(), DispatchKind::WmRecovery);
        assert_eq!(recovery.nd_class(), w.mote.nd_class());
    }

    #[test]
    fn as_u32_saturates() {
        assert_eq!(as_u32(0), 0);
        assert_eq!(as_u32(8), 8);
        assert_eq!(as_u32(usize::MAX), u32::MAX);
    }
}

#[cfg(test)]
mod failure_tests {
    //! PR-1 — the bounded-retry + dead-letter drive-loop behavior. Each test
    //! drives a flat 2-PURE-Mote workflow through `run_with_seams` with a
    //! `FailingExecutor` that fails one Mote, asserting the run COMPLETES with the
    //! failing Mote dead-lettered (`Failed`) and its sibling committed — the live
    //! correctness fix (a mid-DAG failure no longer aborts the whole run) — while
    //! the `None` path still aborts exactly as before.
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use kx_content::LocalFsContentStore;
    use kx_executor::{
        LocalResourceManager, MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs,
        StandardCommitProtocol, TestMoteExecutor,
    };
    use kx_journal::SqliteJournal;
    use kx_mote::Mote;
    use kx_projection::{MoteState, Projection};
    use kx_warrant::{ExecutorClass, WarrantSpec};

    use super::{run_with_seams, RunOutcome};
    use crate::broker::DemoBroker;
    use crate::config::{Mode, RuntimeConfig};
    use crate::error::RuntimeError;
    use crate::failure_policy::FailurePolicy;
    use crate::workflow::flat_pure_workflow;

    /// A `MoteExecutor` that fails a chosen Mote and delegates the rest to a real
    /// deterministic stub, counting how many times the chosen Mote is dispatched.
    struct FailingExecutor {
        inner: TestMoteExecutor,
        target: kx_mote::MoteId,
        error: fn() -> MoteExecutorError,
        target_calls: Arc<AtomicUsize>,
    }

    impl MoteExecutor for FailingExecutor {
        fn run(
            &self,
            mote: &Mote,
            warrant: &WarrantSpec,
            env: Option<Rootfs>,
        ) -> Result<MoteExecutionResult, MoteExecutorError> {
            if mote.id == self.target {
                self.target_calls.fetch_add(1, Ordering::SeqCst);
                return Err((self.error)());
            }
            self.inner.run(mote, warrant, env)
        }

        fn supports(&self, class: ExecutorClass) -> bool {
            self.inner.supports(class)
        }
    }

    fn config_for(dir: &std::path::Path) -> RuntimeConfig {
        RuntimeConfig {
            journal_path: dir.join("journal.sqlite"),
            content_root: dir.join("content"),
            mode: Mode::Run,
            crash_at: None,
            checkpoint_every: None,
            audit_log: None,
        }
    }

    /// Drive a flat 2-PURE-Mote workflow whose second Mote fails with `error`,
    /// under `policy`. Returns the run result, the (target, sibling) ids, the
    /// re-folded final projection, and the target's dispatch count.
    fn drive(
        error: fn() -> MoteExecutorError,
        policy: Option<&FailurePolicy>,
    ) -> (
        Result<RunOutcome, RuntimeError>,
        kx_mote::MoteId,
        kx_mote::MoteId,
        Option<Projection>,
        usize,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let config = config_for(dir.path());
        let workflow = flat_pure_workflow(&[0x10, 0x11]);
        let sibling = workflow.motes[0].mote.id;
        let target = workflow.motes[1].mote.id;

        let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
        let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
        let rm = LocalResourceManager::dev_defaults();
        let broker = Arc::new(DemoBroker::new(store.clone(), BTreeMap::new(), None, None));
        let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
        let target_calls = Arc::new(AtomicUsize::new(0));
        let executor = FailingExecutor {
            inner: TestMoteExecutor::deterministic(),
            target,
            error,
            target_calls: target_calls.clone(),
        };

        let result = run_with_seams(
            &config,
            &workflow,
            store,
            journal.clone(),
            &rm,
            &executor,
            &protocol,
            None, // shaper — flat DAG
            None, // topology_provider — no model-driven topology
            None, // snapshot_sink
            None, // capture_sink
            None, // audit_sink
            policy,
        );

        // Re-fold the on-disk journal into a fresh projection so the assertions read
        // the same durable truth a cold recovery would.
        let projection = Projection::from_journal(&*journal).ok();
        (
            result,
            target,
            sibling,
            projection,
            target_calls.load(Ordering::SeqCst),
        )
    }

    #[test]
    fn terminal_failure_dead_letters_and_run_completes() {
        let policy = FailurePolicy::new(3, Duration::ZERO);
        let (result, target, sibling, projection, calls) =
            drive(|| MoteExecutorError::BodyExited { code: 1 }, Some(&policy));

        let outcome = result.expect("the run completes (not aborts) past a dead-lettered Mote");
        assert_eq!(outcome.total, 2);
        assert_eq!(
            outcome.committed, 1,
            "the sibling commits; the target does not"
        );

        let projection = projection.unwrap();
        assert_eq!(
            projection.state_of(&target),
            MoteState::Failed,
            "the failing Mote is dead-lettered (terminal Failed)"
        );
        assert_eq!(
            projection.state_of(&sibling),
            MoteState::Committed,
            "its sibling still commits"
        );
        // Terminal logic failure ⇒ dispatched exactly once, never retried.
        assert_eq!(calls, 1, "a terminal failure is not retried");
    }

    #[test]
    fn transient_failure_retries_to_budget_then_dead_letters() {
        let policy = FailurePolicy::new(3, Duration::ZERO);
        let (result, target, sibling, projection, calls) = drive(
            || MoteExecutorError::SandboxLoadFailed {
                reason: "transient".into(),
            },
            Some(&policy),
        );

        let outcome = result.expect("the run completes after exhausting the retry budget");
        assert_eq!(outcome.committed, 1);

        let projection = projection.unwrap();
        assert_eq!(projection.state_of(&target), MoteState::Failed);
        assert_eq!(projection.state_of(&sibling), MoteState::Committed);
        // A transient error is retried up to `max_attempts` dispatches, then dead-lettered.
        assert_eq!(calls, 3, "transient retried to the 3-attempt budget");
    }

    #[test]
    fn no_policy_aborts_the_run_exactly_as_before() {
        // The legacy path: with no `FailurePolicy`, a dispatch error propagates and
        // aborts the whole run (`RuntimeError::Lifecycle`) — byte-identical to the
        // pre-PR-1 `?`. This is what every existing caller (and the demo) relies on.
        let (result, _target, _sibling, _projection, _calls) =
            drive(|| MoteExecutorError::BodyExited { code: 1 }, None);
        assert!(
            matches!(result, Err(RuntimeError::Lifecycle(_))),
            "without a policy a failing Mote aborts the run"
        );
    }
}
