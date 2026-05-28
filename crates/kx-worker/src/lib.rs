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

//! # kx-worker — the kortecx P2.3 worker (the `CoordinatorClient`)
//!
//! A worker is a gRPC **client** of the coordinator. It:
//!
//! 1. **registers** with the coordinator ([`Worker::register`] → `RegisterWorker`),
//!    declaring the executor backend it can run;
//! 2. **pulls** ready work ([`Worker::run_once`] → `LeaseWork`) — the coordinator
//!    returns ready PURE Motes runnable on this backend, each with its warrant;
//! 3. **runs** each Mote through the **hosted P1 executor** ([`kx_executor`],
//!    verbatim — which transitively hosts `kx-inference`);
//! 4. **proposes** the result back (`ReportCommit`); the coordinator is the SOLE
//!    journal writer (D13 / D40) and assigns the committed seq.
//!
//! ## Sole-writer, by construction
//!
//! The worker never writes the coordinator's journal. The executor's commit
//! protocol needs *a* [`kx_journal::Journal`] to append to, so the worker hands it
//! a **throwaway in-memory journal** (see `run`); only the `result_ref` crosses
//! back over gRPC. The metadata of the `ReportCommit` proposal is **re-derived**
//! from the held Mote + warrant (the canonical construction the coordinator also
//! uses) — not read back from the throwaway journal, which would lose the Mote's
//! parents and flatten its nd_class.
//!
//! ## Scope (P2.3)
//!
//! PURE Motes only. A PURE Mote has no real-world effect, so running it on the
//! worker and proposing its commit is sound. WORLD-MUTATING Motes need a durable
//! staged-intent RPC (so the coordinator records the `EffectStaged` recovery hint
//! before the effect fires) — deferred. Placement v2 (locality / GPU-slot) is P2.5;
//! worker provisioning / liveness-driven scale is P3 (D47).
//!
//! ## Thesis test
//!
//! `kx-scheduler` / `kx-executor` / `kx-inference` source is **unchanged** —
//! distribution is wiring. kx-worker is a new leaf crate; it adds the client and
//! the propose-don't-write glue, nothing more.

/// The gRPC schema + generated client (re-exported for callers that build wire
/// types directly, e.g. tests).
pub use kx_proto::proto;

mod client;
mod commit_builder;
mod error;
mod run;
mod worker;

pub use client::WorkerClient;
pub use error::WorkerError;
pub use worker::Worker;
