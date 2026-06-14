//! The user-feedback store seam (PR-4.1 `SubmitFeedback` / `ListFeedback`).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`[u8; N]` / `String`
//! / `u64`) — no host type crosses the seam (the [`crate::UploadsLedger`] +
//! [`crate::TelemetryView`] precedent). The host (`kx-gateway`) records 👍/👎
//! feedback into a durable `feedback.db` sidecar and implements
//! [`FeedbackStore`] over it.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** Feedback is CLIENT-ORIGIN product signal: a rating
//!   (`UP`/`DOWN`), an optional note, and advisory target/context keys. It is
//!   never journaled, never a `MoteId` input, never gating execution, never a
//!   digest input. Like uploads (and unlike capture, which is journal-derived),
//!   feedback is NOT derivable from anything — the sidecar is **rebuildable to
//!   EMPTY**: dropping it loses product signal, never truth.
//! - **Advisory only.** `instance_id`/`mote_id`/`content_ref`/`recipe_handle`/
//!   `model_id` are display/join/audit fields; identity is the server-derived
//!   `feedback_id` alone (SN-8). The caller `principal` is SERVER-resolved (the
//!   `PutContent` precedent), never trusted off the wire.
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the sidecar degrades
//!   forward-compatibly.

use crate::error::GatewayError;

/// One feedback write — the advisory row the host durably records. The
/// `feedback_id` + `principal` are SERVER-derived by the handler before this
/// reaches the seam (SN-8); the rest are advisory target/context fields.
#[derive(Clone, Debug)]
pub struct FeedbackRecord {
    /// SERVER-derived id (deterministic over `(message_id, principal)` so a
    /// re-rating of the same answer OVERWRITES — the "changed my mind" UX). The
    /// ONLY identity-bearing field; the client cannot name it.
    pub feedback_id: [u8; 16],
    /// The proto rating int (`1 = UP`, `2 = DOWN`; the handler rejects `0`).
    pub rating: i32,
    /// The client-local chat message id this rates (the stable per-answer key).
    pub message_id: String,
    /// The run backing the answer (all-zero when the turn had no run).
    pub instance_id: [u8; 16],
    /// The terminal mote (all-zero when absent; advisory join).
    pub mote_id: [u8; 32],
    /// The answer's content ref (all-zero when absent; advisory join/audit).
    pub content_ref: [u8; 32],
    /// Optional free note (handler-capped fail-closed before this is built).
    pub comment: String,
    /// Advisory: the backing blueprint handle.
    pub recipe_handle: String,
    /// Advisory: the model that answered.
    pub model_id: String,
    /// The SERVER-RESOLVED caller party (from the auth interceptor — never the
    /// wire request). Audit only.
    pub principal: String,
    /// Wall-clock submit time in unix ms (audit only — never identity).
    pub submitted_unix_ms: u64,
}

/// One feedback row in a [`FeedbackStore::list`] page (the read projection of a
/// [`FeedbackRecord`], minus the audit `principal`, plus the sqlite `rowid`
/// pagination cursor).
#[derive(Clone, Debug)]
pub struct FeedbackEntry {
    /// The server-derived id.
    pub feedback_id: [u8; 16],
    /// The proto rating int (`1 = UP`, `2 = DOWN`).
    pub rating: i32,
    /// The rated chat message id.
    pub message_id: String,
    /// The backing run (all-zero when the turn had no run).
    pub instance_id: [u8; 16],
    /// The terminal mote (all-zero when absent).
    pub mote_id: [u8; 32],
    /// The answer's content ref (all-zero when absent).
    pub content_ref: [u8; 32],
    /// The optional note.
    pub comment: String,
    /// Advisory: the backing blueprint handle.
    pub recipe_handle: String,
    /// Advisory: the model that answered.
    pub model_id: String,
    /// Audit-only submit wall clock (ms since epoch; off every hash).
    pub submitted_unix_ms: u64,
    /// The sqlite rowid (ordering / pagination cursor; never identity).
    pub rowid: u64,
}

/// The feedback store seam behind `SubmitFeedback` + `ListFeedback`. The host
/// implements it over a durable, rebuildable-to-empty `feedback.db` sidecar. A
/// `None` seam on the service ⇒ both RPCs return `unimplemented`.
pub trait FeedbackStore: Send + Sync {
    /// Durably record one feedback row (idempotent on `feedback_id` — a re-rating
    /// of the same answer overwrites).
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn record(&self, rec: FeedbackRecord) -> Result<(), GatewayError>;

    /// One newest-first page of feedback rows, optionally scoped to one run
    /// (`instance_id`) and/or rows strictly below `before_rowid` (the pagination
    /// cursor). `limit` is pre-clamped by the service. Returns `(rows, has_more)`.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError`]).
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        before_rowid: Option<u64>,
    ) -> Result<(Vec<FeedbackEntry>, bool), GatewayError>;
}
