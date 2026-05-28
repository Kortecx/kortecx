#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-coordinator — the kortecx P2.2 coordinator service (the control plane)
//!
//! The coordinator is the hub of a distributed run. It:
//!
//! 1. **hosts the P1 scheduler verbatim** — submitted Motes are registered through
//!    [`kx_scheduler::Scheduler::submit`], exactly as the single-node engine does
//!    (the P2 thesis test: distribution must not change `kx-scheduler` source);
//! 2. **owns the worker registry behind a trait** ([`WorkerRegistry`]) — register
//!    / heartbeat / admission for `ReportCommit`;
//! 3. **is the sole journal writer per run** (D13 / D40) — remote workers PROPOSE
//!    commits via the [`SubmitMote`](proto::coordinator_server::Coordinator::submit_mote)
//!    /
//!    [`ReportCommit`](proto::coordinator_server::Coordinator::report_commit) RPCs;
//!    the coordinator assembles the canonical `Committed` entry and appends it (the
//!    only place a seq is assigned).
//!
//! It implements the [`kx_proto`] gRPC `Coordinator` service. The journal,
//! projection, and scheduler live on one owner thread (see `state`), which keeps
//! the gRPC service `Send + Sync` and makes single-writer structural.
//!
//! ## Scope (P2.2)
//!
//! Passive control plane: the four RPCs, the registry, and the sole-writer commit
//! path. Active dispatch — pushing/pulling ready Motes to workers — is **P2.3**
//! (`kx-worker`), and placement v2 is P2.5.

pub use kx_projection::MoteState;
pub use kx_proto::proto;
pub use kx_scheduler::WorkerId;

mod commit;
mod error;
mod registry;
mod service;
mod state;

pub use error::CoordinatorError;
pub use registry::{InMemoryWorkerRegistry, RegistryError, WorkerRecord, WorkerRegistry};
pub use service::CoordinatorService;
