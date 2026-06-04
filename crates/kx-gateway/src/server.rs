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

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config::GatewayConfig;
use crate::error::GatewayError;

#[cfg(feature = "embedded-worker")]
use std::sync::Arc;
#[cfg(feature = "embedded-worker")]
use std::time::Duration;
#[cfg(feature = "embedded-worker")]
use {
    kx_capability::{CapabilityBroker, LocalCapabilityBroker},
    kx_content::{ContentRef, ContentStore, LocalFsContentStore},
    kx_coordinator::CoordinatorService,
    kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor},
    kx_gateway_core::{GatewayService, ReadOnly, TonicCoordinatorSubmitter},
    kx_journal::SqliteJournal,
    kx_proto::proto::coordinator_server::CoordinatorServer,
    kx_proto::proto::kx_gateway_server::KxGatewayServer,
    kx_warrant::ExecutorClass,
    kx_worker::{Worker, WorkerClient, DEFAULT_HEARTBEAT_CADENCE},
    tonic::transport::Server,
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

/// A running gateway: the bound address plus the handles needed to stop it
/// gracefully. Returned by [`start`]; [`serve`] drives it to a Ctrl-C.
pub struct RunningGateway {
    local_addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    gateway: JoinHandle<Result<(), GatewayError>>,
    /// Background tasks (embedded coordinator server, worker loop, heartbeat)
    /// aborted after the gateway drains.
    aux: Vec<JoinHandle<()>>,
}

impl RunningGateway {
    /// The address the gateway gRPC service is bound to (resolved from a `:0`
    /// request to the OS-assigned port).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stop accepting new gateway RPCs, drain in-flight ones, then abort the
    /// embedded coordinator + worker. The journal is always left at a safe
    /// boundary: commits are durable before they are acked, and any
    /// leased-but-uncommitted Mote is re-leased on the next start.
    pub async fn shutdown(self) -> Result<(), GatewayError> {
        let RunningGateway {
            shutdown,
            gateway,
            aux,
            ..
        } = self;
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
    // Rule 8c: the dev local-allow resolver is loopback-only. (Deny-all may bind
    // anywhere — every RPC is refused, so a public bind is still a closed door.)
    if cfg.dev_allow_local && !cfg.listen.ip().is_loopback() {
        return Err(GatewayError::Config(
            "--dev-allow-local permits a loopback --listen address only".into(),
        ));
    }
    start_impl(cfg).await
}

/// Start the server, then block until Ctrl-C and shut down gracefully.
pub async fn serve(cfg: GatewayConfig) -> Result<(), GatewayError> {
    let running = start(cfg).await?;
    tracing::info!(addr = %running.local_addr(), "kx-gateway listening (Ctrl-C to stop)");
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received; draining");
    running.shutdown().await
}

#[cfg(feature = "embedded-worker")]
async fn start_impl(cfg: GatewayConfig) -> Result<RunningGateway, GatewayError> {
    let content = Arc::new(
        LocalFsContentStore::open(&cfg.content_root)
            .map_err(|e| GatewayError::Content(e.to_string()))?,
    );

    // (1) Embedded coordinator — the SOLE journal writer. It opens the journal
    //     read-write (by value) and verifies each committed result_ref against
    //     the shared store (D55). Hosted on a loopback ephemeral port.
    let writer =
        SqliteJournal::open(&cfg.journal_path).map_err(|e| GatewayError::Journal(e.to_string()))?;
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
    let executor: Arc<dyn MoteExecutor> = storing_executor(content.clone());
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
    let reader = Arc::new(ReadOnly::new(read_journal));
    let submitter = Arc::new(
        TonicCoordinatorSubmitter::connect(coord_endpoint.clone())
            .await
            .map_err(|e| GatewayError::Coordinator(e.to_string()))?,
    );
    let gateway = GatewayService::new(reader, submitter, content);

    // (4) Auth interceptor (deny-all unless --dev-allow-local) + bind + serve.
    let resolver: Arc<dyn crate::auth::PrincipalResolver> = if cfg.dev_allow_local {
        Arc::new(crate::auth::DevAllowLocal)
    } else {
        Arc::new(crate::auth::DenyAll)
    };
    let svc = KxGatewayServer::with_interceptor(gateway, crate::auth::interceptor(resolver));
    let local_addr = resolve_listen(cfg.listen).await?;
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let gateway = tokio::spawn(async move {
        Server::builder()
            .add_service(svc)
            .serve_with_shutdown(local_addr, async move {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(|e| GatewayError::Bind(e.to_string()))
    });

    Ok(RunningGateway {
        local_addr,
        shutdown,
        gateway,
        aux: vec![coord_task, worker_task, heartbeat_task],
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
