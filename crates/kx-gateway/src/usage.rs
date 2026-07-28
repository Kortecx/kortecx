//! The model-usage hook seam, on its own so the inference dispatch path
//! (`model_exec`) depends on ONE trait object rather than on the telemetry
//! ledger module that happens to implement it today.

/// The model-usage hook the inference build's `ModelRouterExecutor` records
/// through (kept trait-shaped so `model_exec` needs no telemetry type beyond
/// one `Arc<dyn UsageSink>`). Implementations MUST be non-blocking + infallible
/// from the caller's view (the fail-open posture). Dead on the FFI-free build
/// (no model dispatch exists to record).
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
pub(crate) trait UsageSink: Send + Sync {
    /// Record that a model dispatch for `mote_id` actually ran `model_id` and
    /// emitted `output_tokens`. Never blocks; never fails the caller.
    fn record_usage(&self, mote_id: [u8; 32], model_id: &str, output_tokens: u64);
}
