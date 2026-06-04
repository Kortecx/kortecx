//! Submit a [`BoundRun`] to the durable spine via the gateway propose-proxy.
//! kx-invoke executes by SUBMITTING (register a run, submit each Mote) — never by
//! writing the journal directly. The worker/executor drives `StageThenCommit` ->
//! `Committed`; the caller awaits the terminal result via the gateway
//! (`GetProjection`) or the coordinator (`ReadEntries`).

use kx_gateway_core::RunSubmitter;
use kx_mote::MoteId;

use crate::bind::BoundRun;
use crate::error::InvokeError;

/// A submitted run: the journaled `instance_id` and the terminal Mote whose
/// committed result is the invocation's output.
#[derive(Debug, Clone, Copy)]
pub struct Submitted {
    /// The coordinator-assigned, journaled run identity.
    pub instance_id: [u8; 16],
    /// The terminal (sink) Mote to await for the output `result_ref`.
    pub terminal_mote_id: MoteId,
}

/// Register the run and submit every bound Mote through `submitter`. Registers
/// FIRST (returns only after the journaled `instance_id`), then submits each Mote
/// under its narrowed warrant. Re-invoking with identical bound Motes is
/// idempotent (same identities → the coordinator dedups by key).
///
/// # Errors
/// [`InvokeError::Submit`] if registration or any submit is refused / unreachable.
pub async fn execute<S>(submitter: &S, bound: &BoundRun) -> Result<Submitted, InvokeError>
where
    S: RunSubmitter + ?Sized,
{
    let instance_id = submitter.register_run(bound.recipe_fingerprint).await?;
    for (mote, warrant) in &bound.motes {
        submitter
            .submit_mote(mote.clone(), warrant.clone(), false)
            .await?;
    }
    Ok(Submitted {
        instance_id,
        terminal_mote_id: bound.terminal_mote_id,
    })
}
