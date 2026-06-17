#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-gateway — the kortecx single-system gateway server (R1 / D130)
//!
//! > **Phase: reachability (v0.1.0).** The first kortecx binary that binds a
//! > network port outside tests. It hosts the FROZEN
//! > [`KxGateway`](kx_proto::proto::kx_gateway_server::KxGateway) gRPC service
//! > over [`kx_gateway_core`] so a human / SDK / browser can connect and run a
//! > workflow over the wire. See the README (How it works; core-vs-distributed legend).
//!
//! ## What it hosts
//!
//! `kx-gateway serve` brings up, in ONE process:
//! - an embedded **coordinator** (the sole journal writer, D40) on a loopback
//!   port (behind the default-on `embedded-worker` feature);
//! - an embedded **local worker** that leases → runs (PURE, deterministic) →
//!   proposes commits, so a submitted run actually reaches `Committed`;
//! - the **gateway** service ([`kx_gateway_core::GatewayService`]) over a
//!   *second, read-only* journal handle + the shared content store. `SubmitRun`
//!   proxies to the embedded coordinator; `GetProjection` / `GetContent` /
//!   `StreamEvents` fold the journal read-only.
//!
//! This is a **single-system** server (D129): one process, one journal, one
//! principal namespace. Multi-node coordination + multi-tenant isolation are the
//! cloud product; a hosted OSS server is single-tenant by construction.
//!
//! ## Auth (R2: real bearer-token resolver)
//!
//! The freshly-bound port defaults to **deny-all** ([`auth::DenyAll`]): every RPC
//! returns `unauthenticated` unless the operator either passes `--dev-allow-local`
//! (a fixed local-dev principal, loopback-only) or configures bearer tokens
//! (`--auth-token <token>=<party>` / `--auth-token-file <path>`) which install a
//! [`auth::TokenResolver`]. The [`auth::PrincipalResolver`] seam is the fill point
//! — mTLS / OIDC are later impls of the same trait (OIDC stays cloud, D94/D101.1).
//! Identity is **server-derived** from transport metadata, never client-asserted
//! (SN-8): the client supplies a credential, not a claimed identity.
//!
//! ## The no-write discipline (D120.5)
//!
//! The gateway never writes the journal: it holds a [`kx_gateway_core::ReadOnly`]
//! handle (no `append`) + a `ContentReader` (no `put`) and a propose-proxy. Only
//! the embedded coordinator appends — sole-writer is structural, not by
//! convention. The frozen trio (`kx-scheduler` / `kx-executor`) source is
//! untouched; this crate only *consumes* their public API.

