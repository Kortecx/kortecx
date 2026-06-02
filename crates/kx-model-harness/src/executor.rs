//! [`ModelExecutor`] — a [`MoteExecutor`] that runs PURE (greedy/deterministic)
//! model Motes through an [`InferenceBackend`], and resolves non-model PURE
//! Motes (downstream consumers) to a deterministic stub result.
//!
//! Routing rationale: a **greedy** (`temperature_bps == 0`) model call is
//! recomputable — re-running it yields byte-identical output — so it is sound to
//! model as PURE and run via the per-Mote executor seam. A **stochastic** model
//! call is ReadOnlyNondet and runs through [`crate::ModelBroker`] instead (it
//! must be served from the journal on replay, never re-sampled).
//!
//! Bound by the thesis test: this is an *implementation* of the existing
//! `kx_executor::MoteExecutor` trait — `kx-executor` source is untouched.

use std::sync::Arc;

use kx_content::ContentStore;
use kx_executor::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use kx_inference::{inference_params_from_mote, InferenceBackend};
use kx_mote::Mote;
use kx_runtime::SnapshotSink;
use kx_tool_registry::ToolRegistry;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::{context, prompt};

/// A [`MoteExecutor`] backed by an [`InferenceBackend`] + a [`ContentStore`].
/// Shares the metered backend `Arc` with [`crate::ModelBroker`] so the dispatch
/// count aggregates across both seams.
///
/// Holds the D78 context seams: the [`SnapshotSink`] the orchestrator publishes
/// to, and the [`ToolRegistry`] the assembler resolves tool grants against.
pub struct ModelExecutor<B: InferenceBackend, S: ContentStore> {
    backend: Arc<B>,
    store: Arc<S>,
    sink: SnapshotSink,
    registry: Arc<dyn ToolRegistry>,
}

impl<B: InferenceBackend, S: ContentStore> std::fmt::Debug for ModelExecutor<B, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn ToolRegistry` is not `Debug`; elide it (mirrors `kx-inference`'s
        // `Dispatcher` Debug impl for its `dyn ModelRegistry`).
        f.debug_struct("ModelExecutor")
            .field("sink", &self.sink)
            .field("registry", &"<dyn ToolRegistry>")
            .finish_non_exhaustive()
    }
}

impl<B: InferenceBackend, S: ContentStore> ModelExecutor<B, S> {
    /// Build an executor over a shared backend + content store, plus the D78
    /// context seams (snapshot sink + tool registry).
    #[must_use]
    pub fn new(
        backend: Arc<B>,
        store: Arc<S>,
        sink: SnapshotSink,
        registry: Arc<dyn ToolRegistry>,
    ) -> Self {
        Self {
            backend,
            store,
            sink,
            registry,
        }
    }
}

impl<B, S> MoteExecutor for ModelExecutor<B, S>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync,
{
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        let bytes = if let Some(instruction) = prompt::raw_prompt(mote) {
            // A model Mote: greedy decode (params come verbatim from the
            // identity-bearing `mote.def.inference_params`, the SOLE permitted
            // constructor — D50). D78: the input is the Mote's instruction plus
            // any assembled upstream context + tool menu (empty ⇒ byte-identical
            // to the pre-D78 `chatml(prompt)` leaf path). An overflow surfaces a
            // typed `Internal` error here (shaper-decision seam), never a panic.
            let input = context::model_input(
                mote,
                warrant,
                &instruction,
                &self.sink,
                &*self.store,
                &*self.registry,
            )
            .map_err(|e| MoteExecutorError::Internal {
                reason: format!("context assembly: {e}"),
            })?;
            let params = inference_params_from_mote(mote, warrant).map_err(|e| {
                MoteExecutorError::Internal {
                    reason: format!("inference params: {e}"),
                }
            })?;
            let out = self
                .backend
                .dispatch(&mote.def.model_id, &input, &params, warrant)
                .map_err(|e| MoteExecutorError::Internal {
                    reason: format!("model dispatch: {e}"),
                })?;
            out.bytes
        } else {
            // A non-model PURE Mote (e.g. a downstream consumer): a deterministic
            // result bound to the Mote's identity, so two processes agree.
            let mut b = b"kx-model-harness-pure:".to_vec();
            b.extend_from_slice(mote.id.as_bytes());
            b
        };

        // The committed `result_ref` is the content hash of the produced bytes —
        // greedy decode ⇒ identical bytes ⇒ identical ref ⇒ identical digest.
        let result_ref = self
            .store
            .put(&bytes)
            .map_err(|e| MoteExecutorError::Internal {
                reason: format!("content store put: {e}"),
            })?;

        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }

    fn supports(&self, _executor_class: ExecutorClass) -> bool {
        true
    }
}
