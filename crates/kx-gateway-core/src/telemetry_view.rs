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

/// One model's token-economy rollup row in a [`TelemetryView::summarize`]
/// result — the EXACT, cross-page aggregate of every committed mote that ran
/// `model_id` in scope. Token-only (no cost/$ — billing is CLOUD, D129/GR19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelTokenRollup {
    /// The model that ACTUALLY ran (never empty — non-model motes are excluded
    /// from per-model rows but still counted in [`TelemetrySummary::total_motes`]).
    pub model_id: String,
    /// Committed model motes that ran this model in scope.
    pub count: u64,
    /// `SUM(output_tokens)` for this model (0 on an FFI-free serve — honest).
    pub total_output_tokens: u64,
    /// `SUM(wall_clock_ms)` for this model.
    pub total_wall_clock_ms: u64,
}

/// The exact, cross-page token-economy rollup behind `ListTelemetrySummary` —
/// folds the WHOLE `telemetry.db` scope server-side (never a page window), so a
/// long ReAct run is summed honestly. `rows` is one entry per model (descending
/// `total_output_tokens`); the two scalars are the window-wide honest totals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetrySummary {
    /// Per-model rollups, descending `total_output_tokens` (ties: `model_id`).
    pub rows: Vec<ModelTokenRollup>,
    /// ALL joined motes in scope (model + non-model).
    pub total_motes: u64,
    /// Window-wide `SUM(output_tokens)`.
    pub total_output_tokens: u64,
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

    /// The exact, cross-page per-model token rollup for one run (`instance_id`)
    /// or all runs (`None`). The default folds [`Self::list`] page-by-page (so
    /// every existing impl works unchanged); a host with a real sidecar SHOULD
    /// override it with a single `SUM ... GROUP BY model_id` query. Per-model
    /// rows exclude non-model motes (empty `model_id`) but they still count
    /// toward [`TelemetrySummary::total_motes`].
    ///
    /// # Errors
    /// A host read failure ([`crate::error::GatewayError`]).
    fn summarize(
        &self,
        instance_id: Option<[u8; 16]>,
    ) -> Result<TelemetrySummary, crate::error::GatewayError> {
        use std::collections::BTreeMap;
        // (count, out_tokens, wall_ms) keyed by model_id — BTreeMap keeps the
        // pre-sort deterministic before the descending-tokens ordering.
        let mut by_model: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
        let mut total_motes: u64 = 0;
        let mut total_output_tokens: u64 = 0;
        let mut before: Option<u64> = None;
        loop {
            let (page, has_more) = self.list(SUMMARY_PAGE, instance_id, None, before)?;
            if page.is_empty() {
                break;
            }
            for row in &page {
                total_motes += 1;
                let out = row.output_tokens.unwrap_or(0);
                total_output_tokens += out;
                before = Some(row.seq);
                if row.model_id.is_empty() {
                    continue;
                }
                let slot = by_model.entry(row.model_id.clone()).or_default();
                slot.0 += 1;
                slot.1 += out;
                slot.2 += row.wall_clock_ms;
            }
            if !has_more {
                break;
            }
        }
        let mut rows: Vec<ModelTokenRollup> = by_model
            .into_iter()
            .map(|(model_id, (count, out, wall))| ModelTokenRollup {
                model_id,
                count,
                total_output_tokens: out,
                total_wall_clock_ms: wall,
            })
            .collect();
        // Descending output tokens; ties break by model_id (ascending) for a
        // deterministic order.
        rows.sort_by(|a, b| {
            b.total_output_tokens
                .cmp(&a.total_output_tokens)
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
        Ok(TelemetrySummary {
            rows,
            total_motes,
            total_output_tokens,
        })
    }
}

/// Page size for the default [`TelemetryView::summarize`] fold (the service's
/// max page clamp). A real host override ignores this (single GROUP BY).
const SUMMARY_PAGE: usize = 500;
