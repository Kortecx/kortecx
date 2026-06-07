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

use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::LocalFsContentStore;
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::LlamaInferenceBackend;
use kx_journal::SqliteJournal;
use kx_mote::{ModelId, NdClass};
use kx_runtime::{
    run_with_seams, DemoWorkflow, RunOutcome, RuntimeConfig, RuntimeError, SnapshotSink,
};
use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

pub mod broker;
mod context;
pub mod evidence;
pub mod executor;
pub mod metered;
pub mod prompt;
pub mod react;
pub mod registration;
pub mod toolcall;
pub mod topology_provider;
pub mod workflows;

pub use broker::{BrokerObserver, ModelBroker};
pub use executor::ModelExecutor;
pub use metered::MeteredBackend;
pub use react::{run_react_loop, ReactBudget, ReactLoopOutcome, ReactStop};
pub use registration::{register_kortecx, RegistrationError};
pub use topology_provider::{
    run_model_loop, run_replan_loop, LoopBudget, ModelTopologyProvider, ReplanLoopOutcome,
    ReplanOutcome,
};

/// The exact quantization of the pinned campaign model, folded into the
/// [`ModelId`] so a different quant yields a different identity.
pub const MODEL_QUANT: &str = "q4_k_m";

/// The default model NAME folded into the [`ModelId`] when `KX_MODEL_NAME` is
/// unset. Kept as the historical string so existing harness rows / recipe-reuse
/// keep their `MoteId` identities byte-stable.
pub const DEFAULT_MODEL_NAME: &str = "qwen2.5-0.5b-instruct";

/// The campaign model name: the `KX_MODEL_NAME` env override, else
/// [`DEFAULT_MODEL_NAME`]. The Qwen3-4B agent campaign sets
/// `KX_MODEL_NAME=qwen3-4b-instruct`; the default keeps the stand-in's identity.
#[must_use]
pub fn model_name() -> String {
    std::env::var("KX_MODEL_NAME").unwrap_or_else(|_| DEFAULT_MODEL_NAME.to_string())
}

/// Resolve the pinned GGUF path: the `KX_MODEL_HARNESS_GGUF` env override, else
/// `<workspace>/target/models/<model_name>-<quant>.gguf` (default
/// `qwen2.5-0.5b-instruct-q4_k_m.gguf`).
#[must_use]
pub fn default_gguf_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("KX_MODEL_HARNESS_GGUF") {
        return std::path::PathBuf::from(p);
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models")
        .join(format!("{}-{MODEL_QUANT}.gguf", model_name()))
}

/// Derive the model [`ModelId`] from a GGUF file: the [`model_name`], the quant,
/// and the file's blake3 (D50 — model identity is content + quant, so a
/// different model/quant yields a different `ModelId`, hence a different
/// `MoteId`). With `KX_MODEL_NAME` unset the id is byte-identical to the
/// historical `qwen2.5-0.5b-instruct:{quant}:{hex}` form.
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
    Ok(ModelId(format!("{}:{MODEL_QUANT}:{hex}", model_name())))
}

