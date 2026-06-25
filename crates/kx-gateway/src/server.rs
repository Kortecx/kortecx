//! Server wiring: bring up the (single-system) runtime and host the FROZEN
//! `KxGateway` service over it.
//!
//! With the default `embedded-worker` feature, one process hosts:
//!   1. an **embedded coordinator** (the sole journal writer, D40) on a loopback
//!      port — it owns the read-write [`SqliteJournal`] handle;
//!   2. an **embedded local worker** that leases → runs (PURE, deterministic) →
//!      proposes commits, so a submitted run actually reaches `Committed`;
//!   3. the **gateway** ([`GatewayService`]) over a SECOND, read-only journal
//!      handle + the shared content store, behind a deny-all auth interceptor.
//!
//! The gateway's `SubmitRun` proxies to the embedded coordinator via the
//! [`TonicCoordinatorSubmitter`] over loopback (reused verbatim — no new
//! submitter impl). Reads fold the journal read-only. Sole-writer is structural:
//! only the coordinator's owner thread appends; the gateway holds a
//! [`ReadOnly`] handle with no `append`.

use std::net::SocketAddr;
// `Arc` + the `AuditSink` trait are used by the (feature-independent)
// `RunningGateway` struct field + `shutdown()` flush, so they MUST be imported
// unconditionally — the rest of the audit/worker wiring is `embedded-worker`-gated.
use std::sync::Arc;

use kx_audit::AuditSink;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::config::GatewayConfig;
use crate::error::GatewayError;

#[cfg(feature = "embedded-worker")]
use std::time::Duration;
#[cfg(feature = "embedded-worker")]
use {
    crate::provision::{
        DemoLibrary, HostRecipeBinder, HostRecipeCatalog, HostSignatureCatalog, HostWorkflowAuthor,
    },
    crate::teams::{seed_workspace_team, HostGrantView, HostMembershipView},
    kx_audit::JsonlAuditSink,
    kx_capability::{CapabilityBroker, LocalCapabilityBroker},
    kx_catalog::SqliteCatalog,
    kx_content::{ContentRef, ContentStore, LocalFsContentStore},
    kx_coordinator::CoordinatorService,
    kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor},
    kx_fleet::SqliteMembershipLedger,
    kx_gateway_core::{
        EventTailer, GatewayService, GlobalEventTailer, GrantView, JournalReader, MembershipView,
        ReadOnly, RecipeBinder, RecipeCatalog, SignatureCatalog, TelemetryView,
        TonicCoordinatorSubmitter, WorkflowAuthor,
    },
    kx_journal::SqliteJournal,
    kx_proto::proto::coordinator_server::CoordinatorServer,
    kx_proto::proto::kx_gateway_server::KxGatewayServer,
    kx_warrant::ExecutorClass,
    kx_worker::{Worker, WorkerClient, DEFAULT_HEARTBEAT_CADENCE},
    std::path::{Path, PathBuf},
    // R9.5: the gRPC-web shim + deny-by-default CORS for browser unary RPCs. The
    // `http` types ride tonic's re-export (no new direct `http`/`tower` dep — both
    // are already locked transitively via tonic).
    tonic::codegen::http::{HeaderName, HeaderValue, Method},
    tonic::transport::Server,
    tonic_web::GrpcWebLayer,
    tower_http::cors::{AllowOrigin, CorsLayer},
};

/// Backoff when a `run_once` lease found no ready work (keeps an idle server off
/// a busy-spin while staying responsive when a run is submitted).
#[cfg(feature = "embedded-worker")]
const POLL_IDLE: Duration = Duration::from_millis(25);
/// Backoff after a `run_once` error (transient coordinator hiccup).
#[cfg(feature = "embedded-worker")]
const POLL_ERR: Duration = Duration::from_millis(200);

/// F-7 wiring: the model executor (as the worker's `MoteExecutor`) + the SAME `Arc`
/// in its `ContextSink` role + the served model id. `None`s ⇒ no model wired.
#[cfg(feature = "serve-engine")]
type WiredExecutor = (
    Arc<dyn MoteExecutor>,
    Option<Arc<dyn kx_worker::ContextSink>>,
    Option<kx_mote::ModelId>,
);

/// The platform-appropriate executor class the embedded worker registers as
/// (mirrors `kx_executor::default_executor()`'s platform choice). A client's
/// submitted warrant must name this class for the local worker to lease it.
#[cfg(feature = "embedded-worker")]
#[must_use]
pub fn default_executor_class() -> ExecutorClass {
    #[cfg(target_os = "macos")]
    {
        ExecutorClass::MacOsSandbox
    }
    #[cfg(not(target_os = "macos"))]
    {
        ExecutorClass::Bwrap
    }
}

/// A ready-to-send [`proto::SubmitRunRequest`](kx_proto::proto::SubmitRunRequest)
/// admitting a single PURE Mote whose warrant names the embedded worker's
/// [`default_executor_class`], so a bound `SubmitRun` leases → runs (the honest
/// passthrough) → reaches `Committed`.
///
/// A low-level PURE-run FIXTURE shared by the gateway end-to-end tests + the
/// `kx-profile` submit→Committed benchmarks — the shape mirrors
/// `tests/common::{pure_mote, pure_warrant}` so the two never drift. The
/// `recipe_fingerprint` is a fixed discovery-only sentinel (`SubmitRun` takes it
/// as-is; it is NEVER identity — the coordinator re-derives every `MoteId`
/// Rust-side, SN-8). The advisory `mote_id` inside the Mote is likewise re-derived.
#[cfg(feature = "embedded-worker")]
#[must_use]
pub fn pure_run_request() -> kx_proto::proto::SubmitRunRequest {
    use kx_proto::proto::{SubmitMoteSpec, SubmitRunRequest};
    SubmitRunRequest {
        // Fixed discovery/dedup sentinel — not identity (SN-8). Matches the e2e fixture.
        recipe_fingerprint: vec![0x5a; 32],
        motes: vec![SubmitMoteSpec {
            mote: Some(pure_run_mote(1).into()),
            warrant: Some(pure_run_warrant().into()),
            accept_at_least_once: false,
            react_seed: false,
        }],
    }
}

