#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-gateway-core — the external `KxGateway` backend (M8 / D120)
//!
//! > **Phase: client surfaces (M8).** The OSS backend behind the
//! > [`KxGateway`](kx_proto::proto::kx_gateway_server::KxGateway) service — the
//! > client-facing surface over the durable runtime. It is a **read-fold +
//! > propose-proxy**, not a new write path. See `ARCHITECTURE.md`.
//!
//! ## What it does
//!
//! - **Read-fold** — `GetProjection` folds the run's journal into a
//!   [`ProjectionView`](kx_proto::proto::ProjectionView) (render-a-run-as-a-DAG);
//!   `GetContent` returns a committed result by ref; `StreamEvents` is a
//!   resumable [`EventFrame`](kx_proto::proto::EventFrame) cursor. Every
//!   `MoteSnapshot` is **server-derived from the fold** — the client never
//!   computes a `MoteId` (SN-8 / D70).
//! - **Propose-proxy** — `SubmitRun` registers a run and submits its Motes
//!   through the [`RunSubmitter`] seam to the coordinator (the sole journal
//!   writer, D40). It returns only after the journaled `instance_id` (never acks
//!   ahead of the journal).
//!
//! ## The no-write wall (D120.5)
//!
//! gateway-core adds **no journal write path**, enforced at the type level
//! (Rule 5.2 — a write cannot type-check):
//! - reads go through [`JournalReader`] (no `append`) + [`ContentReader`]
//!   (no `put`); the [`ReadOnly`] newtype exposes only the read methods of any
//!   [`kx_journal::Journal`].
//! - submits go through [`RunSubmitter`] to the coordinator over gRPC — so
//!   gateway-core never links `kx-coordinator`/`kx-executor`/`kx-scheduler`/
//!   `kx-capture` (a dep-wall test pins it).
//!
//! Auth / ownership / multitenancy never appear in gateway-core signatures —
//! they live in `kx-cloud/gateway-auth` (D102.1). The `instance_id` is treated
//! as an opaque ownership ticket: a request is authorized iff the run's journal
//! names that `instance_id`; `GetContent` returns a **uniform** not-authorized
//! (no existence oracle).

mod error;
mod events;
mod reader;
mod service;
mod submit;
mod view;

pub use error::GatewayError;
pub use reader::{ContentReader, JournalReader, ReadOnly};
pub use service::GatewayService;
pub use submit::{
    RunSubmitter, SubmitMoteOutcome, SubmitStatus, SubmitterError, TonicCoordinatorSubmitter,
};
