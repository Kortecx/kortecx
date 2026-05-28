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

    while let Some(command) = inbox.blocking_recv() {
        match command {
            Command::Submit {
                mote,
                warrant,
                reply,
            } => {
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
            Command::Commit { proposal, reply } => {
                let result = apply_commit(
                    journal,
                    &mut projection,
                    &mut folded_through,
                    &submitted,
                    *proposal,
                );
                let _ = reply.send(result);
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
        }
    }
}

/// Assemble + append the `Committed` entry (the sole-writer step), then fold it
/// into the projection. Refuses commits for never-submitted Motes.
///
/// Dedup is delegated to the journal's own dedup-by-key contract (the single
/// source of truth): `append` is a no-op that returns the pre-existing entry on a
/// duplicate. We detect that — without a second lookup — by comparing the returned
/// seq against the seq captured before the append: a fresh write advances the seq,
/// a dedup hit returns an older one. On a dedup hit we skip the fold (the entry is
/// already folded; re-folding would trip `DuplicateCommitted`).
fn apply_commit<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    submitted: &BTreeSet<MoteId>,
    proposal: CommitProposal,
) -> Result<CommitApplied, CoordinatorError> {
    if !submitted.contains(&proposal.mote_id) {
        return Err(CoordinatorError::UnknownMote(proposal.mote_id));
    }

    let seq_before = journal.current_seq()?;
    let durable = journal.append(JournalEntry::Committed {
        mote_id: proposal.mote_id,
        idempotency_key: proposal.idempotency_key,
        seq: 0,
        nondeterminism: proposal.nd_class,
        result_ref: proposal.result_ref,
        parents: proposal.parents,
        warrant_ref: proposal.warrant_ref,
        mote_def_hash: proposal.mote_def_hash,
    })?;
    // `append` of a `Committed` always returns a `Committed`; the other arm is
    // unreachable by construction.
    let committed_seq = match durable {
        JournalEntry::Committed { seq, .. } => seq,
        _ => 0,
    };

    let already_committed = committed_seq <= seq_before;
    if !already_committed {
        fold_new(journal, projection, folded_through)?;
    }
    Ok(CommitApplied {
        committed_seq,
        already_committed,
    })
}

/// Fold journal entries appended since `folded_through` into `projection`,
/// advancing the watermark. Incremental (bounded range), mirroring the single-node
/// engine's fold so re-scans never go O(n²).
fn fold_new<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
) -> Result<(), CoordinatorError> {
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
