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

use std::collections::BTreeSet;

use kx_journal::{Journal, JournalEntry};
use kx_mote::{Mote, MoteId};
use kx_projection::{MoteState, Projection};
use kx_scheduler::{LocalPlacement, Scheduler, SchedulerError};
use kx_warrant::WarrantSpec;
use tokio::sync::{mpsc, oneshot};

use crate::commit::CommitProposal;
use crate::error::CoordinatorError;

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
}

/// Handle to the orchestration core. Cloneable + `Send + Sync` (it is just the
/// channel sender), so the gRPC service that holds it is too.
#[derive(Clone)]
pub(crate) struct CoreHandle {
    commands: mpsc::Sender<Command>,
}

impl CoreHandle {
    /// Spawn the owner thread, taking sole ownership of `journal`.
    pub(crate) fn spawn<J: Journal + Send + 'static>(journal: J) -> Self {
        let (commands, inbox) = mpsc::channel(COMMAND_BUFFER);
        std::thread::spawn(move || core_loop(&journal, inbox));
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

    async fn dispatch(&self, command: Command) -> Result<(), CoordinatorError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }
}

/// The owner-thread loop. Recovers the projection from the journal, then services
/// commands until every sender drops (the channel closes on coordinator shutdown).
fn core_loop<J: Journal>(journal: &J, mut inbox: mpsc::Receiver<Command>) {
    let mut projection = match Projection::from_journal(journal) {
        Ok(projection) => projection,
        Err(error) => {
            tracing::error!(%error, "coordinator core failed to recover the projection");
            return;
        }
    };
    let mut folded_through = match journal.current_seq() {
        Ok(seq) => seq,
        Err(error) => {
            tracing::error!(%error, "coordinator core failed to read the journal seq");
            return;
        }
    };
    let mut scheduler = Scheduler::new(LocalPlacement);
    // Motes this coordinator has admitted (submitted). Seeds the `ReportCommit`
    // admission guard; on recovery, already-committed Motes count as admitted.
    let mut submitted: BTreeSet<MoteId> = projection
        .snapshot()
        .iter_motes()
        .map(|(id, _)| id)
        .collect();

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
            match command {
                Command::Commit { proposal, reply } => {
                    pending.push((*proposal, reply));
                }
                Command::Submit {
                    mote,
                    warrant,
                    reply,
                } => {
                    flush_commits(
                        journal,
                        &mut projection,
                        &mut folded_through,
                        &submitted,
                        &mut pending,
                    );
                    let mote_id = mote.id;
                    let duplicate = match scheduler.submit(*mote, *warrant, &mut projection) {
                        Ok(()) => {
                            submitted.insert(mote_id);
                            false
                        }
                        Err(SchedulerError::DuplicateSubmission(_)) => true,
                    };
                    let _ = reply.send(SubmitOutcome { mote_id, duplicate });
                }
                Command::StateOf { mote_id, reply } => {
                    flush_commits(
                        journal,
                        &mut projection,
                        &mut folded_through,
                        &submitted,
                        &mut pending,
                    );
                    let _ = reply.send(projection.state_of(&mote_id));
                }
                Command::CommittedCount { reply } => {
                    flush_commits(
                        journal,
                        &mut projection,
                        &mut folded_through,
                        &submitted,
                        &mut pending,
                    );
                    let _ = reply.send(projection.snapshot().committed_count());
                }
                Command::ReadySet { reply } => {
                    flush_commits(
                        journal,
                        &mut projection,
                        &mut folded_through,
                        &submitted,
                        &mut pending,
                    );
                    let _ = reply.send(projection.ready_set());
                }
            }
        }
        flush_commits(
            journal,
            &mut projection,
            &mut folded_through,
            &submitted,
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
/// individually with no write; the admitted ones are appended atomically.
fn flush_commits<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    submitted: &BTreeSet<MoteId>,
    pending: &mut Vec<PendingCommit>,
) {
    if pending.is_empty() {
        return;
    }
    let batch = std::mem::take(pending);

    // Admission guard per proposal: reject never-submitted Motes (no write) so one
    // inadmissible commit never blocks its valid batch-mates.
    let mut entries: Vec<JournalEntry> = Vec::with_capacity(batch.len());
    let mut replies: Vec<oneshot::Sender<Result<CommitApplied, CoordinatorError>>> =
        Vec::with_capacity(batch.len());
    for (proposal, reply) in batch {
        if submitted.contains(&proposal.mote_id) {
            entries.push(committed_entry(proposal));
            replies.push(reply);
        } else {
            let _ = reply.send(Err(CoordinatorError::UnknownMote(proposal.mote_id)));
        }
    }
    if entries.is_empty() {
        return;
    }

    match apply_batch(journal, projection, folded_through, entries) {
        Ok(applied) => {
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
