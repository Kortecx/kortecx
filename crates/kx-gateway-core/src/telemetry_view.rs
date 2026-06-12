//! The mote execution-telemetry read seam (Batch C `ListMoteTelemetry`).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`[u8; N]` / `String`
//! / `u64`) — no host type crosses the seam (the [`crate::CaptureView`]
//! pattern). The host (`kx-gateway`) records execution exhaust into a durable
//! `telemetry.db` sidecar and implements [`TelemetryView`] over it.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** Telemetry is execution EXHAUST the host measures
//!   as motes run: wall-clock, model usage, the fired tool. It is never
//!   journaled, never a `MoteId` input, never gating execution, never a digest
//!   input. Unlike capture (journal-derived, refoldable), exec metrics are NOT
//!   journal-derivable — the sidecar is **rebuildable to EMPTY** (the
//!   uploads.db posture): dropping it loses observability, not truth.
//! - **Honest degradation.** `model_id`/`output_tokens` are populated only for
//!   model motes on an inference build; `input_tokens` is NEVER set in OSS
//!   (the frozen backend seam reports no input count). Absent is absent — the
//!   row never claims a model ran on an echo path.
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the sidecar degrades
//!   forward-compatibly.

/// One mote-execution telemetry row in a [`TelemetryView::list`] page — the
/// host-measured exhaust of a single executed Mote, joined (by the background
/// fold) to its `Committed` fact's `seq` + watermark-attributed `instance_id`.
#[derive(Clone, Debug)]
pub struct MoteTelemetryEntry {
    /// The executed Mote's identity.
    pub mote_id: [u8; 32],
    /// Watermark run attribution (may be all-zero/empty before registration).
    pub instance_id: [u8; 16],
    /// Host-measured execution wall time (the executor wrapper's clock).
    pub wall_clock_ms: u64,
    /// NEVER set in OSS — the frozen backend seam reports no input count.
    pub input_tokens: Option<u64>,
    /// Output tokens, model motes on an inference build only.
    pub output_tokens: Option<u64>,
    /// The model that ACTUALLY ran (empty for non-model motes / FFI-free).
    pub model_id: String,
    /// The pinned tool of a tool-bearing mote (else empty).
    pub tool_id: String,
    /// Audit-only start wall clock (ms since epoch; off every hash).
    pub started_unix_ms: u64,
    /// The `Committed` fact's journal seq (ordering / pagination cursor).
    pub seq: u64,
}

/// The telemetry read seam behind `ListMoteTelemetry`. The host implements it
/// over its durable `telemetry.db` sidecar (rebuildable-to-empty execution
/// exhaust). A `None` seam on the service ⇒ the RPC returns `unimplemented`.
pub trait TelemetryView: Send + Sync {
    /// One newest-first page of telemetry rows, optionally scoped to one run
    /// (`instance_id`), one mote (`mote_id`), and/or rows strictly below
    /// `before_seq` (the pagination cursor). `limit` is pre-clamped by the
    /// service. Returns `(rows, has_more)`.
    ///
    /// # Errors
    /// A host read failure ([`crate::error::GatewayError`]).
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        mote_id: Option<[u8; 32]>,
        before_seq: Option<u64>,
    ) -> Result<(Vec<MoteTelemetryEntry>, bool), crate::error::GatewayError>;
}