/// A parentless PURE Mote fixture, made unique by `seed`. Mirrors
/// `tests/common::pure_mote` (kept in lockstep so `pure_run_request` and the e2e
/// tests admit the identical Mote shape).
#[cfg(feature = "embedded-worker")]
fn pure_run_mote(seed: u8) -> kx_mote::Mote {
    use kx_mote::{
        EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
        MoteDef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use smallvec::SmallVec;
    use std::collections::BTreeMap;

    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

/// A warrant the embedded worker can lease: its `executor_class` is the
/// server's [`default_executor_class`]. Mirrors `tests/common::pure_warrant`.
#[cfg(feature = "embedded-worker")]
fn pure_run_warrant() -> kx_warrant::WarrantSpec {
    use kx_content::ContentRef;
    use kx_mote::ModelId;
    use kx_warrant::{
        FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let mut mounts = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/in"), FsMode::ReadOnly);
    let mut egress = BTreeSet::new();
    egress.insert(Host("api.example.com:443".into()));

    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist(egress),
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 3,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: Some(ContentRef::from_bytes([8u8; 32])),
        executor_class: default_executor_class(),
        ..Default::default()
    }
}

/// A running gateway: the bound address plus the handles needed to stop it
/// gracefully. Returned by [`start`]; [`serve`] drives it to a Ctrl-C.
pub struct RunningGateway {
    local_addr: SocketAddr,
    ws_local_addr: SocketAddr,
    /// Where the embedded web console (D139) is bound, when this binary carries
    /// the `console` feature and the config did not disable it.
    console_local_addr: Option<SocketAddr>,
    /// W1a (T-OBS2): where the Prometheus `/metrics` endpoint is bound, or `None`
    /// when `--metrics-listen` was not given (default OFF).
    metrics_local_addr: Option<SocketAddr>,
    /// W1a (T-OBS1): the serve-path audit sink, retained so a graceful
    /// [`RunningGateway::shutdown`] can flush its buffered trail to disk (the
    /// JSONL sink also flushes on Drop, the crash safety net). `None` ⇒ no audit.
    audit_sink: Option<Arc<dyn AuditSink>>,
    shutdown: oneshot::Sender<()>,
    /// Flips the live-tail poll loops off so their (otherwise endless) streams end
    /// and the gateway's graceful drain can complete (R5). Signalled BEFORE the
    /// gateway is awaited on shutdown.
    live_shutdown: watch::Sender<bool>,
    gateway: JoinHandle<Result<(), GatewayError>>,
    /// Background tasks (embedded coordinator server, worker loop, heartbeat, the
    /// R5 WebSocket-bridge accept loop) aborted after the gateway drains.
    aux: Vec<JoinHandle<()>>,
}

impl RunningGateway {
    /// The address the gateway gRPC service is bound to (resolved from a `:0`
    /// request to the OS-assigned port).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The address the R5 WebSocket `StreamEvents` bridge is bound to (resolved
    /// from a `:0` request to the OS-assigned port).
    #[must_use]
    pub fn ws_local_addr(&self) -> SocketAddr {
        self.ws_local_addr
    }

    /// The address the embedded web console (D139) is bound to, or `None` when
    /// the binary lacks the `console` feature or `--no-console` was given.
    #[must_use]
    pub fn console_local_addr(&self) -> Option<SocketAddr> {
        self.console_local_addr
    }

    /// W1a (T-OBS2): the address the Prometheus `/metrics` endpoint is bound to
    /// (resolved from a `:0` request), or `None` when `--metrics-listen` was not
    /// given. Scrape `http://<addr>/metrics`.
    #[must_use]
    pub fn metrics_local_addr(&self) -> Option<SocketAddr> {
        self.metrics_local_addr
    }

    /// Stop accepting new gateway RPCs, drain in-flight ones, then abort the
    /// embedded coordinator + worker. The journal is always left at a safe
    /// boundary: commits are durable before they are acked, and any
    /// leased-but-uncommitted Mote is re-leased on the next start.
    pub async fn shutdown(self) -> Result<(), GatewayError> {
        let RunningGateway {
            shutdown,
            live_shutdown,
            gateway,
            aux,
            audit_sink,
            ..
        } = self;
        // Stop the live-tail poll loops FIRST so their endless streams end —
        // otherwise the graceful drain below would wait on them forever (R5).
        let _ = live_shutdown.send(true);
        // Signal the gateway server to stop; it finishes in-flight requests
        // (which may still proxy to the not-yet-aborted coordinator).
        let _ = shutdown.send(());
        let result = match gateway.await {
            Ok(inner) => inner,
            Err(join) if join.is_cancelled() => Ok(()),
            Err(join) => Err(GatewayError::Bind(format!("gateway task failed: {join}"))),
        };
        for handle in &aux {
            handle.abort();
        }
        // W1a (T-OBS1): the in-flight commits have drained (their audit lines are
        // buffered in the sink); flush the trail to disk so a graceful shutdown
        // leaves a COMPLETE audit log (the Drop-flush only fires when the last Arc
        // drops — non-deterministic relative to this return). Best-effort.
        if let Some(sink) = &audit_sink {
            sink.flush();
        }
        result
    }
}

/// Build the runtime + bind the gateway, returning a [`RunningGateway`] handle
/// (with the bound address) without blocking. The caller owns shutdown.
pub async fn start(cfg: GatewayConfig) -> Result<RunningGateway, GatewayError> {
    // Rule 8c: the dev local-allow resolver is loopback-only — for BOTH the gRPC
    // and the WebSocket (R5) ports. (Deny-all may bind anywhere — every RPC /
    // handshake is refused, so a public bind is still a closed door.)
    if cfg.dev_allow_local && (!cfg.listen.ip().is_loopback() || !cfg.ws_listen.ip().is_loopback())
    {
        return Err(GatewayError::Config(
            "--dev-allow-local permits loopback --listen and --ws-listen addresses only".into(),
        ));
    }
    start_impl(cfg).await
}

/// Start the server, then block until a shutdown signal and drain gracefully.
///
/// Waits for **Ctrl-C (SIGINT)** on every platform, plus **SIGTERM** on Unix —
/// the signal `docker stop`, Kubernetes, and systemd send FIRST (then SIGKILL
/// after a grace period). Without the SIGTERM arm a containerized `kx serve` was
/// hard-killed at the end of the grace window, skipping the graceful drain
/// ([`RunningGateway::shutdown`] flips the live-tail loops off so the gRPC +
/// WebSocket streams end and `tonic` finishes in-flight requests). The journal is
/// crash-safe either way (replay recovers), but a clean drain avoids dropping
/// in-flight responses + leaving the live-tail sockets abruptly reset.
pub async fn serve(cfg: GatewayConfig) -> Result<(), GatewayError> {
    let running = start(cfg).await?;
    tracing::info!(addr = %running.local_addr(), "kx-gateway listening (Ctrl-C / SIGTERM to stop)");
    wait_for_shutdown_signal().await;
    tracing::info!("shutdown signal received; draining");
    running.shutdown().await
}

/// Resolve when the process receives a shutdown signal: Ctrl-C (SIGINT) on every
/// platform, or SIGTERM on Unix (whichever arrives first). If the SIGTERM handler
/// cannot be installed it falls back to Ctrl-C only — a missing handler is never a
/// hard failure of `serve`.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
            }
            Err(error) => {
                tracing::warn!(%error, "could not install a SIGTERM handler; waiting on Ctrl-C only");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// A flat, sequential wiring function: content store → coordinator → worker →
// gateway read seams → catalog → auth → (optional) TLS → bind. Splitting it would
// scatter the one-shot startup wiring across helpers for no clarity gain (the
// precedent: `kx-runtime::engine`, `kx-executor::spawn`). Allow the length.
#[allow(clippy::too_many_lines)]
#[cfg(feature = "embedded-worker")]
async fn start_impl(cfg: GatewayConfig) -> Result<RunningGateway, GatewayError> {
    let content = Arc::new(
        LocalFsContentStore::open(&cfg.content_root)
            .map_err(|e| GatewayError::Content(e.to_string()))?,
    );

    // (0) Resolve the LIVE agentic-loop runtime (PR-2b, `--features inference`): the serve
    //     model's inference backend + the role→recipe allowlist (the shaper executor lowers
    //     a model proposal through it) + the role→warrant registry (the coordinator narrows
    //     materialized children against it). Resolved BEFORE the coordinator because the
    //     coordinator needs the role registry. Fail-soft: no/unfit model ⇒ `None` ⇒ the
    //     durable spine + AL1 leaf-model path are unaffected (no shaper loop).
    // Resolve the serve runtime — the UNION of the in-process llama.cpp GGUF models
    // (only on the `inference` build) and an auto-detected Ollama daemon's models (the
    // FFI-free path). The `RoutingBackend` it holds is the executor backend + the
    // lifecycle engine + the catalog residency source; `None` ⇒ a model-less serve
    // (the durable spine + demo recipes still run). The shaper runtime + the engine
    // handle are derived from the same routing backend (Arc clones).
    #[cfg(feature = "serve-engine")]
    let serve_rt: Option<crate::model_exec::ServeRuntime> = {
        let rt = crate::model_exec::build_serve_runtime(&content);
        if let Some(r) = &rt {
            // POC-3: OPT-IN warm the PRIMARY model on startup (KX_SERVE_WARM_ON_START=1)
            // so the first chat is not a cold 12B load. OFF by default. Off-journal.
            if crate::model_exec::warm_on_start_enabled() {
                if let Err(error) = r.routing.warm(&r.primary) {
                    tracing::warn!(%error, "primary model warm-on-startup failed (will cold-load on first use)");
                }
            }
        }
        rt
    };
    #[cfg(feature = "serve-engine")]
    let model_catalog_entries: Vec<kx_gateway_core::ModelSummaryEntry> = serve_rt
        .as_ref()
        .map(|r| r.entries.clone())
        .unwrap_or_default();
    #[cfg(feature = "serve-engine")]
    let shaper_runtime: Option<crate::model_exec::ShaperRuntime> = serve_rt.as_ref().map(|r| {
        crate::model_exec::build_shaper_runtime(
            &r.primary,
            r.routing.clone(),
            default_executor_class(),
        )
    });
    #[cfg(feature = "serve-engine")]
    let model_engine: Option<Arc<crate::routing_backend::RoutingBackend>> =
        serve_rt.as_ref().map(|r| r.routing.clone());
    #[cfg(not(feature = "serve-engine"))]
    let model_catalog_entries: Vec<kx_gateway_core::ModelSummaryEntry> = Vec::new();

    // PR-B: capture the dataset server embedder from the resolved serve runtime. It
    // routes through the host `RoutingBackend`, so it embeds via the in-process
    // llama.cpp backend OR an Ollama daemon — whichever serves the embed model
    // (`KX_SERVE_EMBED_MODEL` else the primary). The warrant route names the embed
    // model (the backend refuses an off-route model). `None` (a model-less serve) ⇒
    // datasets fall back to the FFI-free client-vector path.
    #[cfg(all(feature = "hnsw", feature = "serve-engine"))]
    let dataset_embedder: Option<crate::datasets::HostEmbedder> = serve_rt.as_ref().map(|r| {
        let embed_model = r.embed_model.clone();
        crate::datasets::HostEmbedder::new(
            r.routing.clone(),
            embed_model.clone(),
            crate::model_exec::shaper_warrant(&embed_model, default_executor_class()),
        )
    });

    // (1) Embedded coordinator — the SOLE journal writer. It opens the journal
    //     read-write (by value) and verifies each committed result_ref against
    //     the shared store (D55). Hosted on a loopback ephemeral port. With a shaper
    //     runtime it also materializes + dispatches a committed shaper's children (PR-2b).
    let writer =
        SqliteJournal::open(&cfg.journal_path).map_err(|e| GatewayError::Journal(e.to_string()))?;
    // PR-6a/D155 (fs-list): the operator-granted read root (`KX_SERVE_FS_ROOT`),
    // canonicalized once. `None` (default) ⇒ fs-list is NOT registered + the
    // `kx/recipes/react-fs` recipe is NOT seeded ⇒ deny-by-default, byte-identical
    // serve. Resolved BEFORE the coordinator so its registry carries the fs-list
    // def (settle/lease args-validation) when the root is set.
    //
    // D155: resolve the operator read root NON-gated by inference. The agentic
    // fs-list/fs-read TOOLS still register only under `inference` (no model ⇒ no
    // ReAct loop to drive them), but the BRANCH snapshot (`SnapshotInto`) is a data
    // op — it reads files into CAS via the broker confinement, needs no model — so
    // its read root must resolve on any embedded-worker serve (incl. `--features
    // hnsw`). Default-OFF: `None` unless `KX_SERVE_FS_ROOT` is a resolvable dir.
    let fs_list_root: Option<std::path::PathBuf> = serve_fs_root();
    // PR-6b-2: resolve the durable catalog dir + open the durable tools registry
    // EARLY (moved up from the telemetry section) so the embedded coordinator
    // SHARES the SAME live `Arc<SqliteToolRegistry>`. The coordinator's D66
    // submission gate (and the react settle's arg-validation) then resolve a
    // runtime-DIALED or `RegisterTool`'d tool — what an authored `tool()` node /
    // an auto-grant fires — not just the bundled set. The bundled echo/fs-list
    // tools are seeded into it below (before any submission can arrive), and the
    // SqliteToolRegistry's lookups are live DB reads, so the react path resolves
    // byte-identically to the prior in-memory `registry_with_echo`.
    let catalog_dir = resolve_catalog_dir(&cfg)?;
    let tool_registry = Arc::new(
        kx_tool_registry::SqliteToolRegistry::open(catalog_dir.join("tools.db"))
            .map_err(|e| GatewayError::Config(format!("tools.db: {e}")))?,
    );
    #[cfg(feature = "serve-engine")]
    let coordinator = match shaper_runtime.as_ref() {
        Some(rt) => {
            tracing::info!("PR-2b: live model-driven topology loop enabled (kx/recipes/plan)");
            // PR-2d-2/PR-6b-2: the coordinator shares the live serve tool registry
            // (built-ins + the bundled stdio tool + fs-list@1 when a read root is
            // granted + any RegisterTool'd or runtime-DIALED external MCP tool) so
            // its settle/D66 gate validates a proposed call's args + resolves its
            // grant against the SAME registry the broker dispatch sees.
            CoordinatorService::with_store_shaper_and_tools(
                writer,
                content.clone(),
                rt.role_registry.clone(),
                tool_registry.clone(),
            )
        }
        // PR-9a (D66 model-free): a model-free serve (no shaper runtime) still
        // accumulates DIALED + RegisterTool'd tools in the SAME `tool_registry`
        // (both write paths are off the `inference` feature). Resolve the D66
        // admission gate against the live registry — not the echo-less in-memory
        // built-ins `with_store` would default to — so an authored/dialed `tool()`
        // is admissible without a served model.
        None => {
            CoordinatorService::with_store_and_tools(writer, content.clone(), tool_registry.clone())
        }
    };
    // PR-9a (D66 model-free): the FFI-free serve shares the live tool registry too,
    // so a dialed/RegisterTool'd `tool()` resolves at the coordinator D66 gate
    // (parity with the inference-shaper arm; the topology path stays inert).
    #[cfg(not(feature = "serve-engine"))]
    let coordinator =
        CoordinatorService::with_store_and_tools(writer, content.clone(), tool_registry.clone());
    // W1a (T-OBS1): build the optional serve-path operator audit sink ONCE. Opened
    // in APPEND mode so the JSONL trail accumulates across restarts; a bad/unwritable
    // path fails `start` LOUDLY (never a silently-dropped audit). Kept as a shared
    // Arc so the gateway can FLUSH it on graceful shutdown (the Drop-flush is the
    // crash safety net; an explicit flush makes a clean shutdown's trail complete).
    // OFF the truth path (record is infallible, never gates a run, never a digest
    // input) — `None` ⇒ no sink, byte-identical to today (deny-by-default).
    let audit_sink: Option<Arc<dyn AuditSink>> = match cfg.audit_log.as_ref() {
        Some(path) => {
            let sink = JsonlAuditSink::append(path).map_err(|e| {
                GatewayError::Config(format!("--audit-log {}: {e}", path.display()))
            })?;
            Some(Arc::new(sink))
        }
        None => None,
    };
    let coordinator = match &audit_sink {
        Some(sink) => coordinator.with_audit_sink(sink.clone()),
        None => coordinator,
    };
    let coord_addr = resolve_listen(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let coord_task = tokio::spawn(async move {
        if let Err(error) = Server::builder()
            .add_service(CoordinatorServer::new(coordinator))
            .serve(coord_addr)
            .await
        {
            tracing::error!(%error, "embedded coordinator server exited");
        }
    });
    let coord_endpoint = format!("http://{coord_addr}");

    // (2) Embedded local worker — leases ready PURE Motes, runs them through the
    //     deterministic content-storing executor (publishes bytes into the shared
    //     store BEFORE proposing, so D55 holds), and proposes the commit.
    //
    // (`catalog_dir` + the durable `tool_registry` were resolved EARLY above so the
    // embedded coordinator shares the live registry — PR-6b-2.)
    // Batch C: the telemetry.db sidecar (host-measured execution exhaust —
    // wall-clock / model usage / fired tool). Rebuildable-to-EMPTY, off-journal,
    // off-digest; the hot-path sink is bounded + fail-open (drop-on-full), so it
    // can never block, slow, or fail a run.
    let telemetry_ledger = Arc::new(crate::telemetry::TelemetryLedger::open(&catalog_dir)?);
    let client = connect_worker(&coord_endpoint).await?;
    // No real body is provisioned in the OSS serve path (script/tool execution is
    // OSS-scoped-out, D141.4), so the sandbox-routing seam is wired with no body ref:
    // every PURE Mote takes the honest passthrough fallback. The `RouterExecutor`
    // machinery is retained as a stable seam — a later tools/scripts batch re-enables
    // body registration with zero change here.
    let real_body_ref: Option<kx_content::ContentRef> = None;
    // The embedded worker's executor routes a real-body Mote to the platform sandbox
    // (bwrap on Linux / sandbox-exec on macOS) and a bodyless PURE Mote (`echo` /
    // `passthrough-dag`) to the HONEST passthrough fallback (GR15 — it commits the
    // Mote's real input, never a fabricated placeholder). Fail-closed: a sandbox that
    // cannot run errors (worker backs off); never host-exec.
    let executor: Arc<dyn MoteExecutor> = Arc::new(crate::real_exec::RouterExecutor::new(
        (*content).clone(),
        real_body_ref,
        default_executor_class(),
        passthrough_executor(content.clone()),
    ));
    // AL1 + PR-2b: when a fit serve model resolved (above), wrap the router so leased
    // model Motes run REAL in-process inference (`kx/recipes/chat`) AND a leased SHAPER
    // proposes topology that is lowered + committed as a TopologyDecision the coordinator
    // dispatches (`kx/recipes/plan`). The shaper arm shares the run's recipe allowlist with
    // the coordinator's role registry (both from `shaper_runtime`). Fail-soft: no model ⇒
    // unchanged behavior. The default (FFI-free) build keeps `serve_model = None`.
    // F-7 (assemble-into-serve): when the model executor is wired, it is BOTH the
    // worker's `MoteExecutor` AND its `ContextSink` (one Arc, two roles) so the
    // coordinator-resolved `parent_results` reach the model prompt. `None` ⇒ no sink
    // ⇒ the worker dispatch is byte-identical to pre-F-7.
    // PR-4.2 (T-STREAM1): the ADVISORY token broker shared by the model executor
    // (the PUBLISHER, keyed by mote.id) and the live-token subscribers (the gRPC
    // `StreamModelTokens` tailer + the WS `/tokens` bridge). Out-of-band — never
    // journal / digest / identity. Built once on the inference build; the FFI-free
    // build has no model dispatch and serves the empty `NoTokenTailer`.
    #[cfg(feature = "serve-engine")]
    let token_broker = Arc::new(crate::token_broker::TokenBroker::new());
    #[cfg(feature = "serve-engine")]
    let (executor, context_sink, serve_model): WiredExecutor = match shaper_runtime {
        Some(rt) => {
            tracing::info!(model = %rt.model_id.0, "AL1+PR-2b: live model + topology loop enabled");
            let model_exec = Arc::new(
                crate::model_exec::ModelRouterExecutor::new(
                    executor,
                    rt.backend,
                    (*content).clone(),
                    Some(rt.recipes),
                )
                // Batch C: the usage hook — every model arm funnels through
                // dispatch_model, so this records the model that ACTUALLY ran +
                // its output tokens (fail-open; dispatch unchanged).
                .with_usage_sink(Arc::new(telemetry_ledger.sink()))
                // PR-4.2: the ADVISORY token publisher — streams each model mote's
                // tokens out-of-band (keyed by mote.id). Byte-identical dispatch.
                .with_token_publisher(token_broker.clone()),
            );
            let sink: Arc<dyn kx_worker::ContextSink> = model_exec.clone();
            let wrapped: Arc<dyn MoteExecutor> = model_exec;
            (wrapped, Some(sink), Some(rt.model_id))
        }
        None => (executor, None, None),
    };
    #[cfg(not(feature = "serve-engine"))]
    let context_sink: Option<Arc<dyn kx_worker::ContextSink>> = None;
    #[cfg(not(feature = "serve-engine"))]
    let serve_model: Option<kx_mote::ModelId> = None;
    // Batch C: the OUTERMOST executor wrapper — every leased mote (echo /
    // real-exec / model / shaper / react turn / critic) gets a wall-clock row.
    // Structurally fail-open: the wrapper returns the inner result verbatim on
    // every path and records via a bounded try_send.
    let executor: Arc<dyn MoteExecutor> = Arc::new(crate::telemetry::TelemetryExecutor::new(
        executor,
        telemetry_ledger.sink(),
    ));
    // PR-6b-1: held as an `Arc` (one object, two views) so the external MCP
    // gateway can register a dialed tool's firing capability at runtime
    // (`register_capability` is `&self`/interior-mutable) on the SAME broker the
    // worker dispatches through.
    let local_broker = Arc::new(LocalCapabilityBroker::new((*content).clone()));
    // PR-2d-2 (react-tools-live): register the bundled deterministic stdio tool's
    // capability — the live ReAct loop's "Act" step — when its binary is present
    // AND a fit serve model resolved (no model ⇒ no react chain can drive it).
    // Fail-soft: no binary ⇒ no capability, no `kx/recipes/react`; unchanged serve.
    #[cfg(feature = "serve-engine")]
    let react_tool: Option<(kx_mote::ToolName, kx_mote::ToolVersion)> = if serve_model.is_some() {
        crate::mcp_tool::register_echo_capability(&local_broker)
    } else {
        None
    };
    #[cfg(not(feature = "serve-engine"))]
    let react_tool: Option<(kx_mote::ToolName, kx_mote::ToolVersion)> = None;
    // PR-6a/D155 (fs-list): register the read-only host fs-list@1 capability when a
    // read root is granted (`KX_SERVE_FS_ROOT`) AND a model is served (no model ⇒
    // no react chain to drive it). Default-OFF ⇒ no capability, no `react-fs`.
    #[cfg(feature = "serve-engine")]
    let fs_list_tool: Option<(kx_mote::ToolName, kx_mote::ToolVersion)> =
        if serve_model.is_some() && fs_list_root.is_some() {
            crate::mcp_tool::register_fs_list_capability(&local_broker);
            Some(crate::mcp_tool::fs_list_tool())
        } else {
            None
        };
    #[cfg(not(feature = "serve-engine"))]
    let fs_list_tool: Option<(kx_mote::ToolName, kx_mote::ToolVersion)> = None;
    // D155 Phase-A (fs-read): register the read-into-CAS fs-read@1 capability under
    // the SAME operator gate as fs-list (`KX_SERVE_FS_ROOT` + a served model). It
    // joins fs-list in the `react-fs` recipe (list-to-discover + read-to-ingest) and
    // the autonomous `react-auto` auto-grant set. Default-OFF ⇒ byte-identical serve.
    #[cfg(feature = "serve-engine")]
    let fs_read_tool: Option<(kx_mote::ToolName, kx_mote::ToolVersion)> =
        if serve_model.is_some() && fs_list_root.is_some() {
            crate::mcp_tool::register_fs_read_capability(&local_broker);
            Some(crate::mcp_tool::fs_read_tool())
        } else {
            None
        };
    #[cfg(not(feature = "serve-engine"))]
    let fs_read_tool: Option<(kx_mote::ToolName, kx_mote::ToolVersion)> = None;
    // PR-6b-4 (auto-grant): the operator opt-in (`KX_SERVE_AUTOGRANT`, default-OFF)
    // for the autonomous-loop tool auto-grant — gates seeding `kx/recipes/react-auto`
    // AND wiring the binder's live-warrant rebuild. Requires a served model (no model
    // ⇒ no react chain to drive). OFF ⇒ byte-identical serve (react-auto absent).
    #[cfg(feature = "serve-engine")]
    let autogrant = serve_model.is_some() && crate::mcp_tool::autogrant_enabled();
    #[cfg(not(feature = "serve-engine"))]
    let autogrant = false;
    let broker: Arc<dyn CapabilityBroker> = local_broker.clone();
    let worker = Worker::register(
        client,
        default_executor_class(),
        "inproc://kx-gateway-worker",
        executor,
        LocalResourceManager::dev_defaults(),
        content.clone(),
        broker,
        cfg.max_lease,
    )
    .await
    .map_err(|e| GatewayError::Coordinator(e.to_string()))?;
    // F-7: attach the model executor as the worker's context sink (if wired).
    let worker = match context_sink {
        Some(sink) => worker.with_context_sink(sink),
        None => worker,
    };
    // Keep the idle worker live in the registry (background heartbeat) so a run
    // submitted after an idle period leases promptly (no false-death/reschedule).
    let heartbeat_task = worker.spawn_heartbeat(DEFAULT_HEARTBEAT_CADENCE);
    let worker_task = spawn_worker_loop(worker);

    // (3) Gateway read seams: a SECOND (read-only) journal handle on the SAME
    //     path observes the coordinator's commits (WAL: one writer, many readers),
    //     plus the shared content store as the read-only content seam.
    let read_journal =
        SqliteJournal::open(&cfg.journal_path).map_err(|e| GatewayError::Journal(e.to_string()))?;
    // Typed as `Arc<dyn JournalReader>` so the SAME read-only handle backs both the
    // gateway read-fold and the R5 WebSocket bridge (cheap clone, one fold source).
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(read_journal));
    // W1a (T-OBS2): the metrics handle folds the SAME read-only journal handle into
    // RED counters (off the truth path — never journaled, never a digest input).
    // Built only when `--metrics-listen` is set (default OFF, deny-by-default). A
    // background tick refreshes the cached snapshot; the `/metrics` scrape serves it
    // without scanning the journal, so scrape latency is independent of journal size.
    let metrics_handle = cfg.metrics_listen.map(|_| {
        kx_otel::MetricsHandle::new(
            reader.clone(),
            kx_otel::BuildInfo {
                version: env!("CARGO_PKG_VERSION"),
            },
        )
    });
    let submitter = Arc::new(connect_submitter_with_retry(&coord_endpoint).await?);

    // (3b) Durable catalog directory (R2a/R2b): `--catalog-dir` (default:
    //      alongside the journal), holding the signature registry + recipe ledgers
    //      so registered signatures + recipes survive restart. Resolved up in
    //      section (2) — the telemetry ledger needed it before the executor chain.
    let signature_catalog = open_signature_catalog(&catalog_dir)?;
    // (3c) Server-provisioned recipe library (R2b) so `Invoke` runs E2E. Grant
    //      `Use` to every configured token party (+ the dev principal); the step
    //      warrant uses the embedded worker's executor_class so a bound run leases
    //      (see `provision::demo_warrant`).
    let parties: Vec<String> = cfg.auth_tokens.values().cloned().collect();
    // PR-2c-3 critic-live (H5): native deterministic critics are evaluated by the
    // inference build's `ModelRouterExecutor` (which holds `run_critic`). That executor
    // is wired exactly when a fit serve model resolved (`serve_model.is_some()`), so the
    // gateway advertises critic support iff it is present — `SubmitRun` refuses a
    // critic-bearing workflow otherwise (rather than admitting an exit-gate deadlock).
    let critics_supported = serve_model.is_some();
    // PR-2d-2: react support mirrors critic support — the react decode/fence arm
    // lives in the same inference-build executor. Tool FIRING additionally needs
    // the bundled capability (`react_tool`), which gates the recipe seeding below;
    // an answer-only react chain needs only the executor.
    let react_supported = serve_model.is_some();
    let registered_tools: std::collections::BTreeSet<(String, String)> = react_tool
        .iter()
        .map(|(id, ver)| (id.0.clone(), ver.0.clone()))
        .collect();
    // PR-6a: seed the bundled tools into the durable registry so `DiscoverTools`
    // shows the real runnable set (the OSS built-ins are re-seeded on open). When
    // the bundled echo capability resolved (`react_tool`), register `mcp-echo@1`
    // as a server-built (non-deregisterable) tool — matching the coordinator's
    // `registry_with_echo` so the inventory agrees with what the loop can fire.
    #[cfg(feature = "serve-engine")]
    if react_tool.is_some() {
        if let Err(error) = tool_registry.register_server_tool(
            crate::mcp_tool::echo_tool_def(),
            kx_tool_registry::ToolProvenance::HumanAuthored {
                author: "kx-gateway".to_string(),
            },
            None,
        ) {
            tracing::warn!(%error, "PR-6a: failed to seed mcp-echo@1 into tools.db");
        }
    }
    // PR-6a/D155 (fs-list): seed fs-list@1 into the durable registry (so
    // DiscoverTools shows the real runnable set) when the read root is granted.
    #[cfg(feature = "serve-engine")]
    if let Some(root) = fs_list_root.as_deref() {
        if fs_list_tool.is_some() {
            if let Err(error) = tool_registry.register_server_tool(
                crate::mcp_tool::fs_list_tool_def(root),
                kx_tool_registry::ToolProvenance::HumanAuthored {
                    author: "kx-gateway".to_string(),
                },
                None,
            ) {
                tracing::warn!(%error, "PR-6a: failed to seed fs-list@1 into tools.db");
            }
        }
        // D155 Phase-A: seed fs-read@1 alongside fs-list (so DiscoverTools shows the
        // real runnable set + the autonomous react-auto union can grant it).
        if fs_read_tool.is_some() {
            if let Err(error) = tool_registry.register_server_tool(
                crate::mcp_tool::fs_read_tool_def(root),
                kx_tool_registry::ToolProvenance::HumanAuthored {
                    author: "kx-gateway".to_string(),
                },
                None,
            ) {
                tracing::warn!(%error, "D155: failed to seed fs-read@1 into tools.db");
            }
        }
    }
    // Batch A: vision is a SERVE FACT derived from what actually registered
    // (the catalog entry declares "image" iff the projector resolved + the
    // backend built) — the vision recipe seeds exactly when the dispatch path
    // can honour it.
    let vision_supported = model_catalog_entries
        .iter()
        .any(|e| e.modalities.iter().any(|m| m == "image"));
    // PR-6a/D155 (fs-list + fs-read): the filesystem tool identities + read root the
    // react-fs recipe's warrant grants — `None` (default-OFF) ⇒ no react-fs recipe.
    // D155: the recipe grants BOTH fs-list@1 (discover) AND fs-read@1 (ingest).
    let fs_tools: Vec<(kx_mote::ToolName, kx_mote::ToolVersion)> =
        [fs_list_tool.clone(), fs_read_tool.clone()]
            .into_iter()
            .flatten()
            .collect();
    let fs_list_binding: Option<(
        &[(kx_mote::ToolName, kx_mote::ToolVersion)],
        &std::path::Path,
    )> = match (fs_tools.is_empty(), fs_list_root.as_deref()) {
        (false, Some(root)) => Some((fs_tools.as_slice(), root)),
        _ => None,
    };
    // POC-3: the NON-primary registered models — each gets its OWN chat recipe
    // (`kx/recipes/m-<id>`) so a chat turn can route to a chosen model. Derived from
    // the catalog entries (serving == primary); empty on a single-model serve.
    #[cfg(feature = "serve-engine")]
    let secondary_models: Vec<kx_mote::ModelId> = model_catalog_entries
        .iter()
        .filter(|e| !e.serving)
        .map(|e| kx_mote::ModelId(e.model_id.clone()))
        .collect();
    #[cfg(not(feature = "serve-engine"))]
    let secondary_models: Vec<kx_mote::ModelId> = Vec::new();
    let demo = Arc::new(DemoLibrary::open_serve(
        &catalog_dir,
        default_executor_class(),
        &parties,
        serve_model.as_ref(),
        react_tool.as_ref(),
        vision_supported,
        fs_list_binding,
        autogrant,
        &secondary_models,
    )?);
    // One seed, two seams: the binder (Invoke) and the recipe catalog (ListRecipes
    // / GetRecipeForm) share the SAME library, so the published form and the
    // executable bind agree by construction.
    // PR-6b-4: when the operator opted into auto-grant, the binder gets the LIVE
    // tool registry + broker-fireable view so a bind of `kx/recipes/react-auto`
    // rebuilds its union warrant from the live (incl. runtime-dialed) tool set.
    // PR-7: the context-bundle store (bundles.db) under the SAME catalog dir —
    // shared by the gateway service (the 4 context-bundle RPCs) AND the binder +
    // author (resolving a run's attached `context_bundles` at bind). Off-journal,
    // off-digest, rebuildable-to-empty.
    let bundles_db = Arc::new(crate::bundles::BundlesDb::open(&catalog_dir)?);
    // POC-4: the App catalog (apps.db) — caller-scoped kortecx.app/v1 envelopes for
    // the SaveApp/ListApps/GetApp RPCs. Off-journal, off-digest, rebuildable-to-empty
    // (no broker dep — app_ref is a pure content hash, the bundles.db posture).
    let apps_db = Arc::new(crate::apps::AppsDb::open(&catalog_dir)?);
    // POC-5b: the per-App lock store (locks.db) — caller-scoped branch locks toggled
    // by LockApp/UnlockApp + enforced at the AdvanceBranch chokepoint. Off-journal,
    // off-digest, rebuildable-to-empty (FAILS OPEN on loss — an availability gate).
    let locks_db = Arc::new(crate::locks::LocksDb::open(&catalog_dir)?);
    // D155 Phase-A: the branch store (branches.db) shares the content store (the
    // SnapshotInto CAS write target) and the operator FS read root (KX_SERVE_FS_ROOT,
    // default-OFF — None ⇒ SnapshotInto fails-precondition). Off-journal, off-digest.
    let branches_db = Arc::new(crate::branches::BranchesDb::open(
        &catalog_dir,
        content.clone(),
        fs_list_root.clone(),
    )?);
    // (3e) T3.7 / POC-1: the Datasets data-plane (RAG) view, behind the opt-in `hnsw`
    //      feature — a durable SQLite store + a rebuilt-on-open HNSW ANN index under
    //      the catalog dir. Built BEFORE the binder so a `kx/recipes/chat-rag` bind can
    //      ground the turn (embed → top-k → fold the exact refs). One concrete
    //      `Arc<HostDatasetView>` backs the binder grounding seam, the inline RAG seam
    //      (`DatasetView`), AND the advisory Slice-B seam (`FuzzyDiscoveryView`) — one
    //      store, three uses, cloned into the gateway below. The client-vector path is
    //      FFI-free; an `inference` build additionally wires the resolved serve model as
    //      the server embedder (text-only ingest/query).
    #[cfg(feature = "hnsw")]
    let dataset_view: Arc<crate::datasets::HostDatasetView> = {
        let datasets_dir = catalog_dir.join("datasets");
        #[cfg_attr(not(feature = "serve-engine"), allow(unused_mut))]
        let mut view = crate::datasets::HostDatasetView::open(&datasets_dir)?;
        #[cfg(feature = "serve-engine")]
        if let Some(embedder) = dataset_embedder {
            view = view.with_embedder(embedder);
        }
        Arc::new(view)
    };
    let binder: Arc<dyn RecipeBinder> = {
        #[cfg_attr(not(feature = "hnsw"), allow(unused_mut))]
        let mut host_binder = if autogrant {
            let registered: Arc<dyn kx_gateway_core::RegisteredToolsView> =
                Arc::new(HostRegisteredTools {
                    broker: local_broker.clone(),
                });
            HostRecipeBinder::from_shared_with_autogrant(
                demo.clone(),
                tool_registry.clone(),
                registered,
            )
        } else {
            HostRecipeBinder::from_shared(demo.clone())
        }
        .with_bundles(bundles_db.clone());
        // POC-1 CHAT-RAG: wire the dataset retrieval + content-staging seams (the SAME
        // `Arc<HostDatasetView>` + run content store the gateway service holds) so a
        // bind of `kx/recipes/chat-rag` with a `dataset` arg grounds the turn. Without
        // the `hnsw` feature there is no dataset view ⇒ chat-rag binds as a plain chat.
        #[cfg(feature = "hnsw")]
        {
            host_binder = host_binder.with_dataset_grounding(dataset_view.clone(), content.clone());
        }
        Arc::new(host_binder)
    };
    if autogrant {
        tracing::info!(
            "PR-6b-4: KX_SERVE_AUTOGRANT on — kx/recipes/react-auto live (auto-grants the registered/dialed tool set to the autonomous loop, capped)"
        );
    }
    let recipe_catalog: Arc<dyn RecipeCatalog> = Arc::new(HostRecipeCatalog::new(demo.clone()));
    // The Blueprint-builder author seam (SubmitWorkflow) — shares the same library
    // `Arc` (one seed, many seams), so the authoring authority resolves from the
    // SAME grant ledger Invoke uses.
    // PR-6b-2: the author shares the LIVE tool registry (the SAME `Arc` the
    // coordinator + broker hold) so a `tool()` step resolves its def + builds a
    // tool-aware authoring ceiling, and a runtime-dialed tool is authorable the
    // moment it registers.
    let author: Arc<dyn WorkflowAuthor> = Arc::new(
        HostWorkflowAuthor::from_shared_with_tools(demo.clone(), tool_registry.clone())
            .with_bundles(bundles_db.clone()),
    );
    // (3d) UI-3: a durable membership ledger (teams) under the SAME catalog dir,
    //      idempotently seeded with one workspace team (owner = the gateway principal;
    //      members = each --auth-token party + the dev principal, one a Delegate) +
    //      a team grant on `echo` so a member's warrant resolves through membership ∩
    //      grant. The grant/membership VIEW seams read it + the SHARED grant ledger;
    //      managing across parties is cloud (D129).
    let members = Arc::new(
        SqliteMembershipLedger::open(catalog_dir.join("members.db"))
            .map_err(|e| GatewayError::Catalog(e.to_string()))?,
    );
    seed_workspace_team(&members, &demo, &parties)?;
    let membership_view: Arc<dyn MembershipView> =
        Arc::new(HostMembershipView::new(members, demo.clone()));
    let grant_view: Arc<dyn GrantView> = Arc::new(HostGrantView::new(demo.clone()));
    // R5: the gRPC `StreamEvents` becomes a live tail (resumable, bounded,
    // recovery-safe). Read-side only — the digest + frozen proto are untouched. The
    // `live_shutdown` watch lets shutdown stop the poll loops (so their endless
    // streams end and the graceful drain completes).
    let (live_shutdown, live_shutdown_rx) = watch::channel(false);
    // (3f) The Morphic Data Engine (campaign Batch 2): the durable serve-path
    //      capture projection. A `capture.db` sidecar under the catalog dir,
    //      folded from the gateway's read-only journal handle (off the
    //      sole-writer thread ⇒ zero commit-latency / digest impact). Always-on
    //      (FFI-free); ActionsOnly scope (the join-key-only action exhaust).
    //      Reconciles on open (rebuild-from-journal on a stale/corrupt sidecar)
    //      then a background poller folds the journal forward until shutdown.
    let capture_ledger = Arc::new(crate::capture::CaptureLedger::open(&catalog_dir)?);
    capture_ledger.fold(reader.as_ref()); // initial backfill before serving reads
                                          // (3f-bis) Batch A: the uploads sidecar (uploads.db beside capture.db) — the
                                          //      PutContent audit rows + the uploads-scope authorized set. UNLIKE
                                          //      capture it is rebuildable-to-EMPTY (uploads never touch the journal;
                                          //      the blobs in the content store are truth). Same hard-error posture
                                          //      as capture on an unrecoverable open.
    let uploads_db = Arc::new(crate::uploads::UploadsDb::open(&catalog_dir)?);
    // (3f-ter) PR-4.1: the feedback sidecar (feedback.db beside uploads.db) — the
    //      SubmitFeedback 👍/👎 rows + their ListFeedback read-back. Client-origin
    //      product signal: rebuildable-to-EMPTY (never journaled), off-journal,
    //      off-digest. Same hard-error posture as uploads on an unrecoverable open.
    let feedback_db = Arc::new(crate::feedback::FeedbackDb::open(&catalog_dir)?);
    // (3f-quater) PR-D: the run-inputs sidecar (run_inputs.db beside feedback.db) —
    //      the Invoke args captured at submit + their GetRunInputs read-back, so a
    //      run recovered from ListRuns can pre-fill its recipe form and be
    //      re-invoked with edits ("Re-run with changes"). Rebuildable-to-EMPTY
    //      (the args never touch the journal; the run lives in the journal), off-
    //      journal, off-digest. Same hard-error posture as feedback on an
    //      unrecoverable open.
    let run_inputs_db = Arc::new(crate::run_inputs::RunInputsDb::open(&catalog_dir)?);
    // (3f-quinquies) W1a-2: the alerts sidecar (alerts.db beside run_inputs.db) —
    //      the operator alerts inbox FOLDED from the journal's terminal `Failed`
    //      facts (dead-letters + worker-reported terminal failures). A journal-
    //      DERIVED, rebuildable read-cache (the capture.db posture): deleting it
    //      and re-folding re-materializes the SAME item set. Read-only, off-
    //      journal, off-digest. The triage lifecycle (ack/resolve), the rule
    //      engine, and notifications are a Cloud capability (D156). Same hard-
    //      error posture as capture on an unrecoverable open.
    let alerts_db = Arc::new(crate::alerts::AlertsDb::open(&catalog_dir)?);
    alerts_db.fold(reader.as_ref()); // initial backfill before serving reads
    let capture_task = {
        let ledger = capture_ledger.clone();
        let reader = reader.clone();
        let mut shutdown = live_shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = tick.tick() => { ledger.fold(reader.as_ref()); }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            ledger.fold(reader.as_ref()); // final catch-up
                            break;
                        }
                    }
                }
            }
        })
    };
    // (3f-ter) Batch C: the telemetry join tick — drains the bounded hot-path
    //      event queue and joins rows to the journal's Committed facts (seq +
    //      watermark instance), mirroring the capture tick's cadence + shutdown
    //      catch-up. Off the sole-writer thread; read-only journal handle.
    let telemetry_task = {
        let ledger = telemetry_ledger.clone();
        let reader = reader.clone();
        let mut shutdown = live_shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = tick.tick() => { ledger.join_fold(reader.as_ref()); }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            ledger.join_fold(reader.as_ref()); // final catch-up
                            break;
                        }
                    }
                }
            }
        })
    };
    // (3f-quinquies) W1a-2: the alerts fold tick — incrementally folds the
    //      journal tail's terminal `Failed` facts into the alerts.db read-cache,
    //      mirroring the capture tick's cadence + shutdown catch-up. Off the
    //      sole-writer thread; read-only journal handle.
    let alerts_task = {
        let ledger = alerts_db.clone();
        let reader = reader.clone();
        let mut shutdown = live_shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = tick.tick() => { ledger.fold(reader.as_ref()); }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            ledger.fold(reader.as_ref()); // final catch-up
                            break;
                        }
                    }
                }
            }
        })
    };
    // (3f-sexies) W1a (T-OBS2): the metrics fold tick — incrementally folds the
    //      journal tail into the cached RED snapshot, mirroring the telemetry tick's
    //      cadence + shutdown catch-up. Off the sole-writer thread; read-only handle;
    //      fail-open (a fold error keeps the last good snapshot). Spawned only when
    //      `--metrics-listen` is set; `None` ⇒ no task, byte-identical to today.
    let metrics_task = metrics_handle.clone().map(|handle| {
        let mut shutdown = live_shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if let Err(error) = handle.refresh() {
                            tracing::debug!(%error, "metrics fold tick failed; serving last snapshot");
                        }
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            let _ = handle.refresh(); // final catch-up
                            break;
                        }
                    }
                }
            }
        })
    });

    // (3g) W1.A5: the always-on advisory toolscout view — manifests from the
    //      SAME registry surface the serve path resolves against (built-ins
    //      always; `mcp-echo@1` only when its capability actually registered),
    //      ranked by a startup-built index. The verdict dry-runs the REAL
    //      lowering gate against the SERVER react warrant when the react
    //      runtime is live; otherwise it degrades to UNAVAILABLE. Read-only,
    //      display-only — never an authorization (SN-8).
    // PR-6a: the advisory toolscout manifests come from the SAME durable registry
    // the serve path resolves + DiscoverTools reads (built-ins + the bundled echo
    // when its capability resolved). One source for the discovery surfaces.
    let toolscout_defs = tool_registry.defs();
    let toolscout_verdict = match (serve_model.as_ref(), react_tool.as_ref()) {
        (Some(model_id), Some(tool)) => Some(crate::toolscout::VerdictCtx {
            warrant: crate::provision::react_warrant(default_executor_class(), model_id, tool),
            model_id: model_id.clone(),
            capability: tool.0.clone(),
        }),
        _ => None,
    };
    let toolscout_view: Arc<dyn kx_gateway_core::ToolScoutView> = Arc::new(
        crate::toolscout::HostToolScout::new(&toolscout_defs, toolscout_verdict),
    );

    // POC-1 (Settings "Workspace"): the NON-SECRET config projection `GetServerInfo`
    // returns — built from `cfg` + the resolved serve model + the build's feature
    // flags. NO secret enters: the bearer token / TLS key never appear, only a
    // posture LABEL (`auth_mode`) + a `tls_enabled` boolean. Read `model_catalog_entries`
    // BEFORE it is moved into the model catalog below.
    let server_info_facts = {
        let model_id = model_catalog_entries
            .first()
            .map(|e| e.model_id.clone())
            .unwrap_or_default();
        #[cfg(feature = "inference")]
        let (model_path, feature_vision) = if model_id.is_empty() {
            (String::new(), false)
        } else {
            (
                crate::model_exec::resolve_serve_model()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                crate::model_exec::resolve_serve_mmproj().is_some(),
            )
        };
        #[cfg(not(feature = "inference"))]
        let (model_path, feature_vision) = (String::new(), false);
        let auth_mode = if cfg.dev_allow_local {
            "dev-local"
        } else if cfg.auth_tokens.is_empty() {
            "deny-all"
        } else {
            "token"
        };
        let console_addr = if cfg!(feature = "console") {
            cfg.console_listen
                .resolve()
                .map(|a| a.to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        // PR-B: the configured dataset embed model (the entry flagged `can_embed`, set
        // from KX_SERVE_EMBED_MODEL else the primary). Empty on a model-less serve.
        let embed_model_id = model_catalog_entries
            .iter()
            .find(|e| e.can_embed)
            .map(|e| e.model_id.clone())
            .unwrap_or_default();
        kx_gateway_core::ServerInfoFacts {
            model_id,
            embed_model_id,
            model_path,
            listen_addr: cfg.listen.to_string(),
            ws_addr: cfg.ws_listen.to_string(),
            console_addr,
            metrics_addr: cfg
                .metrics_listen
                .map(|a| a.to_string())
                .unwrap_or_default(),
            content_root: cfg.content_root.display().to_string(),
            journal_path: cfg.journal_path.display().to_string(),
            catalog_dir: catalog_dir.display().to_string(),
            max_lease: u64::from(cfg.max_lease),
            content_max_bytes: cfg.content_max_bytes,
            cors_origins: cfg.cors_origins.clone(),
            tls_enabled: cfg.tls.is_some(),
            auth_mode: auth_mode.to_string(),
            feature_hnsw: cfg!(feature = "hnsw"),
            feature_inference: cfg!(feature = "inference"),
            feature_console: cfg!(feature = "console"),
            feature_vision,
            audit_log_enabled: cfg.audit_log.is_some(),
            // T-MULTI-ELEMENT-TOOLCALLS: the resolved server agentic-budget defaults
            // (also the hard ceilings) — a run overrides them per-invocation.
            react_max_turns: kx_coordinator::REACT_MAX_TURNS,
            react_max_tool_calls: kx_coordinator::REACT_DEFAULT_MAX_TOOL_CALLS,
        }
    };
    // Batch A: the content WRITE seam shares the same store Arc the read seam
    // wraps (PutContent lands where GetContent reads); the model catalog is
    // always wired (an FFI-free serve answers with an honest empty list).
    let content_writer: Arc<dyn kx_gateway_core::ContentWriter> = content.clone();
    // POC-3: the FIXED registered set the lifecycle controls are scoped to (the
    // model ids the server provisioned at startup) — load/offload of anything
    // else is fail-closed NotFound.
    #[cfg(feature = "serve-engine")]
    let registered_model_ids: std::collections::BTreeSet<String> = model_catalog_entries
        .iter()
        .map(|e| e.model_id.clone())
        .collect();
    // POC-3: build the lifecycle CONTROL seam over the live routing backend (a
    // serve-engine serve only) — load/offload warm/evict the registered set's RAM
    // residency; the routing backend routes each to the owning engine (llama.cpp or
    // Ollama).
    #[cfg(feature = "serve-engine")]
    let model_lifecycle_view: Option<Arc<dyn kx_gateway_core::ModelLifecycleControl>> =
        model_engine.as_ref().map(|engine| {
            Arc::new(crate::model_lifecycle::HostModelLifecycle::new(
                engine.clone(),
                registered_model_ids,
            )) as Arc<dyn kx_gateway_core::ModelLifecycleControl>
        });
    // The model catalog is always wired (a model-less serve answers ListModels with
    // an honest empty list); on a serve-engine serve it binds the live routing backend
    // so `loaded` reflects real RAM residency (POC-3).
    #[cfg(feature = "serve-engine")]
    let models_view: Arc<dyn kx_gateway_core::ModelCatalogView> = {
        let catalog = crate::models::HostModelCatalog::new(model_catalog_entries);
        let catalog = match model_engine.as_ref() {
            Some(engine) => catalog.with_engine(engine.clone()),
            None => catalog,
        };
        Arc::new(catalog)
    };
    #[cfg(not(feature = "serve-engine"))]
    let models_view: Arc<dyn kx_gateway_core::ModelCatalogView> =
        Arc::new(crate::models::HostModelCatalog::new(model_catalog_entries));
    // Batch B: the def resolver reads the SAME store the coordinator persists
    // admitted defs into (always wired — an absent blob is `def_found = false`).
    let mote_defs_view: Arc<dyn kx_gateway_core::MoteDefView> =
        Arc::new(crate::mote_defs::HostMoteDefView::new(content.clone()));
    // POC-5a: the server-side App-scaffold orchestrator — wired only when a model is
    // served (the `app-scaffold-write` recipe is then seeded). It holds Arc clones of
    // the binder/submitter/reader/content/branch/lock seams (taken BEFORE the builder
    // chain consumes `submitter`/`content`) + spawns the background write loop.
    let app_scaffolder: Option<Arc<dyn kx_gateway_core::AppScaffolder>> = if serve_model.is_some() {
        Some(Arc::new(crate::scaffold::HostScaffolder::new(
            binder.clone(),
            submitter.clone(),
            reader.clone(),
            content.clone(),
            branches_db.clone(),
            Some(locks_db.clone() as Arc<dyn kx_gateway_core::LockStore>),
        )))
    } else {
        None
    };
    #[cfg_attr(not(feature = "hnsw"), allow(unused_mut))]
    let mut gateway = GatewayService::new(reader.clone(), submitter, content)
        .with_signature_catalog(signature_catalog)
        .with_recipe_binder(binder)
        .with_workflow_author(author)
        .with_recipe_catalog(recipe_catalog)
        .with_membership_view(membership_view)
        .with_grant_view(grant_view)
        .with_capture_view(capture_ledger)
        .with_critics_supported(critics_supported)
        .with_react_supported(react_supported)
        .with_registered_tools(registered_tools)
        .with_registered_tools_view(Arc::new(HostRegisteredTools {
            broker: local_broker.clone(),
        }))
        .with_toolscout_view(toolscout_view)
        .with_content_writer(content_writer)
        .with_uploads_ledger(uploads_db)
        .with_feedback_store(feedback_db)
        .with_run_inputs_store(run_inputs_db)
        .with_alerts_view(alerts_db)
        .with_bundles_store(bundles_db)
        .with_apps_catalog(apps_db)
        .with_branches_store(branches_db)
        .with_lock_store(locks_db)
        .with_tool_admin(Arc::new(crate::tools::HostToolRegistry::new(
            tool_registry.clone(),
            crate::tools::tool_host_allowlist(),
        )))
        .with_put_content_cap(cfg.content_max_bytes)
        .with_model_catalog_view(models_view)
        .with_server_info(server_info_facts)
        .with_mote_def_view(mote_defs_view)
        .with_telemetry_view(telemetry_ledger.clone())
        .with_event_tailer(Arc::new(crate::live_tail::LiveTailer::new(
            live_shutdown_rx.clone(),
        )))
        .with_global_event_tailer(Arc::new(crate::live_tail::GlobalLiveTailer::new(
            live_shutdown_rx.clone(),
        )));
    #[cfg(feature = "hnsw")]
    {
        gateway = gateway
            .with_dataset_view(dataset_view.clone())
            .with_fuzzy_discovery(dataset_view);
    }
    // PR-4.2 (T-STREAM1): wire the broker-backed live token tailer behind the gRPC
    // `StreamModelTokens` RPC (the inference build only; the default `NoTokenTailer`
    // serves an honest empty stream otherwise). Read-side / out-of-band.
    #[cfg(feature = "serve-engine")]
    {
        gateway = gateway.with_token_tailer(Arc::new(crate::token_tail::LiveTokenTailer::new(
            token_broker.clone(),
            live_shutdown_rx.clone(),
        )));
    }
    // POC-3: wire the model-lifecycle CONTROL seam (LoadModel/OffloadModel) over
    // the live engine. Only present when a fit model resolved; otherwise the RPCs
    // return `unimplemented` (the GetServerInfo precedent). Off-journal, off-digest.
    #[cfg(feature = "serve-engine")]
    if let Some(lifecycle) = model_lifecycle_view {
        gateway = gateway.with_model_lifecycle(lifecycle);
    }
    // POC-5a: wire the App-scaffold orchestrator (present only when a model is
    // served). Without it ScaffoldApp/GetScaffoldStatus return `unimplemented`.
    if let Some(scaffolder) = app_scaffolder {
        gateway = gateway.with_app_scaffolder(scaffolder);
    }
    // PR-6b-1: wire the EXTERNAL MCP gateway (the 5 MCP-server RPCs + the live
    // Connections govern surface). Opens the off-journal connections.db beside the
    // catalog, registers dialed tools' firing capabilities on the shared broker,
    // and re-dials persisted servers (fail-soft). A connections.db open failure
    // leaves the seam unwired (the RPCs return `unimplemented`) — never aborts serve.
    #[cfg(feature = "mcp-gateway")]
    {
        match crate::mcp_gateway::wire_mcp_gateway(
            &catalog_dir,
            tool_registry.clone(),
            local_broker.clone(),
        ) {
            Ok(admin) => {
                gateway = gateway.with_mcp_admin(admin);
                tracing::info!("PR-6b-1: external MCP gateway wired (connections.db)");
            }
            Err(error) => {
                tracing::warn!(%error, "external MCP gateway disabled (connections.db unavailable)");
            }
        }
    }

    // (4) Auth interceptor + bind + serve. Posture: --dev-allow-local (loopback
    //     dev) → configured bearer tokens → deny-all (the safe default). The SAME
    //     resolver gates the WS-bridge handshake (R5).
    let resolver: Arc<dyn crate::auth::PrincipalResolver> = if cfg.dev_allow_local {
        Arc::new(crate::auth::DevAllowLocal)
    } else if !cfg.auth_tokens.is_empty() {
        Arc::new(crate::auth::TokenResolver::new(cfg.auth_tokens.clone()))
    } else {
        Arc::new(crate::auth::DenyAll)
    };

    // (5) R5 WebSocket bridge: a second listener serving the SAME live-tail event
    //     stream over WS for browser clients, behind the same auth resolver. Bound
    //     before spawning so the resolved (ephemeral) addr is known; aborted on
    //     shutdown like the other aux tasks.
    let ws_tcp = tokio::net::TcpListener::bind(cfg.ws_listen)
        .await
        .map_err(|e| GatewayError::Bind(e.to_string()))?;
    let ws_local_addr = ws_tcp
        .local_addr()
        .map_err(|e| GatewayError::Bind(e.to_string()))?;
    // A1: the gRPC listener can be TLS, but the WebSocket bridge is still plaintext
    // ws:// (wss is a fast-follow). Say so loudly so a TLS deployment doesn't assume
    // the WS surface is encrypted — front it with a TLS proxy if browsers need wss.
    if cfg.tls.is_some() {
        tracing::warn!(
            ws_listen = %ws_local_addr,
            "gRPC TLS is enabled but the WebSocket bridge serves PLAINTEXT ws:// — \
             front it with a TLS proxy for wss (in-binary wss is a follow-on)"
        );
    }
    let ws_tailer: Arc<dyn EventTailer> =
        Arc::new(crate::live_tail::LiveTailer::new(live_shutdown_rx.clone()));
    // PR-4.2 (T-STREAM1): the WS `/tokens` token tailer — the broker-backed
    // `LiveTokenTailer` on the inference build (the browser's only live path; a
    // browser cannot speak gRPC server-streaming), the empty `NoTokenTailer`
    // otherwise. Read-side / out-of-band; same shutdown discipline.
    #[cfg(feature = "serve-engine")]
    let ws_token_tailer: Arc<dyn kx_gateway_core::TokenTailer> = Arc::new(
        crate::token_tail::LiveTokenTailer::new(token_broker.clone(), live_shutdown_rx.clone()),
    );
    #[cfg(not(feature = "serve-engine"))]
    let ws_token_tailer: Arc<dyn kx_gateway_core::TokenTailer> =
        Arc::new(kx_gateway_core::NoTokenTailer);
    // PR-4.2: a background sweep that reclaims idle/finished per-mote token
    // channels (bounded memory on a long-lived serve). Aborted on shutdown like
    // the other aux tasks.
    #[cfg(feature = "serve-engine")]
    let token_evict_task = {
        let broker = token_broker.clone();
        let mut shutdown = live_shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(crate::token_broker::BROKER_TTL);
            loop {
                tokio::select! {
                    _ = tick.tick() => broker.evict_idle(crate::token_broker::BROKER_TTL),
                    res = shutdown.changed() => {
                        if res.is_err() || *shutdown.borrow() { break; }
                    }
                }
            }
        })
    };
    // Batch C: the GLOBAL live tailer behind the WS `/events/all` channel (the
    // same journal handle + shutdown discipline as the per-run bridge tailer).
    let ws_global_tailer: Arc<dyn GlobalEventTailer> =
        Arc::new(crate::live_tail::GlobalLiveTailer::new(live_shutdown_rx));

    // (5b) D139: the embedded web console — a THIRD loopback listener serving the
    //     compile-time-embedded SPA (no runtime filesystem). BOUND here (before the
    //     CORS layer is built, so the gRPC-web allowlist can auto-extend with the
    //     console's OWN bound loopback origins — and ONLY those; deny-by-default
    //     for everything else is untouched) but SERVED only after every fallible
    //     start step below succeeds: spawning the accept loop now would leak an
    //     orphaned forever-task if TLS/CORS construction errored (the
    //     adversarial-review finding — the ws accept loop is deferred the same
    //     way). The OS backlog queues any early connections until then.
    //     Loopback-ness is enforced at parse time (config.rs); a console-less
    //     build resolves to None here.
    #[cfg(feature = "console")]
    let (console_local_addr, console_tcp) = match cfg.console_listen.resolve() {
        Some(addr) => {
            let tcp = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
                GatewayError::Bind(format!(
                    "console listener {addr}: {e} (another kx serve on this port? \
                     pick one with --console-listen <addr:port> or pass --no-console)"
                ))
            })?;
            let local = tcp
                .local_addr()
                .map_err(|e| GatewayError::Bind(e.to_string()))?;
            if cfg.tls.is_some() {
                tracing::warn!(
                    console = %local,
                    "gRPC TLS is enabled but the web console serves PLAINTEXT http:// \
                     on loopback — front it with a TLS proxy if browsers need https"
                );
            }
            (Some(local), Some(tcp))
        }
        None => (None, None),
    };
    #[cfg(not(feature = "console"))]
    let (console_local_addr, console_tcp): (Option<SocketAddr>, Option<()>) = (None, None);

    // (5c) W1a (T-OBS2): bind the Prometheus `/metrics` listener (when enabled)
    //      EARLY so a port conflict fails `start` loudly, but SERVE it LATE (aux)
    //      like the console + ws accept loops (bind early, serve late — no orphan on
    //      a later fallible step). Unauthenticated by design (the scraper convention);
    //      a non-loopback bind is allowed but warns (Cloud adds auth/party-scope).
    let (metrics_local_addr, metrics_tcp) = match cfg.metrics_listen {
        Some(addr) => {
            let tcp = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
                GatewayError::Bind(format!(
                    "metrics listener {addr}: {e} (another process on this port? \
                     pick one with --metrics-listen <addr:port>)"
                ))
            })?;
            let local = tcp
                .local_addr()
                .map_err(|e| GatewayError::Bind(e.to_string()))?;
            if !local.ip().is_loopback() {
                tracing::warn!(
                    metrics = %local,
                    "the /metrics endpoint is UNAUTHENTICATED and bound to a non-loopback \
                     address — restrict it to a trusted network (Cloud adds auth/party-scope)"
                );
            }
            (Some(local), Some(tcp))
        }
        None => (None, None),
    };

    // Cloned for the ws accept loop, which spawns LAST (after every fallible
    // start step) so a failed start never orphans it.
    let ws_resolver = resolver.clone();
    // Batch A: tonic's default 4 MiB DECODE limit would refuse a --content-max-bytes
    // PutContent at the transport before the handler's honest RESOURCE_EXHAUSTED.
    // Size the decode limit from the cap (+1 MiB proto/frame headroom); the ENCODE
    // limit keeps tonic's default (unlimited) — large committed results already
    // stream out today and lowering it would be a regression.
    let decode_limit = usize::try_from(cfg.content_max_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(1024 * 1024);
    // (`with_interceptor` is sugar that hides the sized server — compose the
    // two layers explicitly so the limit lands on the codec.)
    let svc = tonic::service::interceptor::InterceptedService::new(
        KxGatewayServer::new(gateway).max_decoding_message_size(decode_limit),
        crate::auth::interceptor(resolver),
    );
    let local_addr = resolve_listen(cfg.listen).await?;
    // A1: build the (optional) server TLS config up front so a missing/unreadable
    // cert or key fails `start` loudly — before the port is bound — never a silent
    // plaintext fall-back. (The embedded loopback coordinator + worker above stay
    // plaintext: internal traffic that never leaves the process's loopback.)
    let tls_config = match cfg.tls.as_ref() {
        Some(paths) => Some(crate::tls::server_tls_config(paths)?),
        None => None,
    };
    tracing::info!(
        tls = tls_config.is_some(),
        %local_addr,
        "gateway gRPC listener ready"
    );
    // A2: the standard `grpc.health.v1.Health` service, served alongside KxGateway
    // (NOT behind the auth interceptor — a health probe is unauthenticated by
    // design). The runtime is wired + about to bind, so the overall service ("")
    // is set SERVING; `kx health`, `grpc_health_probe`, and k8s gRPC probes read it.
    // The reporter is dropped after — the service keeps the last-set status; on
    // shutdown the port closes, so probes get connection-refused (= unhealthy).
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_service_status("", tonic_health::ServingStatus::Serving)
        .await;
    // R9.5: the deny-by-default CORS layer for the gRPC-web shim. Built BEFORE the
    // spawn so a malformed `--cors-origin` fails `start` loudly (mirrors the TLS
    // fail-fast). An empty allowlist installs a layer that matches no origin ⇒ a
    // browser is never granted cross-origin access (deny-by-default); a native
    // client carries no `Origin` header and is unaffected.
    // D139: when the console is live, its OWN loopback origins (and only those)
    // join the user's allowlist — a same-machine browser served by the console
    // can reach the gRPC-web port with zero flags. The user-listed origins keep
    // their exact semantics; the posture stays deny-by-default for everyone else.
    let mut cors_origins = cfg.cors_origins.clone();
    if let Some(addr) = console_local_addr {
        cors_origins.push(format!("http://127.0.0.1:{}", addr.port()));
        cors_origins.push(format!("http://localhost:{}", addr.port()));
    }
    let cors = build_cors_layer(&cors_origins)?;
    if !cors_origins.is_empty() {
        tracing::info!(
            origins = ?cors_origins,
            "gRPC-web CORS enabled for the listed browser origins"
        );
    }
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let gateway = tokio::spawn(async move {
        // `accept_http1(true)` lets the listener also speak HTTP/1.1 (gRPC-web rides
        // it); native HTTP/2 gRPC clients are detected by the h2 preface and are
        // UNCHANGED. CORS is the outermost layer (it answers an OPTIONS preflight
        // before the gRPC-web translation + the auth interceptor), then GrpcWebLayer
        // translates gRPC-web frames to gRPC for the KxGateway/health services.
        let mut builder = Server::builder().accept_http1(true);
        if let Some(tls) = tls_config {
            builder = builder
                .tls_config(tls)
                .map_err(|e| GatewayError::Tls(e.to_string()))?;
        }
        builder
            .layer(cors)
            .layer(GrpcWebLayer::new())
            .add_service(health_service)
            .add_service(svc)
            .serve_with_shutdown(local_addr, async move {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(|e| GatewayError::Bind(e.to_string()))
    });

    // Every fallible start step has now succeeded — the auxiliary accept loops
    // (ws bridge + console) can spawn without any orphan-on-error hazard (the
    // adversarial-review fix: bind early, serve late).
    let ws_task = tokio::spawn(crate::ws::serve_ws(
        ws_tcp,
        reader.clone(),
        ws_tailer,
        ws_global_tailer,
        ws_token_tailer,
        ws_resolver,
    ));
    // `mut` consumed by the console push (feature-gated) and the token-evict push
    // (serve-engine-gated).
    #[cfg_attr(
        all(not(feature = "console"), not(feature = "serve-engine")),
        allow(unused_mut)
    )]
    let mut aux = vec![
        coord_task,
        worker_task,
        heartbeat_task,
        ws_task,
        capture_task,
        telemetry_task,
        alerts_task,
    ];
    #[cfg(feature = "serve-engine")]
    aux.push(token_evict_task);
    #[cfg(feature = "console")]
    if let Some(tcp) = console_tcp {
        aux.push(tokio::spawn(crate::console::serve_console(tcp)));
        if let Some(local) = console_local_addr {
            tracing::info!(url = %format!("http://{local}/"), "web console ready");
        }
    }
    #[cfg(not(feature = "console"))]
    let _ = console_tcp;

    // W1a (T-OBS2): the metrics fold tick + the /metrics accept loop join the aux
    // set (spawned only when `--metrics-listen` is set; served LATE, after every
    // fallible start step). The accept loop reuses the telemetry seam for the
    // recent-window latency block (None when no model Mote has run — honest omit).
    if let Some(task) = metrics_task {
        aux.push(task);
    }
    if let (Some(tcp), Some(handle)) = (metrics_tcp, metrics_handle) {
        let telemetry_view: Option<Arc<dyn TelemetryView>> = Some(telemetry_ledger.clone());
        aux.push(tokio::spawn(crate::metrics::serve_metrics(
            tcp,
            handle,
            telemetry_view,
        )));
        if let Some(local) = metrics_local_addr {
            tracing::info!(url = %format!("http://{local}/metrics"), "metrics endpoint ready");
        }
    }

    // The operator-facing startup banner: every RESOLVED durable path + endpoint
    // in one place, so a zero-config `kx serve` (auto paths under ~/.kortecx) is
    // transparent — the operator can find the journal, inspect the sidecar DBs,
    // and point a client at the bound port without guessing.
    log_startup_banner(
        &cfg,
        &catalog_dir,
        local_addr,
        ws_local_addr,
        console_local_addr,
        metrics_local_addr,
    );

    Ok(RunningGateway {
        local_addr,
        ws_local_addr,
        console_local_addr,
        metrics_local_addr,
        audit_sink,
        shutdown,
        live_shutdown,
        gateway,
        aux,
    })
}

