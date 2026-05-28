//! Running a leased PURE Mote through the hosted executor.

use kx_content::ContentRef;
use kx_executor::{run_pure_mote, MoteExecutor, ResourceManager};
use kx_journal::InMemoryJournal;
use kx_mote::Mote;
use kx_warrant::WarrantSpec;

use crate::error::WorkerError;

/// Run a PURE Mote via [`run_pure_mote`] (kx-executor, verbatim) and return its
/// `result_ref`.
///
/// The executor's commit protocol appends a `Proposed` + `Committed` pair to the
/// `Journal` it is given; the worker hands it a **throwaway** [`InMemoryJournal`]
/// so it never touches the coordinator's durable journal (D40 sole-writer). The
/// local seq is meaningless and discarded — only the body's `result_ref` is real
/// (and is what the worker PROPOSES via `ReportCommit`).
pub(crate) fn run_pure<E, R>(
    mote: &Mote,
    warrant: &WarrantSpec,
    executor: &E,
    resource_manager: &R,
) -> Result<ContentRef, WorkerError>
where
    E: MoteExecutor + ?Sized,
    R: ResourceManager + ?Sized,
{
    let scratch = InMemoryJournal::new();
    let commit = run_pure_mote(mote, warrant, &scratch, resource_manager, executor)?;
    Ok(commit.result_ref)
}
