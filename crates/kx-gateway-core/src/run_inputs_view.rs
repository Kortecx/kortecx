//! The run-inputs store seam (PR-D `GetRunInputs` — the "Re-run with changes"
//! capture). The host (`kx-gateway`) durably captures the `Invoke` args into a
//! `run_inputs.db` sidecar keyed by `instance_id` and implements
//! [`RunInputsStore`] over it; the gateway reads them back so a run recovered
//! from `ListRuns` in a fresh session (with no client-side localStorage) can be
//! re-invoked with edited params.
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`[u8; N]` / `String` /
//! `Vec<u8>` / `u64`) — no host type crosses the seam (the [`crate::UploadsLedger`]
//! + [`crate::FeedbackStore`] precedent).
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The captured `args` are the opaque JSON bytes the
//!   client submitted to `Invoke`. They are never journaled, never a `MoteId`
//!   input, never gating execution, never a digest input. Like uploads/feedback
//!   (and unlike capture, which is journal-derived), they are NOT derivable from
//!   anything — the sidecar is **rebuildable to EMPTY**: dropping it loses only
//!   the re-run pre-fill convenience, never truth. A run still serves via
//!   `ListRuns`/`GetProjection`; only the pre-filled form degrades to blank.
//!   Because the args never become committed facts, the canonical projection
//!   digest is invariant by construction (the coordinator stays the sole writer).
//! - **Identity is unchanged.** A *changed* arg flows through the recipe binder's
//!   `config_subset` into a NEW, honest `MoteId` (a new answer / `terminal_mote_id`);
//!   an *unchanged* arg yields the same `MoteId` (the kernel's exact-equality
//!   dedup). This seam only captures/returns the args — it never touches identity.
//! - **One-run-per-journal.** `kx serve` shares one journal, so all invokes share
//!   one `instance_id` and `record` is `INSERT OR REPLACE` (latest invoke's args
//!   win per run). Per-answer history (terminal_mote_id keying) is the explicitly
//!   post-RC conversation-spanning work.
//! - **Advisory / audit only.** The caller `principal` is SERVER-resolved (the
//!   `PutContent`/`SubmitFeedback` precedent), never trusted off the wire, and is
//!   stored for audit only — there is NO read-time party filter in gateway-core
//!   (single-tenant; cross-tenant enforcement is the `kx-cloud/gateway-auth`
//!   SN-8 wall above, exactly as for `feedback.db`/`uploads.db`).
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the sidecar degrades
//!   forward-compatibly (old client / old binary).

use crate::error::GatewayError;

/// One run-inputs capture — the row the host durably records at `Invoke`. The
/// `principal` is SERVER-resolved by the handler before this reaches the seam
/// (SN-8); `handle`/`args` are echoed back verbatim so the caller can re-render
/// the recipe form and re-invoke.
#[derive(Clone, Debug)]
pub struct RunInputsRecord {
    /// The run the args were submitted under (the `ListRuns` recovery key).
    pub instance_id: [u8; 16],
    /// The recipe the run registered under (advisory display/join).
    pub recipe_fingerprint: [u8; 32],
    /// The `Invoke` handle (so `GetRecipeForm` can re-render the form — a durable
    /// run otherwise carries only the fingerprint, not the handle).
    pub handle: String,
    /// The opaque JSON object bytes the client submitted to `Invoke`. Off-digest,
    /// off-identity — captured verbatim, never re-parsed by the gateway.
    pub args: Vec<u8>,
    /// The SERVER-RESOLVED caller party (from the auth interceptor — never the
    /// wire request). Audit only; never a read filter.
    pub principal: String,
    /// Wall-clock capture time in unix ms (audit only — never identity).
    pub captured_unix_ms: u64,
}

/// The read projection of a captured run's inputs (a [`RunInputsRecord`] minus
/// the audit `principal`/`captured_unix_ms`).
#[derive(Clone, Debug)]
pub struct RunInputsEntry {
    /// The run the args were submitted under.
    pub instance_id: [u8; 16],
    /// The recipe the run registered under.
    pub recipe_fingerprint: [u8; 32],
    /// The `Invoke` handle to re-render via `GetRecipeForm`.
    pub handle: String,
    /// The captured opaque JSON object bytes (the original `Invoke` args).
    pub args: Vec<u8>,
}

/// The run-inputs store seam behind `GetRunInputs` + the `Invoke` capture. The
/// host implements it over a durable, rebuildable-to-empty `run_inputs.db`
/// sidecar. A `None` seam on the service ⇒ `GetRunInputs` returns `unimplemented`
/// and the `Invoke` capture is skipped.
pub trait RunInputsStore: Send + Sync {
    /// Durably capture one run's `Invoke` args (idempotent on `instance_id` —
    /// `INSERT OR REPLACE`, latest invoke's args win). Best-effort at the call
    /// site: a failure here must NEVER fail the `Invoke` (the args are
    /// convenience capture, not part of run admission).
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn record(&self, rec: RunInputsRecord) -> Result<(), GatewayError>;

    /// The captured inputs for one run, or `None` if nothing was captured (a
    /// pre-PR-D run, an old binary, or a rebuilt-to-empty sidecar).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError`]).
    fn get(&self, instance_id: &[u8; 16]) -> Result<Option<RunInputsEntry>, GatewayError>;
}
