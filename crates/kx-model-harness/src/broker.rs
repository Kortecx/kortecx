//! [`ModelBroker`] — a [`CapabilityBroker`] that dispatches ReadOnlyNondet /
//! WorldMutating Motes either to an [`InferenceBackend`] (a model Mote, carrying
//! a prompt) or to a deterministic mock tool (a WM tool Mote, no prompt).
//!
//! This is the **serve-not-re-sample centerpiece** path. A stochastic model
//! sample is ROND and commits through the standard commit protocol
//! (`run_wm_mote → StandardCommitProtocol → broker.dispatch → R-11 → Committed`).
//! On replay, `serve_if_committed` re-reads the committed `result_ref` — the
//! broker is never called again, so the model is never re-sampled.
//!
//! The mock tool stands in for the not-yet-built MCP `Capability` (build-status
//! gap #4): its response is content-addressed to `mote.id`, so a re-dispatch on
//! recovery stages byte-identical bytes → the same ref → the journal's
//! idempotency-key dedup makes the external effect exactly-once.
//!
//! Mirrors `kx_runtime::broker::DemoBroker` (including the `PreCommitStc` crash
//! injection) and implements the existing `kx_capability::CapabilityBroker`
//! trait — `kx-capability` / `kx-inference` source is untouched (thesis test).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::ContentStore;
use kx_inference::{inference_params_from_mote, InferenceBackend};
use kx_mote::{Mote, MoteId, ToolName, ToolVersion};
use kx_runtime::CrashPoint;
use kx_warrant::WarrantSpec;

use crate::prompt;

/// Capability version reported on every harness dispatch.
const CAPABILITY_VERSION: &str = "kx-model-harness-0.1.0";

/// Shared, observable counters a [`ModelBroker`] writes through — held by the
/// caller so dispatch counts + idempotency tokens survive the broker's lifetime
/// (the broker is rebuilt per run, but the counters persist).
#[derive(Debug, Default)]
pub struct BrokerObserver {
    /// Total `dispatch` calls (model + tool).
    pub dispatches: AtomicU64,
    /// Idempotency tokens observed, in dispatch order. For row G: a re-dispatch
    /// on recovery must carry the SAME token (= `mote.id`), and the
    /// content-addressed staged ref must be identical (exactly-once effect).
    pub tokens: Mutex<Vec<[u8; 32]>>,
}

impl BrokerObserver {
    /// Number of dispatches observed.
    #[must_use]
    pub fn dispatches(&self) -> u64 {
        self.dispatches.load(Ordering::SeqCst)
    }
}

/// A [`CapabilityBroker`] backed by an [`InferenceBackend`] + a [`ContentStore`].
#[derive(Debug)]
pub struct ModelBroker<B: InferenceBackend, S: ContentStore> {
    backend: Arc<B>,
    store: Arc<S>,
    crash_at: Option<CrashPoint>,
    stc_crash_target: Option<MoteId>,
    observer: Arc<BrokerObserver>,
}

impl<B: InferenceBackend, S: ContentStore> ModelBroker<B, S> {
    /// Build a broker over a shared backend + content store, with optional
    /// `PreCommitStc` crash injection on `stc_crash_target`, writing counters
    /// through `observer`.
    #[must_use]
    pub fn new(
        backend: Arc<B>,
        store: Arc<S>,
        crash_at: Option<CrashPoint>,
        stc_crash_target: Option<MoteId>,
        observer: Arc<BrokerObserver>,
    ) -> Self {
        Self {
            backend,
            store,
            crash_at,
            stc_crash_target,
            observer,
        }
    }

    /// Deterministic mock-tool response bytes, bound to the Mote's identity so a
    /// re-dispatch stages byte-identical bytes (content-addressed dedup).
    fn tool_response(mote_id: &MoteId) -> Vec<u8> {
        let mut bytes = b"kx-model-harness-tool:".to_vec();
        bytes.extend_from_slice(mote_id.as_bytes());
        bytes
    }
}

impl<B, S> CapabilityBroker for ModelBroker<B, S>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync,
{
    fn dispatch(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        // Record the idempotency token (= mote.id, D38 §1) + bump the dispatch
        // counter. A re-dispatch on recovery re-records the SAME token.
        self.observer.dispatches.fetch_add(1, Ordering::SeqCst);
        let token = kx_capability::idempotency_token_for(mote);
        if let Ok(mut t) = self.observer.tokens.lock() {
            t.push(token);
        }

        // A model Mote (carries a prompt) runs the backend; a tool Mote stages a
        // deterministic, content-addressed response.
        let bytes = if let Some(input) = prompt::input_for(mote) {
            let params = inference_params_from_mote(mote, warrant).map_err(|e| {
                BrokerError::StageWriteFailed {
                    capability: capability.clone(),
                    diagnostic: format!("inference params: {e}"),
                }
            })?;
            let out = self
                .backend
                .dispatch(&mote.def.model_id, &input, &params, warrant)
                .map_err(|e| BrokerError::StageWriteFailed {
                    capability: capability.clone(),
                    diagnostic: format!("model dispatch: {e}"),
                })?;
            out.bytes
        } else {
            Self::tool_response(&mote.id)
        };

        // The external effect "happens" here — its payload is staged in the
        // content store. Content-addressing means a re-dispatch on recovery
        // stages byte-identical bytes → the same ref (dedup) → exactly-once.
        let staged_ref = self
            .store
            .put(&bytes)
            .map_err(|e| BrokerError::StageWriteFailed {
                capability: capability.clone(),
                diagnostic: format!("{e}"),
            })?;

        // Scenario-1 injection: abort AFTER staging (effect happened, and
        // `EffectStaged` is already in the journal because StageThenCommit
        // writes it before calling dispatch) but BEFORE the commit protocol
        // appends `Committed`.
        if self.crash_at == Some(CrashPoint::PreCommitStc) && self.stc_crash_target == Some(mote.id)
        {
            CrashPoint::PreCommitStc.abort_now();
        }

        Ok(BrokerHandle {
            staged_ref,
            capability: capability.clone(),
            capability_version: ToolVersion(CAPABILITY_VERSION.to_string()),
        })
    }

    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        // No effect read-back: recovery relies on the deterministic
        // idempotency-key dedup at re-dispatch (same as DemoBroker).
        Ok(None)
    }
}
