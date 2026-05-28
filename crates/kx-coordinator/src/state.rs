//! The orchestration core — a single owner thread holding the run's journal,
//! projection, and hosted scheduler, driven by a command channel.
//!
//! ## Why a thread, not a shared mutex
//!
//! [`kx_projection::Projection`] holds a non-`Send` `Box<dyn TopologyMaterializer>`,
//! so it cannot live inside a `Send + Sync` tonic service. Rather than refactor
//! the P1 `kx-projection` crate (Rule 1 — upstream refactors are their own PR), the
//! coordinator confines the projection (and the journal and scheduler) to one owner
//! thread. The async RPC handlers send [`Command`]s over an `mpsc` channel and await
//! a `oneshot` reply. This also makes the **D40 sole-writer invariant structural**:
//! there is exactly one thread, and it is the only code that ever appends.
//!
//! ## Hosting the scheduler verbatim (thesis test)
//!
//! Registration routes through [`kx_scheduler::Scheduler::submit`] exactly as the
//! single-node `kx-runtime` engine does — the scheduler source is unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentStore, LocalFsContentStore};
use kx_journal::{Journal, JournalEntry};
use kx_mote::{Mote, MoteId, NdClass};
use kx_projection::{MoteState, Projection};
use kx_scheduler::{LocalPlacement, Placement, Scheduler, SchedulerError, WorkerId};
use kx_warrant::{ExecutorClass, WarrantSpec};
use tokio::sync::{mpsc, oneshot};

use crate::commit::CommitProposal;
use crate::error::CoordinatorError;
use crate::placement::LoadAwarePlacement;
use crate::registry::WorkerRegistry;

/// Bound on in-flight commands queued to the orchestration core. A bounded
/// channel applies backpressure: when the core is saturated, `dispatch` awaits
/// instead of letting an unbounded queue grow without limit under a flood of RPCs.
const COMMAND_BUFFER: usize = 1024;

/// Max commands the core drains per wake. Consecutive `Commit`s within a drain
/// coalesce into one journal transaction (group commit); this bounds the size of
/// that transaction.
const MAX_DRAIN: usize = 256;

/// Outcome of a `SubmitMote`: the canonically re-derived id and whether it was a
/// duplicate (idempotent re-submit before commit).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SubmitOutcome {
    pub(crate) mote_id: MoteId,
    pub(crate) duplicate: bool,
}

/// Outcome of a `ReportCommit`: the journal-assigned seq and whether the commit
/// was newly appended or a dedup-by-key hit (first-wins).
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommitApplied {
    pub(crate) committed_seq: u64,
    pub(crate) already_committed: bool,
}

/// Messages the async handlers send to the owner thread.
pub(crate) enum Command {
    Submit {
        mote: Box<Mote>,
        warrant: Box<WarrantSpec>,
        reply: oneshot::Sender<SubmitOutcome>,
    },
    Commit {
        proposal: Box<CommitProposal>,
        reply: oneshot::Sender<Result<CommitApplied, CoordinatorError>>,
    },
    StateOf {
        mote_id: MoteId,
        reply: oneshot::Sender<MoteState>,
    },
    CommittedCount {
        reply: oneshot::Sender<usize>,
    },
    ReadySet {
        reply: oneshot::Sender<Vec<MoteId>>,
    },
    LeaseWork {
        worker: WorkerId,
        executor_class: ExecutorClass,
        max: usize,
        reply: oneshot::Sender<Vec<(Mote, WarrantSpec)>>,
    },
    ReadEntries {
        since_seq: u64,
        max: usize,
        reply: oneshot::Sender<Result<(Vec<JournalEntry>, u64), CoordinatorError>>,
    },
}

/// Handle to the orchestration core. Cloneable + `Send + Sync` (it is just the
/// channel sender), so the gRPC service that holds it is too.
#[derive(Clone)]
pub(crate) struct CoreHandle {
    commands: mpsc::Sender<Command>,
}

