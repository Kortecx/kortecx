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

use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::config::GatewayConfig;
use crate::error::GatewayError;

#[cfg(feature = "embedded-worker")]
use std::sync::Arc;
#[cfg(feature = "embedded-worker")]
use std::time::Duration;
#[cfg(feature = "embedded-worker")]
use {
    crate::provision::{DemoLibrary, HostRecipeBinder, HostSignatureCatalog},
    kx_capability::{CapabilityBroker, LocalCapabilityBroker},
    kx_catalog::SqliteCatalog,
    kx_content::{ContentRef, ContentStore, LocalFsContentStore},
    kx_coordinator::CoordinatorService,
    kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor},
    kx_gateway_core::{
        EventTailer, GatewayService, JournalReader, ReadOnly, RecipeBinder, SignatureCatalog,
        TonicCoordinatorSubmitter,
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

/// The deterministic payload the embedded demo executor publishes for a PURE
/// Mote. Exposed so an end-to-end test (a separate crate) can assert the exact
/// bytes `GetContent` returns without duplicating the format (no drift).
#[cfg(feature = "embedded-worker")]
#[must_use]
pub fn demo_pure_result(mote_id: &[u8; 32]) -> Vec<u8> {
    let mut payload = b"kx-gateway demo result for mote ".to_vec();
    payload.extend_from_slice(mote_id);
    payload
}

/// A ready-to-send [`proto::SubmitRunRequest`](kx_proto::proto::SubmitRunRequest)
/// admitting a single PURE demo Mote whose warrant names the embedded worker's
/// [`default_executor_class`], so a bound `SubmitRun` leases → runs → reaches
/// `Committed`.
///
/// This is the ONE source of truth for the demo run shared by `kx submit --demo`
/// (the R3 CLI's low-level SubmitRun path) and the gateway end-to-end tests — the
/// shape mirrors `tests/common::{pure_mote, pure_warrant}` so the two never drift.
/// The `recipe_fingerprint` is a fixed discovery-only sentinel (`SubmitRun` takes
/// it as-is; it is NEVER identity — the coordinator re-derives every `MoteId`
/// Rust-side, SN-8). The advisory `mote_id` inside the Mote is likewise re-derived.
#[cfg(feature = "embedded-worker")]
#[must_use]
pub fn demo_submit_run_request() -> kx_proto::proto::SubmitRunRequest {
    use kx_proto::proto::{SubmitMoteSpec, SubmitRunRequest};
    SubmitRunRequest {
        // Fixed discovery/dedup sentinel — not identity (SN-8). Matches the e2e fixture.
        recipe_fingerprint: vec![0x5a; 32],
        motes: vec![SubmitMoteSpec {
            mote: Some(demo_pure_mote(1).into()),
            warrant: Some(demo_pure_warrant().into()),
            accept_at_least_once: false,
        }],
    }
}

/// A parentless PURE demo Mote, made unique by `seed`. Mirrors
/// `tests/common::pure_mote` (kept in lockstep so `submit --demo` and the e2e
/// tests admit the identical Mote shape).
#[cfg(feature = "embedded-worker")]
fn demo_pure_mote(seed: u8) -> kx_mote::Mote {
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

/// A warrant the embedded demo worker can lease: its `executor_class` is the
/// server's [`default_executor_class`]. Mirrors `tests/common::pure_warrant`.
#[cfg(feature = "embedded-worker")]
fn demo_pure_warrant() -> kx_warrant::WarrantSpec {
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
    #[cfg(feature = "inference")]
    let shaper_runtime: Option<crate::model_exec::ShaperRuntime> =
        match crate::model_exec::resolve_serve_model() {
            Some(gguf) => {
                let model_id = crate::model_exec::serve_model_id(&gguf);
                match crate::model_exec::build_serve_backend(&gguf, &model_id) {
                    Ok(backend) => Some(crate::model_exec::build_shaper_runtime(
                        &model_id,
                        backend,
                        default_executor_class(),
                    )),
                    Err(error) => {
                        tracing::warn!(%error, "serve model is not fit; live loop NOT enabled");
                        None
                    }
                }
            }
            None => None,
        };

    // (1) Embedded coordinator — the SOLE journal writer. It opens the journal
    //     read-write (by value) and verifies each committed result_ref against
    //     the shared store (D55). Hosted on a loopback ephemeral port. With a shaper
    //     runtime it also materializes + dispatches a committed shaper's children (PR-2b).
    let writer =
        SqliteJournal::open(&cfg.journal_path).map_err(|e| GatewayError::Journal(e.to_string()))?;
    #[cfg(feature = "inference")]
    let coordinator = match shaper_runtime.as_ref() {
        Some(rt) => {
            tracing::info!("PR-2b: live model-driven topology loop enabled (kx/recipes/plan)");
            CoordinatorService::with_store_and_shaper_materialization(
                writer,
                content.clone(),
                rt.role_registry.clone(),
            )
        }
        None => CoordinatorService::with_store(writer, content.clone()),
    };
    #[cfg(not(feature = "inference"))]
    let coordinator = CoordinatorService::with_store(writer, content.clone());
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
    let client = connect_worker(&coord_endpoint).await?;
    // PR-9b: locate + register the sandbox demo body. `None` ⇒ no body binary on
    // this host/image, so the `exec-demo` recipe is not provisioned and the router
    // behaves exactly like the R1 storing executor.
    let real_body_ref = crate::real_exec::register_demo_body(content.as_ref());
    // Probe the sandbox once before advertising exec-demo: if it can't actually run
    // here (e.g. Docker's default seccomp blocks the user namespace bubblewrap
    // needs), DROP the body ref so exec-demo is NOT provisioned — an Invoke then
    // gets a clean refusal instead of a worker re-leasing a never-committable Mote
    // forever. The durable spine + the `echo` recipe are unaffected.
    let exec_class = default_executor_class();
    let real_body_ref = real_body_ref.filter(|&body_ref| {
        crate::real_exec::probe_sandbox(
            content.as_ref(),
            body_ref,
            exec_class,
            &crate::provision::real_exec_warrant(exec_class),
        )
    });
    // The embedded worker's executor routes a real-body Mote to the platform
    // sandbox (bwrap on Linux / sandbox-exec on macOS) and the bodyless PURE demo
    // `echo` to the unchanged deterministic storing fallback. Fail-closed: a
    // sandbox that cannot run errors (worker backs off); never host-exec.
    let executor: Arc<dyn MoteExecutor> = Arc::new(crate::real_exec::RouterExecutor::new(
        (*content).clone(),
        real_body_ref,
        default_executor_class(),
        storing_executor(content.clone()),
    ));
    // AL1 + PR-2b: when a fit serve model resolved (above), wrap the router so leased
    // model Motes run REAL in-process inference (`kx/recipes/chat`) AND a leased SHAPER
    // proposes topology that is lowered + committed as a TopologyDecision the coordinator
    // dispatches (`kx/recipes/plan`). The shaper arm shares the run's recipe allowlist with
    // the coordinator's role registry (both from `shaper_runtime`). Fail-soft: no model ⇒
    // unchanged behavior. The default (FFI-free) build keeps `serve_model = None`.
    #[cfg(feature = "inference")]
    let (executor, serve_model): (Arc<dyn MoteExecutor>, Option<kx_mote::ModelId>) =
        match shaper_runtime {
            Some(rt) => {
                tracing::info!(model = %rt.model_id.0, "AL1+PR-2b: live model + topology loop enabled");
                let wrapped: Arc<dyn MoteExecutor> =
                    Arc::new(crate::model_exec::ModelRouterExecutor::new(
                        executor,
                        rt.backend,
                        (*content).clone(),
                        Some(rt.recipes),
                    ));
                (wrapped, Some(rt.model_id))
            }
            None => (executor, None),
        };
    #[cfg(not(feature = "inference"))]
    let serve_model: Option<kx_mote::ModelId> = None;
    let broker: Arc<dyn CapabilityBroker> =
        Arc::new(LocalCapabilityBroker::new((*content).clone()));
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
    let submitter = Arc::new(
        TonicCoordinatorSubmitter::connect(coord_endpoint.clone())
            .await
            .map_err(|e| GatewayError::Coordinator(e.to_string()))?,
    );

    // (3b) Durable catalog directory (R2a/R2b): `--catalog-dir` (default:
    //      alongside the journal), holding the signature registry + recipe ledgers
    //      so registered signatures + recipes survive restart.
    let catalog_dir = resolve_catalog_dir(&cfg)?;
    let signature_catalog = open_signature_catalog(&catalog_dir)?;
    // (3c) Server-provisioned demo recipe library (R2b) so `Invoke` runs E2E.
    //      Grant `Use` to every configured token party (+ the dev principal); the
    //      step warrant uses the embedded worker's executor_class so a bound run
    //      leases (see `provision::demo_warrant`).
    let parties: Vec<String> = cfg.auth_tokens.values().cloned().collect();
    let demo = DemoLibrary::open_full(
        &catalog_dir,
        default_executor_class(),
        &parties,
        real_body_ref,
        serve_model,
    )?;
    let binder: Arc<dyn RecipeBinder> = Arc::new(HostRecipeBinder::new(demo));
    // R5: the gRPC `StreamEvents` becomes a live tail (resumable, bounded,
    // recovery-safe). Read-side only — the digest + frozen proto are untouched. The
    // `live_shutdown` watch lets shutdown stop the poll loops (so their endless
    // streams end and the graceful drain completes).
    let (live_shutdown, live_shutdown_rx) = watch::channel(false);
    let gateway = GatewayService::new(reader.clone(), submitter, content)
        .with_signature_catalog(signature_catalog)
        .with_recipe_binder(binder)
        .with_event_tailer(Arc::new(crate::live_tail::LiveTailer::new(
            live_shutdown_rx.clone(),
        )));

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
        Arc::new(crate::live_tail::LiveTailer::new(live_shutdown_rx));
    let ws_task = tokio::spawn(crate::ws::serve_ws(
        ws_tcp,
        reader.clone(),
        ws_tailer,
        resolver.clone(),
    ));

    let svc = KxGatewayServer::with_interceptor(gateway, crate::auth::interceptor(resolver));
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
    let cors = build_cors_layer(&cfg.cors_origins)?;
    if !cfg.cors_origins.is_empty() {
        tracing::info!(
            origins = ?cfg.cors_origins,
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

    Ok(RunningGateway {
        local_addr,
        ws_local_addr,
        shutdown,
        live_shutdown,
        gateway,
        aux: vec![coord_task, worker_task, heartbeat_task, ws_task],
    })
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

/// A `MoteExecutor` for PURE Motes that PUBLISHES its deterministic result bytes
/// into the shared store and returns the ref (the correct producer for the PURE
/// path — content-addressed, so the committed ref == the stored object, and the
/// coordinator's D55 phantom-ref guard passes). Built from the existing public
/// `TestMoteExecutor::new` — kx-executor source is untouched. (R1 does NOT
/// sandbox; the hardened spawn backend is a later step — stated honestly.)
#[cfg(feature = "embedded-worker")]
fn storing_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        let payload = demo_pure_result(mote.id.as_bytes());
        store.put(&payload).unwrap_or_else(|error| {
            // No unwrap/panic on the worker task: a phantom (absent) ref makes the
            // coordinator reject the commit; run_once errors and the loop backs off.
            tracing::error!(%error, "content-store put failed; proposing a phantom ref");
            ContentRef::from_bytes([0u8; 32])
        })
    }))
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