/// Emit the human-readable startup banner over `tracing` (the gateway never uses
/// `println!`). Reports every RESOLVED durable path (the data dir + journal +
/// content + catalog + each sidecar DB) and the actually-bound endpoints (gRPC /
/// WebSocket / console), the auth posture, and a copy-pasteable connect hint.
/// All values are post-resolution: the bound `local_addr` reflects an ephemeral
/// `:0` if one was requested, and the sidecar paths mirror the gateway's own
/// `catalog_dir` layout (catalog.db / members.db / telemetry.db / capture.db /
/// uploads.db / feedback.db / run_inputs.db / alerts.db / datasets/).
#[cfg(feature = "embedded-worker")]
fn log_startup_banner(
    cfg: &GatewayConfig,
    catalog_dir: &Path,
    local_addr: SocketAddr,
    ws_local_addr: SocketAddr,
    console_local_addr: Option<SocketAddr>,
    metrics_local_addr: Option<SocketAddr>,
) {
    let auth_mode = if cfg.dev_allow_local {
        "dev-allow-local (loopback only, no token)"
    } else if !cfg.auth_tokens.is_empty() {
        "bearer-token"
    } else {
        "deny-all (no auth posture configured)"
    };
    // The base data dir is the journal's parent (== the zero-config base under
    // ~/.kortecx); best-effort for explicit/non-sibling paths.
    let data_dir = cfg
        .journal_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let console_url =
        console_local_addr.map_or_else(|| "(disabled)".to_string(), |a| format!("http://{a}/"));
    // W1a (T-OBS2/T-OBS1): the opt-in observability surfaces, "(disabled)" when off.
    let metrics_url = metrics_local_addr.map_or_else(
        || "(disabled)".to_string(),
        |a| format!("http://{a}/metrics"),
    );
    let audit_log = cfg
        .audit_log
        .as_ref()
        .map_or_else(|| "(disabled)".to_string(), |p| p.display().to_string());
    let connect_hint = if cfg.dev_allow_local {
        format!("kx runs list --endpoint http://{local_addr}")
    } else {
        format!("kx runs list --endpoint http://{local_addr} --token <token>")
    };

    tracing::info!(
        target: "kx_gateway::startup",
        data_dir      = %data_dir.display(),
        journal       = %cfg.journal_path.display(),
        content_dir   = %cfg.content_root.display(),
        catalog_dir   = %catalog_dir.display(),
        catalog_db    = %catalog_dir.join("catalog.db").display(),
        members_db    = %catalog_dir.join("members.db").display(),
        telemetry_db  = %catalog_dir.join("telemetry.db").display(),
        capture_db    = %catalog_dir.join("capture.db").display(),
        uploads_db    = %catalog_dir.join("uploads.db").display(),
        feedback_db   = %catalog_dir.join("feedback.db").display(),
        run_inputs_db = %catalog_dir.join("run_inputs.db").display(),
        alerts_db     = %catalog_dir.join("alerts.db").display(),
        branches_db   = %catalog_dir.join("branches.db").display(),
        datasets_dir  = %catalog_dir.join("datasets").display(),
        grpc_endpoint = %format!("http://{local_addr}"),
        ws_endpoint   = %format!("ws://{ws_local_addr}"),
        console_url   = %console_url,
        metrics_url   = %metrics_url,
        audit_log     = %audit_log,
        auth_mode,
        connect_hint  = %connect_hint,
        "kx-gateway STARTUP — resolved durable layout + endpoints",
    );
}

