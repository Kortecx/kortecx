#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-invoke — inbound Mote-as-MCP execution (M8 / D121)
//!
//! > **Phase: client surfaces (M8).** Turns an *advertised* catalog snapshot into
//! > a *served*, guaranteed-execution run — the G3 enterprise-adoption unlock.
//! > A composition over the M7 catalog + the D120 gateway; it adds **no new
//! > journal write path** and never links the frozen trio.
//!
//! ## The flow (D121.1)
//!
//! ```text
//! external caller (party, handle, args)
//!   │
//!   ├─ Use authority  ── UseWarrantResolver (GovernedCatalog / GovernedFleet)  → effective warrant
//!   ├─ Read authority ── GovernedCatalog::resolve(handle)                      → recipe identity
//!   ├─ executable body ── BodyLedger::get_body(manifest_id)                    → WorkflowDef
//!   ├─ validate args  ── validate_args(free_params → InputSchema)  (fail-closed)
//!   ├─ bind args      ── WorkflowDef::bind_param per variable slot (fail-closed)
//!   ├─ compile        ── kx_workflow::compile                                  → Motes
//!   └─ narrow warrant ── intersect(effective, step) per Mote (no-widen)        → BoundRun
//!        │
//!        └─ execute ── RunSubmitter: RegisterRun + N×SubmitMote → the durable
//!                      spine drives StageThenCommit → Committed (exactly-once).
//! ```
//!
//! ## Guarantees
//!
//! - **Exactly-once-per-input.** Bound args flow (via `config_subset`) into each
//!   `MoteDef`, so distinct args yield distinct `MoteId`s (a fresh run) and
//!   identical args re-derive identical identities (the coordinator dedups → an
//!   idempotent re-invoke).
//! - **No privilege escalation (SN-8).** `Use` authority is resolved from the
//!   authoritative ledger via [`UseWarrantResolver`] — never a caller-supplied
//!   warrant; each Mote runs under `intersect(effective, step)` ⊆ both.
//! - **Fail-closed.** Untyped/unknown/over-range args, an unbound variable slot,
//!   an uncompilable body, or a missing grant all refuse before any submit.
//! - **No new write path.** Execution is `RegisterRun` + `SubmitMote` through the
//!   coordinator (the sole journal writer); kx-invoke links no journal writer.

mod bind;
mod error;
mod execute;

pub use bind::{bind_snapshot, BoundRun, UseWarrantResolver};
pub use error::InvokeError;
pub use execute::{execute, Submitted};
