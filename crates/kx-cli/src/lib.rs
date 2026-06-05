#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-cli — the unified `kx` command-line front end (R3 / D130)
//!
//! > **Phase: reachability (v0.1.0).** The single binary a human (or an agent)
//! > uses to touch the kortecx runtime. It is a thin dispatcher — every verb
//! > either forwards to an existing library or makes a gRPC call against the
//! > FROZEN [`KxGateway`](kx_proto::proto::kx_gateway_server::KxGateway) service.
//!
//! ## Verbs
//!
//! - **`run` / `replay` / `digest`** — forward VERBATIM to the
//!   [`kx_runtime`] engine (the projection-digest invariant is preserved; the
//!   CLI never re-implements the orchestrator).
//! - **`serve`** — forward to [`kx_gateway::serve`] (the embedded single-system
//!   server). `--listen` defaults to `127.0.0.1:50151` when omitted.
//! - **`invoke` / `submit` / `projection` / `content` / `events` /
//!   `signatures`** — gRPC client calls over the gateway. `invoke`/`submit`
//!   accept `--wait` to run-to-result (poll the projection, then fetch the
//!   committed content) and print one parseable object; every verb accepts
//!   `--json`.
//!
//! ## Discipline
//!
//! The CLI holds no journal handle and adds no write path (the coordinator is
//! the sole writer, D40). It never computes a `MoteId` / `instance_id` — those
//! are server-derived (SN-8); the CLI only echoes server bytes as hex and sends
//! a credential (a bearer token), never a claimed identity.

pub mod cli;
pub mod client;
pub mod error;
pub mod format;
pub mod hex;
pub mod verbs;
pub mod wait;

pub use cli::{run, Cli, USAGE};
pub use error::CliError;