#[cfg(not(feature = "embedded-worker"))]
#[allow(clippy::unused_async)]
async fn start_impl(_cfg: GatewayConfig) -> Result<RunningGateway, GatewayError> {
    Err(GatewayError::Unsupported(
        "kx-gateway was built without the `embedded-worker` feature (default on); \
         the gateway-only / external-coordinator mode is a later step. \
         Rebuild with default features."
            .into(),
    ))
}

/// Dial the EMBEDDED coordinator with a bounded retry (≤ 5 s of 10 ms
/// attempts). Its loopback listener was spawned moments ago in this same
/// process; on a contended host the accept loop can lag the gateway's eager
/// single dial — the CI-observed `committed_run_survives_a_restart` race. A
/// real failure (wrong port, dead task) still surfaces, just bounded-late.
#[cfg(feature = "embedded-worker")]
async fn connect_submitter_with_retry(
    endpoint: &str,
) -> Result<TonicCoordinatorSubmitter, GatewayError> {
    let mut last = String::from("never attempted");
    for _ in 0..500 {
        match TonicCoordinatorSubmitter::connect(endpoint.to_string()).await {
            Ok(submitter) => return Ok(submitter),
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Err(GatewayError::Coordinator(format!(
        "embedded coordinator never accepted at {endpoint}: {last}"
    )))
}

/// PR-6b-2: the host [`RegisteredToolsView`](kx_gateway_core::RegisteredToolsView)
/// — the authoring/invoke backstop's LIVE truth source. Wraps the serve broker so
/// a runtime-DIALED external MCP tool's `(tool_id, tool_version)` becomes
/// authorable the moment its firing capability registers (never a startup
/// snapshot). Read-only; never authorizes (SN-8) — the broker's 6-gate precheck
/// re-verifies at dispatch.
///
/// Gated to `embedded-worker`: the field references `LocalCapabilityBroker` /
/// `LocalFsContentStore`, imported only under that feature, and the struct is
/// constructed only inside the embedded `start_impl`. Without the gate the
/// `--no-default-features` build leaks the feature-gated import (the W1a-1 cfg-leak
/// class `features-guard` pins). (Fix-or-guardrail, Rule 2 — pre-existing PR-6b-2.)
#[cfg(feature = "embedded-worker")]
struct HostRegisteredTools {
    broker: Arc<LocalCapabilityBroker<LocalFsContentStore>>,
}

#[cfg(feature = "embedded-worker")]
impl kx_gateway_core::RegisteredToolsView for HostRegisteredTools {
    fn registered_grants(&self) -> std::collections::BTreeSet<(String, String)> {
        self.broker.registered_grants()
    }
}

/// Resolve the operator-granted read root from `KX_SERVE_FS_ROOT` (D155 / PR-6a).
/// `None` (unset / empty / non-existent / non-canonicalizable) ⇒ host snapshot +
/// fs-list/fs-read are OFF (deny-by-default, byte-identical serve). The path is
/// CANONICALIZED here so every downstream confinement check (the warrant grant,
/// the tool's declared scope, the capability prefix check, the branch snapshot)
/// shares one canonical root. NON-gated by `inference`: the branch `SnapshotInto`
/// data path needs the root without a model (the agentic fs tools register only
/// under `inference`, but they read the SAME knob).
#[cfg(feature = "embedded-worker")]
fn serve_fs_root() -> Option<PathBuf> {
    let raw = std::env::var_os("KX_SERVE_FS_ROOT")?;
    if raw.is_empty() {
        return None;
    }
    match PathBuf::from(&raw).canonicalize() {
        Ok(root) if root.is_dir() => Some(root),
        _ => {
            tracing::warn!(
                root = ?raw,
                "KX_SERVE_FS_ROOT is not a resolvable directory — host fs tools + branch snapshot disabled"
            );
            None
        }
    }
}

/// Resolve (creating if absent) the durable catalog directory: `--catalog-dir`
/// if set, else the journal's parent directory (else the cwd). Holds the
/// signature registry + the recipe ledgers (the G1a SQLite backends).
#[cfg(feature = "embedded-worker")]
fn resolve_catalog_dir(cfg: &GatewayConfig) -> Result<PathBuf, GatewayError> {
    let dir = cfg.catalog_dir.clone().unwrap_or_else(|| {
        cfg.journal_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    });
    std::fs::create_dir_all(&dir).map_err(|e| GatewayError::Catalog(e.to_string()))?;
    Ok(dir)
}

/// Open the durable signature catalog under `dir` and wrap it as the gateway's
/// catalog seam. Registered signatures survive restart (the G1a SQLite backend).
#[cfg(feature = "embedded-worker")]
fn open_signature_catalog(dir: &Path) -> Result<Arc<dyn SignatureCatalog>, GatewayError> {
    let registry = SqliteCatalog::open(dir.join("catalog.db"))
        .map_err(|e| GatewayError::Catalog(e.to_string()))?;
    Ok(Arc::new(HostSignatureCatalog::new(registry)))
}

/// A `MoteExecutor` for PURE Motes that PUBLISHES an HONEST passthrough of the
/// Mote's real input — its bound free-params (`config_subset`), or (when it
/// carries none) its declared `InputDataId` — into the shared store and returns
/// the ref. The correct producer for the PURE path: content-addressed (the
/// committed ref == the stored object ⇒ the coordinator's D55 phantom-ref guard
/// passes), deterministic (a pure function of the Mote's identity-bearing fields
/// ⇒ idempotent re-lease + recovery re-fold), and HONEST (GR15): the committed
/// bytes are the real input, NEVER a fabricated "demo result" placeholder — so
/// `Invoke kx/recipes/echo {topic:"hello"}` commits exactly `hello`. Built from
/// the public `TestMoteExecutor::new` — kx-executor source is untouched (the
/// frozen trio). Model recipes (chat/react/vision) route through the model
/// executor and never reach here.
#[cfg(feature = "embedded-worker")]
fn passthrough_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        let payload = passthrough_payload(mote);
        store.put(&payload).unwrap_or_else(|error| {
            // No unwrap/panic on the worker task: a phantom (absent) ref makes the
            // coordinator reject the commit; run_once errors and the loop backs off.
            tracing::error!(%error, "content-store put failed; proposing a phantom ref");
            ContentRef::from_bytes([0u8; 32])
        })
    }))
}

