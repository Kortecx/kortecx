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
use kx_mote::{EffectPattern, Mote, MoteId, ToolName, ToolVersion};
use kx_runtime::{CrashPoint, SnapshotSink};
use kx_tool_registry::ToolRegistry;
use kx_warrant::{FsScope, NetScope, WarrantSpec};

use crate::{context, prompt, toolcall};

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
///
/// Holds the D78 context seams (snapshot sink + tool registry) so a ROND/WM
/// **model** Mote assembles its upstream context + tool menu before dispatch,
/// exactly like [`crate::ModelExecutor`]; a tool Mote (no prompt) is unaffected.
pub struct ModelBroker<B: InferenceBackend, S: ContentStore> {
    backend: Arc<B>,
    store: Arc<S>,
    crash_at: Option<CrashPoint>,
    stc_crash_target: Option<MoteId>,
    observer: Arc<BrokerObserver>,
    sink: SnapshotSink,
    registry: Arc<dyn ToolRegistry>,
    /// M5.2: where a model-proposed tool call is dispatched after the fail-closed
    /// decode. Holds the concrete `McpCapability` (registered by the caller) so the
    /// proposal flows through the authoritative `LocalCapabilityBroker::precheck`
    /// warrant gate (net_scope ⊆ warrant, tool_grants, pattern) — never a second,
    /// re-implemented gate. An empty broker means "no tools" (the model's proposals,
    /// if any, are refused as ungranted before reaching here).
    tool_broker: Arc<dyn CapabilityBroker>,
}

impl<B: InferenceBackend, S: ContentStore> std::fmt::Debug for ModelBroker<B, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn ToolRegistry` is not `Debug`; elide it (mirrors `kx-inference`'s
        // `Dispatcher` Debug impl for its `dyn ModelRegistry`).
        f.debug_struct("ModelBroker")
            .field("crash_at", &self.crash_at)
            .field("stc_crash_target", &self.stc_crash_target)
            .field("observer", &self.observer)
            .field("sink", &self.sink)
            .field("registry", &"<dyn ToolRegistry>")
            .field("tool_broker", &"<dyn CapabilityBroker>")
            .finish_non_exhaustive()
    }
}

impl<B: InferenceBackend, S: ContentStore> ModelBroker<B, S> {
    /// Build a broker over a shared backend + content store, with optional
    /// `PreCommitStc` crash injection on `stc_crash_target`, writing counters
    /// through `observer`, plus the D78 context seams (snapshot sink + tool
    /// registry).
    #[must_use]
    #[allow(clippy::too_many_arguments)] // Wiring seam; grouped meaningfully (M5.2 adds tool_broker).
    pub fn new(
        backend: Arc<B>,
        store: Arc<S>,
        crash_at: Option<CrashPoint>,
        stc_crash_target: Option<MoteId>,
        observer: Arc<BrokerObserver>,
        sink: SnapshotSink,
        registry: Arc<dyn ToolRegistry>,
        tool_broker: Arc<dyn CapabilityBroker>,
    ) -> Self {
        Self {
            backend,
            store,
            crash_at,
            stc_crash_target,
            observer,
            sink,
            registry,
            tool_broker,
        }
    }

    /// Deterministic mock-tool response bytes, bound to the Mote's identity so a
    /// re-dispatch stages byte-identical bytes (content-addressed dedup).
    fn tool_response(mote_id: &MoteId) -> Vec<u8> {
        let mut bytes = b"kx-model-harness-tool:".to_vec();
        bytes.extend_from_slice(mote_id.as_bytes());
        bytes
    }

    /// Scenario-1 (`PreCommitStc`) crash injection: abort AFTER the effect is staged
    /// (the staged content + `EffectStaged` journal entry are durable) but BEFORE
    /// the commit protocol appends `Committed`. Shared by the model-completion and
    /// the model-driven-MCP staging paths.
    fn maybe_crash_pre_commit_stc(&self, mote_id: MoteId) {
        if self.crash_at == Some(CrashPoint::PreCommitStc) && self.stc_crash_target == Some(mote_id)
        {
            CrashPoint::PreCommitStc.abort_now();
        }
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
        let bytes = if let Some(instruction) = prompt::raw_prompt(mote) {
            // D78: assemble upstream context + tool menu into the input (empty ⇒
            // byte-identical to the pre-D78 leaf path). Overflow ⇒ typed
            // `StageWriteFailed` (shaper-decision seam), never a panic.
            let input = context::model_input(
                mote,
                warrant,
                &instruction,
                &self.sink,
                &*self.store,
                &*self.registry,
            )
            .map_err(|e| BrokerError::StageWriteFailed {
                capability: capability.clone(),
                diagnostic: format!("context assembly: {e}"),
            })?;
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

            // M5.2 — IMP-5: decode a model-PROPOSED tool call, fail-closed. The
            // model selects a tool from the menu M5.1 placed in its context; the
            // runtime ENFORCES (SN-8). On a valid, warrant-granted call we route it
            // through `tool_broker` — whose `precheck` is the authoritative warrant
            // gate (net_scope ⊆ warrant, tool_grants, pattern) — and return its
            // handle (already carrying the MCP capability identity as provenance,
            // D72). No call ⇒ commit the completion bytes (byte-identical to
            // pre-M5.2; the A–J rows grant no tools ⇒ always this arm). A malformed
            // or ungranted proposal is REFUSED and never fires an effect.
            match toolcall::parse_tool_call(&out.bytes, warrant, toolcall::max_args_bytes(warrant))
            {
                Ok(Some(call)) => {
                    let effect = EffectRequest {
                        payload: call.args_bytes,
                        // MCP effects are world-mutating by default → StageThenCommit (D66).
                        pattern: EffectPattern::StageThenCommit,
                        // Run-stable dedup: a recovery re-dispatch re-derives the same
                        // token (= mote.id) so the staged effect is exactly-once
                        // (content-addressed). Matches the existing row-G tool path.
                        // M5.2b note: when the HTTP transport sends a real remote
                        // `Idempotency-Key` header, switch to `run_scoped_token`
                        // (kx_capability::token) so a re-SUBMITTED run gets a fresh key.
                        idempotency_key: Some(token),
                        // M5.2a stdio transport performs no egress.
                        net_scope: NetScope::None,
                        fs_scope: FsScope::empty(),
                    };
                    let handle = self
                        .tool_broker
                        .dispatch(mote, warrant, &call.name, effect)?;
                    self.maybe_crash_pre_commit_stc(mote.id);
                    return Ok(handle);
                }
                Ok(None) => out.bytes,
                Err(reason) => {
                    return Err(BrokerError::StageWriteFailed {
                        capability: capability.clone(),
                        diagnostic: format!("model-proposed tool call rejected: {reason:?}"),
                    });
                }
            }
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
        self.maybe_crash_pre_commit_stc(mote.id);

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
