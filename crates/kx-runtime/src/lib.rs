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

//! # kx-runtime — the single-node kortecx runtime (P1.13)
//!
//! Wires the scheduler, executor, capability broker, resource manager, journal,
//! and content store into ONE process. It is the local dev runtime **and** the
//! primary public proof of the novel kortecx claim:
//!
//! > *A committed non-deterministic, world-mutating step is a fact; recovery
//! > re-reads what it did, never re-runs it.*
//!
//! ## The seam
//!
//! The scheduler and executor never message each other directly. The scheduler
//! holds submitted Motes; the ready set is read from a [`kx_projection::Projection`] folded
//! from the journal; the executor commits results back to the journal. The
//! journal is the only synchronization substrate (CLAUDE.md core principle 1).
//!
//! ## The canonical demo workflow ([`workflow`])
//!
//! ```text
//! M1 PURE → { M2 READ-ONLY-NONDET, Wstc WM-StageThenCommit, M3 WM-ValidateThenCommit → M3c PURE critic }
//! ```
//!
//! Every body executes deterministically (`TestMoteExecutor::deterministic` +
//! the deterministic [`broker::DemoBroker`]) so the journal is byte-identical
//! across runs, processes, and machines — the precondition for the
//! kill-and-replay proof.
//!
//! ## Kill-and-replay (the exit-gate proof, exercised by `kx-p1-demo`)
//!
//! Two crash scenarios ([`crash::CrashPoint`]), each injected as a real
//! `process::abort` over an on-disk journal, then recovered by a fresh process:
//!
//! - **`pre-commit-stc`** — kill mid `StageThenCommit` (after `EffectStaged` +
//!   broker stage, before `Committed`). Recovery re-dispatches; the broker's
//!   idempotency-key dedup makes the external effect exactly-once.
//! - **`post-commit-vtc`** — kill the instant the `ValidateThenCommit` Mote's
//!   `Committed` is durable. Recovery RE-READS the committed result, never
//!   re-running the effect — the headline novel claim.
//!
//! Both prove: (a) the final committed-result set is bit-identical to a clean
//! run, (b) no Mote has more than one `Committed` entry, (c) a fresh process
//! folding only the journal reconstructs a bit-identical projection.

pub mod audit_sink;
pub mod broker;
pub mod capture_sink;
pub mod checkpoint_io;
pub mod config;
pub mod crash;
pub mod digest;
pub mod engine;
pub mod error;
pub mod failure_policy;
pub mod migrate;
pub mod snapshot_sink;
pub mod topology;
pub mod workflow;

pub use audit_sink::RuntimeAuditSink;
pub use capture_sink::CaptureSink;
pub use config::{Mode, RuntimeConfig};
pub use crash::CrashPoint;
pub use digest::{digest_journal, digest_projection, ProjectionDigest};
pub use engine::{
    canonical_mote_ids, canonical_targets, digest_only, run, run_with_capture, run_with_seams,
    RunOutcome,
};
pub use error::RuntimeError;
pub use failure_policy::FailurePolicy;
// Re-export the audit vocabulary so callers (CLI / harness / future gateway) use
// the runtime facade, mirroring how `CaptureSink` fronts `kx-capture`.
pub use kx_audit::{AuditEvent, AuditSink, DispatchKind, InMemoryAuditSink, JsonlAuditSink};
pub use migrate::migrate_and_verify;
pub use snapshot_sink::SnapshotSink;
pub use topology::{decode_topology_decision, TopologyProvider, TopologyProviderError};
pub use workflow::DemoWorkflow;
