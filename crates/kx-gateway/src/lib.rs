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
// The Morphic Data Engine (campaign Batch 2): the durable serve-path capture
// projection (capture.db sidecar folded from the read-only journal handle).
// Always-on, off-truth-path; FFI-free (rusqlite is already in the closure).
mod capture;
mod error;
mod live_tail;
// PR-2d-2: the bundled deterministic stdio MCP tool's wiring (locate the bin,
// register the capability + the typed ToolDef). Behind `inference` (the react
// decode arm lives in the inference-gated executor; the MCP adapter is FFI-free).
#[cfg(feature = "inference")]
mod mcp_tool;
// AL1: the in-process model executor for `kx serve` (live LLM dispatch). Pulls
// the inference FFI, so it's behind the off-by-default `inference` feature.
#[cfg(feature = "inference")]
mod model_exec;
// Batch A: the host-side model catalog (the ModelCatalogView seam) — always
// wired so an FFI-free serve answers ListModels with an honest empty list.
mod models;
mod provision;
#[cfg(feature = "embedded-worker")]
mod real_exec;
mod server;
// UI-3: the host-side teams (MembershipView) + grants (GrantView) read seams + the
// idempotent demo-team seed.
mod teams;
mod tls;
// W1.A5: the host-side advisory toolscout view (manifests + bundle scoring +
// the lowering dry-run verdict). Always-on; display-only by construction.
mod toolscout;
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
pub use live_tail::LiveTailer;
pub use provision::{
    DemoLibrary, HostRecipeBinder, HostRecipeCatalog, HostSignatureCatalog, HostWorkflowAuthor,
    DEMO_RECIPE_HANDLE, EXEC_RECIPE_HANDLE, FANOUT_RECIPE_HANDLE, MODEL_RECIPE_HANDLE,
    REACT_RECIPE_HANDLE, VISION_RECIPE_HANDLE,
};
pub use server::{serve, start, RunningGateway};
pub use teams::{seed_demo_team, HostGrantView, HostMembershipView, DEMO_TEAM_HANDLE};

#[cfg(feature = "embedded-worker")]
pub use server::{default_executor_class, demo_pure_result, demo_submit_run_request};