/// The honest passthrough bytes a PURE Mote commits (GR15). The decoded,
/// non-empty `config_subset` free-param values (the `BTreeMap` iterates sorted by
/// key ⇒ deterministic order; each value JSON-string-or-UTF-8 decoded, mirroring
/// the model arm's `prompt_from_config`) joined by newlines — a true echo of the
/// bound inputs. When the Mote carries no bound free-param (a parentless PURE
/// Mote, or a structural fan-out/gather node), it echoes the declared
/// `InputDataId` as lowercase hex — a real, printable, deterministic content
/// address, never a fabricated sentence. `PROMPT_KEY` is skipped (a prompt-bearing
/// Mote is a MODEL Mote and never reaches this PURE executor).
#[cfg(feature = "embedded-worker")]
fn passthrough_payload(mote: &kx_mote::Mote) -> Vec<u8> {
    use kx_mote::PROMPT_KEY;
    let parts: Vec<String> = mote
        .def
        .config_subset
        .iter()
        .filter(|(k, v)| k.0 != PROMPT_KEY && !v.0.is_empty())
        .map(|(_, v)| {
            serde_json::from_slice::<String>(&v.0)
                .unwrap_or_else(|_| String::from_utf8_lossy(&v.0).into_owned())
        })
        .collect();
    if parts.is_empty() {
        use std::fmt::Write as _;
        let mut hex = String::with_capacity(64);
        for b in mote.input_data_id.as_bytes() {
            let _ = write!(hex, "{b:02x}");
        }
        return hex.into_bytes();
    }
    parts.join("\n").into_bytes()
}

