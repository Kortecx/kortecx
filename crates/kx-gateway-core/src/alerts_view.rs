//! The alerts-inbox read seam (W1a-2 `ListAlerts`).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`[u8; N]` / `String`
//! / `u64`) — no host type crosses the seam (the [`crate::TelemetryView`] /
//! [`crate::CaptureView`] pattern). The host (`kx-gateway`) folds the read-only
//! journal's TERMINAL `Failed` facts into a durable `alerts.db` read-cache and
//! implements [`AlertView`] over it.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path, derived-read only.** An alert is a projection of a
//!   committed terminal `Failed` journal fact — never journaled by this seam,
//!   never a `MoteId` input, never gating execution, never a digest input. Like
//!   capture (journal-derived, refoldable) the sidecar is **rebuildable**:
//!   deleting `alerts.db` and re-folding re-materializes the SAME item set
//!   (`alert_id` is server-derived + re-fold-stable), so it cannot perturb the
//!   canonical projection digest.
//! - **Terminal `Failed` only.** The host filter is `!is_pre_commit_crash` —
//!   dead-letters (F4) + worker-reported terminal failures. Liveness
//!   `TimedOut`/`WorkerCrashed` retries are EXCLUDED (they re-dispatch, not
//!   alert). Serve-path admission refusals write NOTHING to the journal (they
//!   are synchronous `SUBMIT_STATUS_REJECTED` responses), so they are not in
//!   this inbox.
//! - **OSS = the read-only VIEW.** The triage LIFECYCLE (acknowledge / resolve),
//!   the alert-rule engine, and outbound notifications are a CLOUD capability
//!   (D156 / D129) — this seam carries no mutate method (GR19).
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the sidecar degrades
//!   forward-compatibly.

/// One alert in an [`AlertView::list`] page — a host projection of a single
/// committed terminal `Failed` journal fact.
#[derive(Clone, Debug)]
pub struct AlertEntry {
    /// SERVER-derived, re-fold-stable id (blake3 over `(mote_id, seq)`; SN-8 —
    /// the client can neither name nor forge it, no existence oracle).
    pub alert_id: [u8; 16],
    /// The failed Mote's identity (the deep-link target).
    pub mote_id: [u8; 32],
    /// Watermark run attribution (may be all-zero before registration folds).
    pub instance_id: [u8; 16],
    /// The terminal `FailureReason` wire token (e.g. `"dead_lettered"`).
    pub reason_class: String,
    /// The numeric `FailureReason` discriminant (0-8) — lets a UI reuse its
    /// single failure-label map instead of a parallel token→label table.
    pub reason_code: u32,
    /// Closed display vocabulary: `"error"` | `"refused"`.
    pub severity: String,
    /// The `Failed` fact's journal seq (deep-link cursor + pagination).
    pub seq: u64,
    /// AUDIT-ONLY first-folded wall clock (ms since epoch; off every hash; may
    /// differ after a rebuild — the item identity is `alert_id`, not the time).
    pub created_unix_ms: u64,
}

/// The alerts read seam behind `ListAlerts`. The host implements it over its
/// durable `alerts.db` sidecar (rebuildable read-cache folded from terminal
/// `Failed` facts). A `None` seam on the service ⇒ the RPC returns
/// `unimplemented`. Read-only by construction — no acknowledge/resolve method
/// (that triage lifecycle is a Cloud capability, D156).
pub trait AlertView: Send + Sync {
    /// One newest-first page of alerts, optionally scoped to one run
    /// (`instance_id`) and/or rows strictly below `before_seq` (the pagination
    /// cursor). `limit` is pre-clamped by the service. Returns `(rows, has_more)`.
    ///
    /// # Errors
    /// A host read failure ([`crate::error::GatewayError`]).
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        before_seq: Option<u64>,
    ) -> Result<(Vec<AlertEntry>, bool), crate::error::GatewayError>;
}
