#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-model-harness — real-model validation seam
//!
//! Runs a real, non-deterministic GGUF model through the **actual** kx-runtime
//! orchestrator (`kx_runtime::run_with_seams`) to prove the runtime guarantees
//! hold when the producer is a sampling model. It is *wiring, not a rewrite*:
//! it implements the existing `kx_executor::MoteExecutor` +
//! `kx_capability::CapabilityBroker` traits and calls the existing generic
//! lifecycle — `kx-scheduler` / `kx-executor` / `kx-inference` source is
//! untouched (the P2 thesis test).
//!
//! ## Routing
//!
//! - **PURE / greedy** model Motes → [`ModelExecutor`] (recomputable; safe to
//!   re-run).
//! - **ReadOnlyNondet / WorldMutating** model Motes + WM tool Motes →
//!   [`ModelBroker`] (committed as a fact; served-not-re-sampled on replay).
//!
//! Both seams share one [`MeteredBackend`] (`Arc`) so the dispatch count
//! aggregates — the instrument for "no re-sample after crash" (row C) and
//! "memoizer hit = 0 calls" (row E).
//!
//! ## Identity
//!
//! The [`ModelId`] folds the GGUF's blake3 + exact quant (D50): a different
//! model/quant ⇒ a different `ModelId` ⇒ a different `MoteId`. The prompt is
//! carried in `config_subset` (also identity-bearing — see [`prompt`]).

use std::path::Path;
use std::sync::Arc;

use kx_content::LocalFsContentStore;
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::LlamaInferenceBackend;
use kx_journal::SqliteJournal;
use kx_mote::{ModelId, NdClass};
use kx_runtime::{run_with_seams, DemoWorkflow, RunOutcome, RuntimeConfig, RuntimeError};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

pub mod broker;
pub mod evidence;
pub mod executor;
pub mod metered;
pub mod prompt;
pub mod workflows;

pub use broker::{BrokerObserver, ModelBroker};
pub use executor::ModelExecutor;
pub use metered::MeteredBackend;

/// The exact quantization of the pinned campaign model, folded into the
/// [`ModelId`] so a different quant yields a different identity.
pub const MODEL_QUANT: &str = "q4_k_m";

/// Resolve the pinned GGUF path: the `KX_MODEL_HARNESS_GGUF` env override, else
/// `<workspace>/target/models/qwen2.5-0.5b-instruct-q4_k_m.gguf`.
#[must_use]
pub fn default_gguf_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("KX_MODEL_HARNESS_GGUF") {
        return std::path::PathBuf::from(p);
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen2.5-0.5b-instruct-q4_k_m.gguf")
}

/// Derive the pinned-model [`ModelId`] from a GGUF file: the name
/// `qwen2.5-0.5b-instruct`, the quant, and the file's blake3 (D50 — model
/// identity is content and quant, so a different model/quant yields a different
/// `ModelId`, hence a different `MoteId`).
pub fn model_id_for(gguf_path: &Path) -> std::io::Result<ModelId> {
    use std::io::Read;
    let mut f = std::fs::File::open(gguf_path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex = hasher.finalize().to_hex();
    Ok(ModelId(format!("qwen2.5-0.5b-instruct:{MODEL_QUANT}:{hex}")))
}

/// A permissive warrant for harness Motes. Unlike the demo's `permissive_warrant`
/// it sets a **positive** `wall_clock_ms` (a `0` budget makes `LlamaInferenceBackend`
/// time out immediately) and routes `model_route.model_id` to `model_id` (the
/// backend refuses a model the warrant did not authorise, D35).
pub fn harness_warrant(model_id: &ModelId, max_output_tokens: u32, wall_clock_ms: u64) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants: std::collections::BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model_id.clone(),
            max_input_tokens: 8192,
            max_output_tokens,
            max_calls: 64,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

/// Deterministic prompt shared by the bin + the crash tests for the model rows
/// (short ⇒ fast inference; greedy ⇒ reproducible).
pub const ROW_PROMPT: &str =
    "Reply with exactly one short sentence describing the color of the sky.";

/// Build the workflow for a named row. Shared by the bin + the crash tests so
/// `run`, `replay`, and the parent test all agree on Mote identities.
/// Returns `None` for an unknown row.
#[must_use]
pub fn workflow_for_row(
    row: &str,
    model_id: &ModelId,
    warrant: &WarrantSpec,
    seed: u32,
) -> Option<DemoWorkflow> {
    Some(match row {
        "serve" => workflows::serve_chain(model_id, warrant, ROW_PROMPT, seed),
        "greedy" => workflows::model_chain(
            model_id,
            warrant,
            ROW_PROMPT,
            workflows::greedy(32),
            NdClass::Pure,
        ),
        "sampled" => workflows::model_chain(
            model_id,
            warrant,
            ROW_PROMPT,
            workflows::sampled(32, seed),
            NdClass::ReadOnlyNondet,
        ),
        "tool" => workflows::tool_stage(model_id, warrant),
        _ => return None,
    })
}

/// Owns the per-run seams (content store, journal, the shared metered backend,
/// the broker observer) and drives a workflow through the real orchestrator.
#[derive(Debug)]
pub struct Harness {
    /// The on-disk content store (shared with the executor + broker + verdicts).
    pub store: Arc<LocalFsContentStore>,
    /// The on-disk SQLite journal.
    pub journal: Arc<SqliteJournal>,
    /// The metered llama.cpp backend — read `.calls()` for dispatch counts.
    pub backend: Arc<MeteredBackend<LlamaInferenceBackend>>,
    /// Broker dispatch + idempotency-token observations (survive each drive).
    pub observer: Arc<BrokerObserver>,
    /// The pinned model identity (blake3 + quant).
    pub model_id: ModelId,
}

impl Harness {
    /// Open the store + journal at the configured paths and load the GGUF model
    /// behind a metered backend keyed by `model_id`.
    pub fn open(
        config: &RuntimeConfig,
        gguf_path: &Path,
        model_id: ModelId,
    ) -> Result<Self, RuntimeError> {
        let store = Arc::new(LocalFsContentStore::open(&config.content_root)?);
        let journal = Arc::new(SqliteJournal::open(&config.journal_path)?);
        let llama = LlamaInferenceBackend::with_model(model_id.clone(), gguf_path.to_path_buf());
        let backend = Arc::new(MeteredBackend::new(llama));
        let observer = Arc::new(BrokerObserver::default());
        Ok(Self {
            store,
            journal,
            backend,
            observer,
            model_id,
        })
    }

    /// Drive `workflow` through `kx_runtime::run_with_seams` with the real model
    /// seams. Shaperless (the harness workflows are flat DAGs). Returns the run
    /// outcome (digest + committed counts); aborts the process at a configured
    /// crash point (row C / G).
    pub fn drive(
        &self,
        config: &RuntimeConfig,
        workflow: &DemoWorkflow,
    ) -> Result<RunOutcome, RuntimeError> {
        let rm = LocalResourceManager::dev_defaults();
        let executor = ModelExecutor::new(self.backend.clone(), self.store.clone());
        let broker = Arc::new(ModelBroker::new(
            self.backend.clone(),
            self.store.clone(),
            config.crash_at,
            Some(workflow.stc_crash_target),
            self.observer.clone(),
        ));
        let protocol =
            StandardCommitProtocol::new(self.store.clone(), self.journal.clone(), broker);
        run_with_seams(
            config,
            workflow,
            self.store.clone(),
            self.journal.clone(),
            &rm,
            &executor,
            &protocol,
            None,
        )
    }
}
