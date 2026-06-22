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

/// Run a coordinator-materialized ReAct TURN (PR-2d-2) — a ROND,
/// `IdempotentByConstruction`, prompt-carrying model Mote (the identity-bearing
/// `REACT_TURN_KEY` marker, empty `tool_contract`) — DIRECTLY through the
/// hosted executor and return its `result_ref` to PROPOSE via `ReportCommit`.
///
/// A turn fits NEITHER existing worker arm: it is not PURE (the frozen
/// `run_pure_mote` enforces the class), and the broker arm (`run_wm`) resolves a
/// capability from `tool_contract` — a turn deliberately declares none (it
/// PROPOSES; the separate observation Mote fires). In the HARNESS the model
/// lives behind the broker (`ModelBroker` runs prompt-carrying ROND Motes); in
/// serve it lives behind the EXECUTOR (`ModelRouterExecutor`, whose react arm
/// decodes + fences the output pre-commit), so the distributed mirror is a
/// direct `executor.run`. Dispatch semantics match `run_wm`'s
/// `IdempotentByConstruction` arm: fire directly (no `EffectStaged` — a greedy
/// decode is serve-once via the coordinator's first-wins commit dedup, R49) and
/// propose the staged `result_ref`. Warrant ceilings are enforced INSIDE the
/// model dispatch (`inference_params_from_mote` refuses a widening, D35).
///
/// **Also serves the T-AGENT2 LLM-JUDGE critic** (any ReadOnlyNondet model Mote
/// dispatched directly through the executor, never the broker): the executor's
/// `run_judge` arm grades the producer + commits a `CriticVerdict`. The function
/// is intentionally generic — it is `executor.run` plus the result-ref extraction.
pub(crate) fn run_react_turn<E>(
    mote: &Mote,
    warrant: &WarrantSpec,
    executor: &E,
) -> Result<ContentRef, WorkerError>
where
    E: MoteExecutor + ?Sized,
{
    // A react turn never carries an environment_ref (minimal-base sandbox).
    let result = executor
        .run(mote, warrant, None)
        .map_err(kx_executor::LifecycleError::from)?;
    Ok(result.result_ref)
}
