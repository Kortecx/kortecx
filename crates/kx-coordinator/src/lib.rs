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
//! > **Phase: distributed (P2/P3).** The multi-node control plane — wiring on the
//! > same trait seams as the single-node core, *not* a rewrite of it. You do
//! > **not** need this crate to build, run, or understand single-node kortecx
//! > (`kx-runtime`). See the README (How it works) for the core-vs-distributed legend.
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
//! ## Scope (P2.2 + P2.3 dispatch)
//!
//! The control plane: registration / heartbeat / admission, the sole-writer commit
//! path, **and the `LeaseWork` dispatch surface** (P2.3) — a worker polls for ready
//! PURE Motes runnable on its backend, runs them, and proposes the result via
//! `ReportCommit`. Selection is trivial (ready ∩ PURE ∩ executor_class); placement
//! v2 (locality / GPU-slot awareness) is P2.5, and worker provisioning is P3 (D47).

pub use kx_projection::{MoteState, RunResolvedVersions};
pub use kx_proto::proto;
pub use kx_scheduler::WorkerId;

mod clock;
mod commit;
mod error;
mod nonce;
mod placement;
mod registry;
mod repudiation;
mod reschedule;
mod service;
mod state;

pub use clock::{Clock, SystemClock};
pub use error::CoordinatorError;
pub use kx_journal::RepudiationReason;
pub use nonce::{OsRandomNonce, RunNonceSource};
pub use registry::{
    is_live, InMemoryWorkerRegistry, RegistryError, WorkerRecord, WorkerRegistry, WorkerStatus,
    DEFAULT_LIVENESS_TIMEOUT,
};
pub use repudiation::{RepudiationError, RepudiationOutcome};
pub use service::CoordinatorService;