// PR-2c F-7: render a model Mote's resolved Data context (WorkItem.parent_results)
// into the prompt. Self-contained + FFI-free; shares the `inference` feature with
// the model executor that consumes it.
#[cfg(feature = "inference")]
mod assemble_serve;
mod auth;
mod config;
// D139: the embedded web console — hyper serving the compile-time-embedded SPA
// on a third loopback listener. Behind the off-by-default `console` feature so
// plain builds never need node or a built `ui/dist` (build.rs embeds it).
#[cfg(feature = "console")]
mod console;
// T3.7: the host-side Datasets data-plane (the kx-dataset-hnsw-backed DatasetView
// seam). Pulls the FFI-free dataset crates, so it's behind the off-by-default
// `hnsw` feature (the default build + the dep-wall stay unchanged).
#[cfg(feature = "hnsw")]
mod datasets;
// W1a-2: the alerts.db read-cache (the AlertView seam) — the operator alerts
// inbox folded from the journal's TERMINAL `Failed` facts. Read-only,
// off-truth-path, rebuildable (the capture.db posture). The triage lifecycle
// (ack/resolve), rule engine, and notifications are a Cloud capability (D156).
mod alerts;
// The Morphic Data Engine (campaign Batch 2): the durable serve-path capture
// projection (capture.db sidecar folded from the read-only journal handle).
// Always-on, off-truth-path; FFI-free (rusqlite is already in the closure).
mod capture;
mod error;
// PR-4.1: the feedback.db sidecar (the FeedbackStore seam) — the SubmitFeedback
// 👍/👎 rows + their ListFeedback read-back. Rebuildable-to-empty (client-origin
// product signal, never journaled), off-journal, off-digest. Like uploads, no
// executor wrapper — always-on, FFI-free (rusqlite already in the closure).
mod feedback;
mod live_tail;
// W1a (T-OBS2): the always-available Prometheus `/metrics` listener (opt-in via
// `--metrics-listen`). FFI-free (hyper http1 only); not feature-gated.
mod metrics;
// PR-2d-2: the bundled deterministic stdio MCP tool's wiring (locate the bin,
// register the capability + the typed ToolDef). Behind `inference` (the react
// decode arm lives in the inference-gated executor; the MCP adapter is FFI-free).
#[cfg(feature = "inference")]
mod mcp_tool;
// PR-6b-1 (D159): the EXTERNAL MCP gateway host wiring — the McpGatewayAdmin impl
// over kx_mcp_gateway::McpGateway + the BrokerCapabilitySink. Behind the
// `mcp-gateway` feature (ON by default); FFI-free, off-journal, off-digest. DIALS
// external MCP servers + governs connections; the autonomous-loop auto-grant is
// PR-6b-2 (a dialed tool is registered + fireable through a granting warrant).
#[cfg(feature = "mcp-gateway")]
mod mcp_gateway;
// AL1: the in-process model executor for `kx serve` (live LLM dispatch). Pulls
// the inference FFI, so it's behind the off-by-default `inference` feature.
#[cfg(feature = "inference")]
mod model_exec;
// PR-4.2 (T-STREAM1): the ADVISORY in-process token broker — the rendezvous
// between the model executor (publisher) and the live-token subscribers. Only
// the inference build dispatches models + publishes tokens, so it's inference-
// gated; out-of-band (never journal / digest / identity).
#[cfg(feature = "inference")]
mod token_broker;
// PR-4.2 (T-STREAM1): the LiveTokenTailer — the broker-backed `TokenTailer` impl
// the gRPC `StreamModelTokens` handler + the WS `/tokens` bridge subscribe
// through. Needs the broker, so inference-gated; the FFI-free build serves the
// core `NoTokenTailer` (an honest empty stream).
#[cfg(feature = "inference")]
mod token_tail;
// Batch A: the host-side model catalog (the ModelCatalogView seam) — always
// wired so an FFI-free serve answers ListModels with an honest empty list.
mod models;
// Batch B: the host-side def resolver (the MoteDefView seam) — always wired
// over the SAME content store the coordinator persists admitted defs into.
mod mote_defs;
mod provision;
#[cfg(feature = "embedded-worker")]
mod real_exec;
// PR-D: the run_inputs.db sidecar (the RunInputsStore seam) — the Invoke args
// captured at submit so a run recovered from ListRuns can pre-fill its recipe
// form and be re-invoked with edits ("Re-run with changes"). Rebuildable-to-
// EMPTY, off-journal, off-digest, off-identity.
mod run_inputs;
mod server;
// Batch C: the telemetry.db sidecar (the TelemetryView seam) — host-measured
// execution exhaust (wall-clock / model usage / fired tool), joined to the
// journal's Committed facts by a background tick. Rebuildable-to-EMPTY,
// off-journal, off-digest; the hot-path sink is bounded + fail-open. Needs the
// embedded worker (the executor wrapper measures the worker's mote loop).
#[cfg(feature = "embedded-worker")]
mod telemetry;
// UI-3: the host-side teams (MembershipView) + grants (GrantView) read seams + the
// idempotent demo-team seed.
mod teams;
mod tls;
// W1.A5: the host-side advisory toolscout view (manifests + bundle scoring +
// the lowering dry-run verdict). Always-on; display-only by construction.
mod toolscout;
// PR-6a: the host-side declarative tools registry admin (the ToolRegistryAdmin
// seam over the durable tools.db) + admission-time SSRF vetting of a
// RegisterTool's server_host. Always-on; off-journal, off-digest. DIALING the
// external MCP server + Connections + parallel fan-out are PR-6b/Cloud (D159).
mod tools;
// Batch A: the uploads.db sidecar (the UploadsLedger seam) — the PutContent
// audit rows + the uploads-scope authorized set. Rebuildable-to-empty (truth
// stays in the content store), off-journal, off-digest.
mod uploads;
#[cfg(feature = "embedded-worker")]
mod ws;

pub use auth::{DenyAll, DevAllowLocal, Principal, PrincipalResolver, TokenResolver};
pub use config::{
    Cli, ConsoleMode, GatewayConfig, TlsPaths, DEFAULT_CONSOLE_LISTEN, DEFAULT_CONTENT_MAX_BYTES,
    DEFAULT_MAX_LEASE, DEFAULT_WS_LISTEN, USAGE,
};
#[cfg(feature = "hnsw")]
pub use datasets::HostDatasetView;
#[cfg(all(feature = "hnsw", feature = "inference"))]
pub use datasets::HostEmbedder;
pub use error::GatewayError;
pub use live_tail::{GlobalLiveTailer, LiveTailer};
pub use provision::{
    DemoLibrary, HostRecipeBinder, HostRecipeCatalog, HostSignatureCatalog, HostWorkflowAuthor,
    DEMO_RECIPE_HANDLE, MODEL_RECIPE_HANDLE, PASSTHROUGH_DAG_HANDLE, REACT_AUTO_RECIPE_HANDLE,
    REACT_FS_RECIPE_HANDLE, REACT_RECIPE_HANDLE, VISION_RECIPE_HANDLE,
};
pub use server::{serve, start, RunningGateway};
pub use teams::{seed_workspace_team, HostGrantView, HostMembershipView, WORKSPACE_TEAM_HANDLE};

#[cfg(feature = "embedded-worker")]
pub use server::{default_executor_class, pure_run_request};
