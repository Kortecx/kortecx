//! [`SchedulerError`] — the scheduler's failure vocabulary.

use kx_mote::MoteId;
use thiserror::Error;

/// Errors surfaced by [`crate::Scheduler::submit`] / [`crate::Scheduler::tick`].
///
/// The scheduler does not interpret per-Mote dispatch outcomes — those are
/// surfaced on [`crate::DispatchedMote::result`] as the executor's typed
/// `MoteExecutorError`. `SchedulerError` covers only failures of the scheduler's
/// own bookkeeping (e.g., duplicate registrations).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    /// `submit` was called twice for the same `MoteId` before the first
    /// instance was dispatched. The submitter should de-duplicate at the
    /// workflow layer; the scheduler's pending map is keyed on `MoteId` and
    /// admits exactly one (mote, warrant) pair per id at a time.
    #[error("mote {0:?} already submitted and not yet dispatched")]
    DuplicateSubmission(MoteId),
}