/// A permissive warrant for harness Motes. Unlike the demo's `permissive_warrant`
/// it sets a **positive** `wall_clock_ms` (a `0` budget makes `LlamaInferenceBackend`
/// time out immediately) and routes `model_route.model_id` to `model_id` (the
/// backend refuses a model the warrant did not authorise, D35).
pub fn harness_warrant(
    model_id: &ModelId,
    max_output_tokens: u32,
    wall_clock_ms: u64,
) -> WarrantSpec {
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
        ..Default::default()
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
        // Bind the content store so a multimodal dispatch can fetch image
        // `content_ref`s; the text path ignores it (harmless, consistent).
        let llama = LlamaInferenceBackend::with_model(model_id.clone(), gguf_path.to_path_buf())
            .with_content_store(store.clone());
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

    /// Open the store + journal and load an **image (vision) model** behind a
    /// metered backend: the VLM weights `gguf_path` plus its vision projector
    /// `mmproj_path`. The content store is bound so the multi-modal dispatch
    /// path can fetch + decode image `content_ref`s (an image-typed Data parent
    /// of a model Mote routes through this path automatically).
    ///
    /// # Errors
    /// Propagates store / journal open failures, as [`Self::open`].
    pub fn open_multimodal(
        config: &RuntimeConfig,
        gguf_path: &Path,
        mmproj_path: &Path,
        model_id: ModelId,
    ) -> Result<Self, RuntimeError> {
        let store = Arc::new(LocalFsContentStore::open(&config.content_root)?);
        let journal = Arc::new(SqliteJournal::open(&config.journal_path)?);
        let llama = LlamaInferenceBackend::with_image_model(
            model_id.clone(),
            gguf_path.to_path_buf(),
            mmproj_path.to_path_buf(),
        )
        .with_content_store(store.clone());
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
        // Default: NO MCP capability registered. A model that proposes a tool call
        // then has no matching `warrant.tool_grants` entry → `parse_tool_call`
        // returns `Ok(None)` → the path is byte-identical to pre-M5.2 (the A–J
        // rows are unaffected). The model-driven MCP path is exercised via
        // [`drive_with_tool_broker`] (M5.2 deterministic tests); a real MCP server
        // for the bin is M5.2b.
        let tool_broker: Arc<dyn CapabilityBroker> =
            Arc::new(LocalCapabilityBroker::new(self.store.clone()));
        self.drive_with_tool_broker(config, workflow, tool_broker)
    }

    /// Like [`drive`](Self::drive), but routes a model-proposed tool call through
    /// `tool_broker` (M5.2). Register the concrete `McpCapability` on a
    /// `LocalCapabilityBroker` and pass it here: a model Mote whose warrant grants
    /// that tool can then SELECT it (the runtime decodes the proposal fail-closed
    /// and dispatches through the warrant gate). An empty broker reproduces
    /// [`drive`](Self::drive) exactly.
    pub fn drive_with_tool_broker(
        &self,
        config: &RuntimeConfig,
        workflow: &DemoWorkflow,
        tool_broker: Arc<dyn CapabilityBroker>,
    ) -> Result<RunOutcome, RuntimeError> {
        let rm = LocalResourceManager::dev_defaults();
        // D78 context seams: one snapshot sink shared with the orchestrator + a
        // tool registry the assembler resolves tool grants against. The
        // builtins suffice for the A–J rows (their warrants grant no tools, so
        // assemble emits no tool items); a warrant that DOES grant a tool
        // resolves it here and the description reaches the model window.
        let sink = SnapshotSink::new();
        let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
        let executor = ModelExecutor::new(
            self.backend.clone(),
            self.store.clone(),
            sink.clone(),
            registry.clone(),
        );
        let broker = Arc::new(ModelBroker::new(
            self.backend.clone(),
            self.store.clone(),
            config.crash_at,
            Some(workflow.stc_crash_target),
            self.observer.clone(),
            sink.clone(),
            registry,
            tool_broker,
            // The A–J demo grants no tools, so the tool-dispatch arm (the only user
            // of instance_id, for the run-scoped remote idempotency key) is never
            // entered — an all-zero sentinel is inert here. A real tool-firing run
            // constructs `ModelBroker` directly with its registered instance_id
            // (D64), as the M5.2 e2e tests do.
            [0u8; kx_capability::INSTANCE_ID_LEN],
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
            // topology_provider (PR-2) — the generic driver runs flat DAGs; a
            // model-driven topology loop uses `drive_model_loop`.
            None,
            Some(&sink),
            // capture_sink — off for the harness; the runtime seam captures only
            // the action, and `Full` reasoning/thinking enrichment is M3.2 (D67).
            None,
            // audit_sink (R4) — off for the harness driver.
            None,
            // failure_policy (PR-1) — the harness drives deterministic workflows;
            // `None` keeps legacy abort-on-failure.
            None,
        )
    }

    /// PR-2 (F-4) — drive a **model-driven topology loop**. The model computes the
    /// shaper's [`kx_mote::TopologyDecision`] (via [`ModelTopologyProvider`]), its
    /// children materialize + execute, and a cold re-fold re-derives byte-identical
    /// children (R49 — the model's choice is replayed, never re-sampled).
    ///
    /// `workflow` is a [`workflows::loop_shaper`]; `recipes` is the vetted
    /// role→recipe allowlist the proposal lowers through (an unregistered role
    /// fails closed); `budget` bounds the fan-out. A refused proposal dead-letters
    /// the shaper and the run completes with no children (PR-1 discipline).
    pub fn drive_model_loop(
        &self,
        config: &RuntimeConfig,
        workflow: &DemoWorkflow,
        recipes: Arc<dyn kx_planner::RoleRecipeResolver>,
        budget: crate::LoopBudget,
    ) -> Result<RunOutcome, RuntimeError> {
        let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
        crate::run_model_loop(
            config,
            self.store.clone(),
            self.journal.clone(),
            self.backend.clone(),
            registry,
            recipes,
            workflow,
            budget,
        )
    }

    /// PR-3 (AL2) — drive a **bounded model-driven re-plan-on-failure loop**. The
    /// initial plan runs; if a step dead-letters, the model sees WHY (the read-side
    /// [`kx_projection::Snapshot::failure_reason_of`]) and proposes a correction or
    /// escalates — bounded by `budget`, every round a replayable committed fact (R49).
    ///
    /// `workflow` is a round-0 [`workflows::loop_shaper`]; the corrective rounds use
    /// [`workflows::replan_shaper`] internally. See [`run_replan_loop`].
    pub fn drive_replan_loop(
        &self,
        config: &RuntimeConfig,
        workflow: &DemoWorkflow,
        recipes: Arc<dyn kx_planner::RoleRecipeResolver>,
        budget: crate::LoopBudget,
    ) -> Result<ReplanLoopOutcome, RuntimeError> {
        let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
        crate::run_replan_loop(
            config,
            self.store.clone(),
            self.journal.clone(),
            self.backend.clone(),
            registry,
            recipes,
            workflow,
            budget,
        )
    }

    /// PR-4 (M5) — drive a **bounded model-driven tool-call ReAct loop**: the model
    /// proposes a tool, the runtime ENFORCES + fires it (SN-8), the committed result
    /// is the OBSERVATION the next turn reads back, until a final answer or a budget
    /// is hit. Each acting turn writes two durable facts (model output + observation);
    /// a crash resumes by re-folding (committed turns served, the tail exactly-once),
    /// and a cold re-fold reproduces the chain (R49).
    ///
    /// `registry` must resolve the run's MCP tool(s); `tool_broker` holds the concrete
    /// `McpCapability` under each granted tool name; `instance_id` (D64) anchors the
    /// run-scoped idempotency token. The `warrant` MUST grant every tool the model may
    /// call. See [`run_react_loop`].
    #[allow(clippy::too_many_arguments)] // distinct injected seams (registry/tool_broker/instance_id/warrant/budget)
    pub fn drive_react_loop(
        &self,
        config: &RuntimeConfig,
        warrant: &WarrantSpec,
        instruction: &str,
        registry: Arc<dyn ToolRegistry>,
        tool_broker: Arc<dyn CapabilityBroker>,
        instance_id: [u8; kx_capability::INSTANCE_ID_LEN],
        budget: crate::ReactBudget,
    ) -> Result<ReactLoopOutcome, RuntimeError> {
        crate::run_react_loop(
            config,
            self.store.clone(),
            self.journal.clone(),
            self.backend.clone(),
            registry,
            tool_broker,
            instance_id,
            &self.model_id,
            warrant,
            instruction,
            budget,
        )
    }
}
