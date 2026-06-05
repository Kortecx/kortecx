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

//! # kx-worker — the kortecx worker (the `CoordinatorClient`)
//!
//! > **Phase: distributed (P2/P3).** Part of the multi-node layer — wiring on the
//! > same trait seams as the single-node core, *not* a rewrite of it. You do
//! > **not** need this crate to build, run, or understand single-node kortecx
//! > (`kx-runtime`). See the README (How it works) for the core-vs-distributed legend.
//!
//! A worker is a gRPC **client** of the coordinator. It:
//!
//! 1. **registers** with the coordinator ([`Worker::register`] → `RegisterWorker`),
//!    declaring the executor backend it can run;
//! 2. **pulls** ready work ([`Worker::run_once`] → `LeaseWork`) — the coordinator
//!    returns ready Motes runnable on this backend, each with its warrant;
//! 3. **dispatches** each Mote: PURE recomputes through the **hosted P1 executor**
//!    ([`kx_executor`], verbatim — which transitively hosts `kx-inference`);
//!    WORLD-MUTATING / READ-ONLY-NONDET stages-then-fires through the capability
//!    broker (P3.6b, D58 — see `run_wm`);
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
//! ## Scope
//!
//! PURE recomputes locally and proposes its commit (sound: no real-world effect).
//! **P3.6b (D58)** adds WORLD-MUTATING / READ-ONLY-NONDET dispatch: the worker holds
//! an `Arc<dyn CapabilityBroker>` and drives **stage→fire→commit** — for
//! `StageThenCommit`, `ReportEffectStaged` records the `EffectStaged` recovery hint
//! through the sole writer (D40) before the broker fires, then `ReportCommit` proposes
//! the staged ref. Worker-death after stage re-leases (D57) and the tool-boundary
//! idempotency key makes the re-dispatch a no-op (exactly-once, D58 §7). The VTC
//! critic is an ordinary DAG Mote the coordinator leases once the producer commits —
//! the worker does not schedule it (§6). Placement v2 (locality / GPU-slot) is P2.5.
//!
//! From **P3.1** the worker also runs a background **liveness heartbeat**
//! ([`Worker::spawn_heartbeat`]) so an idle worker stays live in the coordinator's
//! registry (worker-death detection is heartbeat-timeout based). Worker provisioning
//! / liveness-driven scale-to-zero remains P3+ (D47).
//!
//! ## Thesis test
//!
//! `kx-scheduler` / `kx-executor` / `kx-inference` source is **unchanged** —
//! distribution is wiring. kx-worker is a new leaf crate; it adds the client and
//! the propose-don't-write glue, nothing more. The P3.6b WORLD-MUTATING path holds an
//! `Arc<dyn CapabilityBroker>` and re-implements the stage→fire→commit ORDERING via
//! RPCs, but never calls `run_wm_mote` / `StandardCommitProtocol` (those write a
//! journal the worker doesn't own) — so the engine is still not forked.

/// The gRPC schema + generated client (re-exported for callers that build wire
/// types directly, e.g. tests).
pub use kx_proto::proto;

mod client;
mod commit_builder;
mod error;
mod read_model;
mod run;
mod run_wm;
mod worker;

pub use client::WorkerClient;
pub use error::WorkerError;
pub use worker::{Worker, DEFAULT_HEARTBEAT_CADENCE};