impl CoreHandle {
    /// Spawn the owner thread, taking sole ownership of `journal`. When `store` is
    /// `Some`, the core verifies `store.contains(result_ref)` before committing a
    /// proposal (D55 phantom-ref guard); when `None`, commits are not content-checked
    /// (the P2.2/P2.3 behavior). `registry` is the live worker view the lease-time
    /// placement policy ranks over (D56).
    pub(crate) fn spawn<J: Journal + Send + 'static>(
        journal: J,
        store: Option<Arc<LocalFsContentStore>>,
        registry: Arc<dyn WorkerRegistry>,
    ) -> Self {
        let (commands, inbox) = mpsc::channel(COMMAND_BUFFER);
        std::thread::spawn(move || core_loop(&journal, store.as_deref(), &*registry, inbox));
        Self { commands }
    }

    pub(crate) async fn submit(
        &self,
        mote: Mote,
        warrant: WarrantSpec,
    ) -> Result<SubmitOutcome, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::Submit {
            mote: Box::new(mote),
            warrant: Box::new(warrant),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    pub(crate) async fn commit(
        &self,
        proposal: CommitProposal,
    ) -> Result<CommitApplied, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::Commit {
            proposal: Box::new(proposal),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    pub(crate) async fn state_of(&self, mote_id: MoteId) -> Result<MoteState, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::StateOf { mote_id, reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    pub(crate) async fn committed_count(&self) -> Result<usize, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::CommittedCount { reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    pub(crate) async fn ready_set(&self) -> Result<Vec<MoteId>, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ReadySet { reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// Lease up to `max` ready PURE Motes runnable on `executor_class`, returning
    /// each with the warrant it was submitted under (the dispatch surface the worker
    /// pulls). Placement v2 (D56) prefers Motes the load-aware policy routes to
    /// `worker`, then fills to `max` with the rest so a live poller never idles while
    /// ready work exists. No lease/lock is held: double execution is harmless under
    /// the journal's dedupe-by-key, and worker-death reschedule is reserved for P3.
    pub(crate) async fn lease_work(
        &self,
        worker: WorkerId,
        executor_class: ExecutorClass,
        max: usize,
    ) -> Result<Vec<(Mote, WarrantSpec)>, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::LeaseWork {
            worker,
            executor_class,
            max,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// Read up to `max` committed journal entries with seq `> since_seq`, plus the
    /// cursor (`next_seq`) to pass on the next poll. The distributed-read surface
    /// (D55): peers pull committed-entry deltas and fold a local read model rather
    /// than round-tripping per result.
    pub(crate) async fn read_entries(
        &self,
        since_seq: u64,
        max: usize,
    ) -> Result<(Vec<JournalEntry>, u64), CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ReadEntries {
            since_seq,
            max,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    async fn dispatch(&self, command: Command) -> Result<(), CoordinatorError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }
}

/// Recover the read-side from the durable journal at startup: fold the log into a
/// projection, read the current seq watermark, and seed the admission set from the
/// already-committed Motes (on recovery, committed Motes count as admitted). Returns
/// `None` (after logging) on a durable-layer fault, which stops the core — the
/// journal stays the truth and a restart re-folds from it.
fn recover<J: Journal>(journal: &J) -> Option<(Projection, u64, BTreeSet<MoteId>)> {
    let projection = match Projection::from_journal(journal) {
        Ok(projection) => projection,
        Err(error) => {
            tracing::error!(%error, "coordinator core failed to recover the projection");
            return None;
        }
    };
    let folded_through = match journal.current_seq() {
        Ok(seq) => seq,
        Err(error) => {
            tracing::error!(%error, "coordinator core failed to read the journal seq");
            return None;
        }
    };
    let submitted = projection
        .snapshot()
        .iter_motes()
        .map(|(id, _)| id)
        .collect();
    Some((projection, folded_through, submitted))
}

/// Select up to `max` ready PURE Motes for `worker` to run. Candidates are
/// ready (parents-all-committed) ∩ PURE (the only class executed-then-proposed in
/// P2.x; WM is deferred) ∩ matching the worker's backend (`executor_class`). Placement
/// v2 (D56) then orders them: Motes the load-aware policy routes to `worker` come
/// **first**, the rest **fill to `max`** — so work balances across workers by load
/// (shard-by-mote on ties) while a live poller never idles when ready work exists
/// (starvation-free; double execution stays harmless under dedup, D54).
fn lease_ready(
    projection: &Projection,
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
    registry: &dyn WorkerRegistry,
    worker: WorkerId,
    executor_class: ExecutorClass,
    max: usize,
) -> Vec<(Mote, WarrantSpec)> {
    let placement = LoadAwarePlacement::new(registry, executor_class);
    let mut preferred: Vec<MoteId> = Vec::new();
    let mut rest: Vec<MoteId> = Vec::new();
    for mote_id in projection.ready_set() {
        if let Some((mote, warrant)) = submitted_defs.get(&mote_id) {
            if mote.nd_class() == NdClass::Pure && warrant.executor_class == executor_class {
                if placement.place(&mote_id) == worker {
                    preferred.push(mote_id);
                } else {
                    rest.push(mote_id);
                }
            }
        }
    }
    preferred
        .into_iter()
        .chain(rest)
        .take(max)
        .filter_map(|id| submitted_defs.get(&id).cloned())
        .collect()
}

/// Read up to `max` `Committed` entries with seq in `(since_seq, current_seq]`, in
/// seq order, plus the cursor to resume from. The cursor (`next_seq`) advances past
/// everything scanned: it is `current_seq` when the whole range was scanned (the peer
/// is caught up), or the last collected entry's seq when the `max` cap was hit (so the
/// next poll resumes right after it — no entry is skipped or re-scanned). Non-committed
/// entries (e.g. `Proposed`) are skipped but still advance the scan, so a peer that only
/// wants committed results never re-reads them. The whole journal below `current_seq` is
/// immutable + append-only, so this read is consistent without locking the writer.
fn read_committed_since<J: Journal>(
    journal: &J,
    since_seq: u64,
    max: usize,
) -> Result<(Vec<JournalEntry>, u64), CoordinatorError> {
    let current = journal.current_seq()?;
    if since_seq >= current || max == 0 {
        return Ok((Vec::new(), current));
    }
    let mut entries = Vec::new();
    let mut next_seq = current;
    for entry in journal.read_entries_by_seq((since_seq + 1)..(current + 1))? {
        if matches!(entry, JournalEntry::Committed { .. }) {
            if entries.len() == max {
                // Cap hit: stop one short and resume after the last collected entry.
                next_seq = entries.last().map_or(since_seq, JournalEntry::seq);
                break;
            }
            entries.push(entry);
        }
    }
    Ok((entries, next_seq))
}

/// The owner-thread loop. Recovers the projection from the journal, then services
/// commands until every sender drops (the channel closes on coordinator shutdown).
fn core_loop<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    registry: &dyn WorkerRegistry,
    mut inbox: mpsc::Receiver<Command>,
) {
    let Some((mut projection, mut folded_through, mut submitted)) = recover(journal) else {
        return;
    };
    let mut scheduler = Scheduler::new(LocalPlacement);
    // Definitions of admitted-but-not-yet-committed Motes, kept so `LeaseWork` can
    // hand a worker the Mote + warrant to run. (kx-scheduler consumes the Mote on
    // submit and exposes no get-by-id accessor — and is frozen by the thesis test —
    // so the coordinator retains its own copy.) Recovery does not repopulate this:
    // committed Motes are never ready, and pending work lost to a crash is a P3
    // concern.
    let mut submitted_defs: BTreeMap<MoteId, (Mote, WarrantSpec)> = BTreeMap::new();

    while let Some(first) = inbox.blocking_recv() {
        // Drain everything immediately available (up to MAX_DRAIN) so consecutive
        // ReportCommits coalesce into one journal transaction (group commit).
        let mut drained = vec![first];
        while drained.len() < MAX_DRAIN {
            match inbox.try_recv() {
                Ok(command) => drained.push(command),
                Err(_) => break,
            }
        }

        // Process in arrival order, accumulating a run of consecutive `Commit`s and
        // flushing it (as one group commit) whenever a non-`Commit` command is
        // reached — so `Submit`s and reads keep their exact in-order semantics
        // (a read or submit always observes the commits queued before it).
        let mut pending: Vec<PendingCommit> = Vec::new();
        for command in drained {
            // `Commit`s accumulate into the pending run; every other command must
            // observe the commits queued before it, so flush the run first (one
            // group commit), then handle the command.
            let command = match command {
                Command::Commit { proposal, reply } => {
                    pending.push((*proposal, reply));
                    continue;
                }
                other => other,
            };
            flush_commits(
                journal,
                store,
                &mut projection,
                &mut folded_through,
                &submitted,
                &mut submitted_defs,
                &mut pending,
            );
            match command {
                Command::Commit { .. } => unreachable!("Commit is handled above"),
                Command::Submit {
                    mote,
                    warrant,
                    reply,
                } => {
                    let mote_id = mote.id;
                    let mote = *mote;
                    let warrant = *warrant;
                    let duplicate =
                        match scheduler.submit(mote.clone(), warrant.clone(), &mut projection) {
                            Ok(()) => {
                                submitted.insert(mote_id);
                                submitted_defs.insert(mote_id, (mote, warrant));
                                false
                            }
                            Err(SchedulerError::DuplicateSubmission(_)) => true,
                        };
                    let _ = reply.send(SubmitOutcome { mote_id, duplicate });
                }
                Command::StateOf { mote_id, reply } => {
                    let _ = reply.send(projection.state_of(&mote_id));
                }
                Command::CommittedCount { reply } => {
                    let _ = reply.send(projection.snapshot().committed_count());
                }
                Command::ReadySet { reply } => {
                    let _ = reply.send(projection.ready_set());
                }
                Command::LeaseWork {
                    worker,
                    executor_class,
                    max,
                    reply,
                } => {
                    let items = lease_ready(
                        &projection,
                        &submitted_defs,
                        registry,
                        worker,
                        executor_class,
                        max,
                    );
                    let _ = reply.send(items);
                }
                Command::ReadEntries {
                    since_seq,
                    max,
                    reply,
                } => {
                    let _ = reply.send(read_committed_since(journal, since_seq, max));
                }
            }
        }
        flush_commits(
            journal,
            store,
            &mut projection,
            &mut folded_through,
            &submitted,
            &mut submitted_defs,
            &mut pending,
        );
    }
}

/// One queued `ReportCommit`: its validated proposal + the reply channel.
type PendingCommit = (
    CommitProposal,
    oneshot::Sender<Result<CommitApplied, CoordinatorError>>,
);

/// Build the durable `Committed` entry from a validated proposal.
fn committed_entry(proposal: CommitProposal) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: proposal.mote_id,
        idempotency_key: proposal.idempotency_key,
        seq: 0,
        nondeterminism: proposal.nd_class,
        result_ref: proposal.result_ref,
        parents: proposal.parents,
        warrant_ref: proposal.warrant_ref,
        mote_def_hash: proposal.mote_def_hash,
    }
}

/// Flush a run of queued commits as ONE group commit
/// ([`Journal::append_batch`](kx_journal::Journal::append_batch) — a single journal
/// transaction), then fold the new range once. Never-submitted Motes are rejected
/// individually with no write; the admitted ones are appended atomically. When
/// `store` is `Some`, each proposal's `result_ref` must be present in the content
/// store (D55 phantom-ref guard) — an absent ref is rejected individually, never
/// blocking its batch-mates.
fn flush_commits<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    submitted: &BTreeSet<MoteId>,
    submitted_defs: &mut BTreeMap<MoteId, (Mote, WarrantSpec)>,
    pending: &mut Vec<PendingCommit>,
) {
    if pending.is_empty() {
        return;
    }
    let batch = std::mem::take(pending);

    // Per-proposal admission: reject never-submitted Motes and (when a store is
    // configured) results whose bytes were never published — neither writes, so one
    // inadmissible commit never blocks its valid batch-mates.
    let mut entries: Vec<JournalEntry> = Vec::with_capacity(batch.len());
    let mut replies: Vec<oneshot::Sender<Result<CommitApplied, CoordinatorError>>> =
        Vec::with_capacity(batch.len());
    let mut committed_ids: Vec<MoteId> = Vec::with_capacity(batch.len());
    for (proposal, reply) in batch {
        if !submitted.contains(&proposal.mote_id) {
            let _ = reply.send(Err(CoordinatorError::UnknownMote(proposal.mote_id)));
        } else if store.is_some_and(|s| !s.contains(&proposal.result_ref)) {
            let _ = reply.send(Err(CoordinatorError::ResultRefAbsent(proposal.mote_id)));
        } else {
            committed_ids.push(proposal.mote_id);
            entries.push(committed_entry(proposal));
            replies.push(reply);
        }
    }
    if entries.is_empty() {
        return;
    }

    match apply_batch(journal, projection, folded_through, entries) {
        Ok(applied) => {
            // The defs are no longer leasable once committed (a committed Mote is
            // never in the ready-set); free them to keep the map bounded by
            // in-flight work, not total submissions.
            for id in &committed_ids {
                submitted_defs.remove(id);
            }
            for (reply, applied) in replies.into_iter().zip(applied) {
                let _ = reply.send(Ok(applied));
            }
        }
        Err(message) => {
            // The batch is atomic, so on failure nothing was durably written; report
            // the same fault to every waiter (they may retry).
            for reply in replies {
                let _ = reply.send(Err(CoordinatorError::CommitFailed(message.clone())));
            }
        }
    }
}

/// Append a pre-validated, pre-admitted batch in one transaction, fold the newly
/// appended entries **in-hand**, and derive each entry's [`CommitApplied`].
///
/// `append_batch` returns each entry's durable form, so we fold those directly
/// instead of re-reading the new range back from the journal (one fewer query +
/// decode per batch). A commit is **newly committed** iff its returned seq is past
/// the pre-batch watermark AND is the first occurrence of that seq in this batch —
/// a re-report (across batches → older seq; within the batch → an already-seen seq)
/// is `already_committed` and is NOT re-folded (re-folding a `Committed` would trip
/// `DuplicateCommitted`). The newly appended entries arrive in ascending-seq order,
/// so the watermark advances monotonically as we fold. Returns a stringified error
/// (relayable to every waiter) only on a catastrophic journal/projection fault; the
/// batch is atomic, so on error nothing was durably written.
fn apply_batch<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    entries: Vec<JournalEntry>,
) -> Result<Vec<CommitApplied>, String> {
    // The watermark equals the journal's current seq by invariant (everything
    // <= `folded_through` is folded), so use it directly as the pre-batch boundary
    // — no extra `current_seq()` query.
    let seq_before = *folded_through;
    let durable = journal.append_batch(entries).map_err(|e| e.to_string())?;

    let mut new_seqs = BTreeSet::new();
    let mut applied = Vec::with_capacity(durable.len());
    for entry in &durable {
        let seq = entry.seq();
        let is_new = seq > seq_before && new_seqs.insert(seq);
        if is_new {
            projection.fold(entry).map_err(|e| e.to_string())?;
            *folded_through = seq; // ascending-seq → monotonic advance
        }
        applied.push(CommitApplied {
            committed_seq: seq,
            already_committed: !is_new,
        });
    }
    Ok(applied)
}
