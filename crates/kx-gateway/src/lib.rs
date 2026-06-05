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

mod auth;
mod config;
mod error;
mod live_tail;
mod provision;
mod server;
#[cfg(feature = "embedded-worker")]
mod ws;

pub use auth::{DenyAll, DevAllowLocal, Principal, PrincipalResolver, TokenResolver};
pub use config::{Cli, GatewayConfig, DEFAULT_MAX_LEASE, DEFAULT_WS_LISTEN, USAGE};
pub use error::GatewayError;
pub use live_tail::LiveTailer;
pub use provision::{DemoLibrary, HostRecipeBinder, HostSignatureCatalog, DEMO_RECIPE_HANDLE};
pub use server::{serve, start, RunningGateway};

#[cfg(feature = "embedded-worker")]
pub use server::{default_executor_class, demo_pure_result, demo_submit_run_request};