/// Drive the worker: lease → run → propose, forever, with idle/error backoff.
/// Aborted on shutdown. Never returns on its own.
#[cfg(feature = "embedded-worker")]
fn spawn_worker_loop(mut worker: Worker) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match worker.run_once().await {
                Ok(0) => tokio::time::sleep(POLL_IDLE).await,
                Ok(n) => tracing::debug!(committed = n, "worker committed a lease batch"),
                Err(error) => {
                    tracing::warn!(%error, "worker run_once failed; backing off");
                    tokio::time::sleep(POLL_ERR).await;
                }
            }
        }
    })
}

/// Connect a worker client to the embedded coordinator, retrying briefly while
/// the loopback server comes up (mirrors the established test idiom).
#[cfg(feature = "embedded-worker")]
async fn connect_worker(endpoint: &str) -> Result<WorkerClient, GatewayError> {
    let mut last = String::new();
    for _ in 0..100 {
        match WorkerClient::connect(endpoint.to_string()).await {
            Ok(client) => return Ok(client),
            Err(error) => {
                last = error.to_string();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    Err(GatewayError::Coordinator(format!(
        "the embedded worker could not reach the coordinator at {endpoint}: {last}"
    )))
}

/// Resolve a listen address: if the port is `0`, bind a probe to learn the
/// OS-assigned port, then release it (the server re-binds). Mirrors the existing
/// in-tree test idiom; the tiny re-bind race is acceptable for a dev server.
#[cfg(feature = "embedded-worker")]
async fn resolve_listen(listen: SocketAddr) -> Result<SocketAddr, GatewayError> {
    if listen.port() != 0 {
        return Ok(listen);
    }
    let probe = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|e| GatewayError::Bind(e.to_string()))?;
    let addr = probe
        .local_addr()
        .map_err(|e| GatewayError::Bind(e.to_string()))?;
    drop(probe);
    Ok(addr)
}

/// Build the gRPC-web CORS layer (R9.5) from the parsed `--cors-origin` allowlist.
///
/// **Deny-by-default**: an empty `origins` yields an [`AllowOrigin::list`] that
/// matches no origin, so a browser is never granted cross-origin access — and a
/// non-browser client (no `Origin` header) is untouched, so the native gRPC path is
/// unchanged. The allowlist is ALWAYS explicit (never [`AllowOrigin::any`]); the
/// `*`/`null` wildcards are already rejected at parse time
/// ([`crate::config`]'s `validate_cors_origin`), so reaching here with a bad shape
/// is impossible — the only residual failure is a header-value encoding error,
/// surfaced as a fail-closed config error before the port binds.
///
/// Allowed request headers cover the gRPC-web client surface (`content-type`,
/// `x-grpc-web`, `x-user-agent`, `grpc-timeout`) plus `authorization` (the bearer
/// token); exposed response headers carry the gRPC trailers the browser client
/// reads (`grpc-status`/`grpc-message`/`grpc-status-details-bin`). Credentials are
/// NOT enabled — the token rides the `Authorization` header, not a cookie.
#[cfg(feature = "embedded-worker")]
fn build_cors_layer(origins: &[String]) -> Result<CorsLayer, GatewayError> {
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .map(|o| {
            HeaderValue::from_str(o).map_err(|e| {
                GatewayError::Config(format!(
                    "--cors-origin {o:?} is not a valid header value: {e}"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(parsed))
        .allow_methods([Method::POST, Method::OPTIONS])
        .allow_headers([
            HeaderName::from_static("content-type"),
            HeaderName::from_static("x-grpc-web"),
            HeaderName::from_static("x-user-agent"),
            HeaderName::from_static("grpc-timeout"),
            HeaderName::from_static("authorization"),
        ])
        .expose_headers([
            HeaderName::from_static("grpc-status"),
            HeaderName::from_static("grpc-message"),
            HeaderName::from_static("grpc-status-details-bin"),
        ]))
}

#[cfg(all(test, unix))]
mod sigterm_tests {
    use std::time::Duration;
    use tokio::time::timeout;

    /// SIGTERM (what `docker stop` / Kubernetes / systemd send first) must wake the
    /// shutdown wait, not just Ctrl-C — otherwise a containerized `kx serve` is
    /// SIGKILLed after the stop-grace period and skips the graceful drain. We
    /// pre-register a SIGTERM stream so tokio installs its process-global handler
    /// (replacing the default *terminate* action — the raised signal can't kill the
    /// test binary), spawn the real `wait_for_shutdown_signal`, raise SIGTERM, and
    /// assert the wait resolves. Only our two registered streams consume the signal;
    /// no other code in this binary awaits SIGTERM, so parallel unit tests are
    /// unaffected.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wait_for_shutdown_signal_wakes_on_sigterm() {
        use tokio::signal::unix::{signal, SignalKind};
        // Install the global SIGTERM handler BEFORE raising so the default
        // terminate action is replaced and this test process survives the signal.
        let mut _guard = signal(SignalKind::terminate()).expect("install SIGTERM guard stream");
        let waiter = tokio::spawn(super::wait_for_shutdown_signal());
        // Give the spawned task a beat to register its own SIGTERM stream before we
        // raise — tokio streams only observe signals delivered AFTER registration.
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Deliver SIGTERM to this process. `nix`'s `raise` is a safe wrapper (no
        // `unsafe` block — kx-gateway forbids it); the handler installed above turns
        // delivery into a stream notification rather than terminating the test binary.
        nix::sys::signal::raise(nix::sys::signal::Signal::SIGTERM).expect("raise(SIGTERM) failed");
        timeout(Duration::from_secs(5), waiter)
            .await
            .expect("the shutdown wait did not wake on SIGTERM within 5s")
            .expect("the shutdown-wait task panicked");
    }
}
