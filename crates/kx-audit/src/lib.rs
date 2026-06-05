// SPDX-License-Identifier: Apache-2.0
//! `kx-audit` — the OFF-TRUTH-PATH, best-effort runtime audit sink (R4, D127 step 2.1).
//!
//! An audit trail makes the runtime's execution *observable*: a structured,
//! append-only stream of lifecycle events — a run started, it recovered N
//! committed Motes, a shaper derived children, a Mote was dispatched / committed /
//! failed / repudiated, the run completed with digest D and K-of-N committed. It
//! closes the "no observability" gap the architecture audit named, without ever
//! touching the durability/identity spine.
//!
//! ## What it records, and what it is NOT
//!
//! - **Join keys only.** Every [`AuditEvent`] field is an ALREADY-DERIVED value —
//!   a [`kx_mote::MoteId`] / [`kx_content::ContentRef`] hash, an
//!   [`kx_mote::NdClass`], an integer count, or the 32-byte product digest. The
//!   sink ECHOES runtime state; it NEVER recomputes a `MoteId` (SN-8). It records
//!   **no payload bytes, no model output, no warrant secrets** — only the hashes
//!   that join back to truth.
//! - **Operational telemetry, not the source of truth.** The journal is the
//!   durable truth and the product digest is recomputable from it; the audit log
//!   is a best-effort operational stream. On a hard crash the buffered tail may be
//!   lost — recompute from the journal.
//!
//! ## Invariants (the wall)
//!
//! - **Off the truth path.** Audit is NEVER journaled, is NEVER an input to a
//!   `MoteId`, and NEVER gates scheduling / promotion / eviction. Turning it on
//!   changes only what is *observed*, never the committed facts — so the canonical
//!   product digest `a6b5c679…` is byte-unchanged with audit on, off, or inspected.
//! - **Best-effort / non-fatal.** [`AuditSink::record`] returns unit: an audit
//!   write failure can NEVER propagate into the run loop. Failures are swallowed,
//!   logged via `tracing`, and counted via [`AuditSink::dropped`].
//! - **Timestamps never feed the digest.** [`AuditEvent`] carries **no time and no
//!   float**. A wall-clock stamp (epoch millis) + a monotonic sequence number are
//!   added by the sink at the *wire* layer ([`JsonlAuditSink`]) only, so time can
//!   never reach identity or the digest.
//! - **The dependency wall.** Guarantee-path crates (the frozen trio
//!   `kx-scheduler`/`kx-executor`/`kx-inference` + `kx-journal`/`kx-projection`/
//!   `kx-memoizer`) do **not** depend on this crate. The compiler enforces the
//!   direction every build; `tests/boundary.rs` is the tripwire.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod event;
mod in_memory;
mod jsonl;
mod sink;
mod wire;

pub use error::AuditError;
pub use event::{AuditEvent, DispatchKind};
pub use in_memory::InMemoryAuditSink;
pub use jsonl::JsonlAuditSink;
pub use sink::AuditSink;
