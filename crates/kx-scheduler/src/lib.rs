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
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-scheduler — dependency resolution + dispatch (P1.10)
//!
//! Per the P1.10 crate spec: *"decide what is ready and hand it to the executor —
//! **only by reading the projection***."*
//!
//! The scheduler is the placement-and-dispatch layer between the workflow author
//! (who submits Motes) and the executor (which runs them). On each [`Scheduler::tick`]
//! it asks the [`Projection`](kx_projection::Projection) which submitted Motes are
//! ready (parents all `Committed-and-not-Repudiated`), routes each through a
//! [`Placement`] policy, and hands the Mote + warrant to the
//! [`kx_executor::MoteExecutor`].
//!
//! ## Hard constraints (the P1.10 exit gate)
//!
//! - **Never writes the journal.** The crate has no `kx-journal` production
//!   dependency; the type literally cannot be imported in src/. (kx-journal is a
//!   dev-dep, used by tests to construct synthesized `JournalEntry` values to
//!   feed `Projection::fold`.)
//! - **Reads only the projection.** Orchestration decisions come from
//!   `Projection::ready_set()`, never from a direct journal read.
//! - **Does no execution.** Dispatch is delegated to `MoteExecutor::run`; the
//!   scheduler does not invoke inference, dispatch tools, or open the content store.
//! - **Does not message producers.** No async tasks, no channels, no actor
//!   plumbing. Dispatch is a synchronous call on each `tick`.
//!
//! ## What lives here
//!
//! - [`Scheduler`] — the per-run dispatch state machine (placement + pending map).
//! - [`Placement`] trait + [`LocalPlacement`] + [`RoundRobinPlacement`]
//!   (the "trivial local impl" plus the "second trivial impl" the DoD requires).
//! - [`WorkerId`] — opaque worker identifier returned by a placement.
//! - [`DispatchSummary`] / [`DispatchedMote`] — per-tick outcome shapes.
//! - [`SchedulerError`] — the scheduler's failure vocabulary.
//!
//! ## What does NOT live here
//!
//! - Journal append/read (executor's job).
//! - Input recomputation, parent `result_ref` lookup, or input-set rebuilds
//!   (the executor reads what it needs from the journal directly — see
//!   `kx-executor/src/lifecycle.rs` recovery path).
//! - Topology-decision child materialization (P1.11 — `is_topology_shaper`
//!   Motes are dispatched here exactly like any other Mote; child rebuilding
//!   happens in the projection on Commit per `projection.md` §5).
//! - Workers / process supervision (P1.13 binary owns that).

mod errors;
mod placement;
mod scheduler;
mod worker;

pub use errors::SchedulerError;
pub use placement::{LocalPlacement, Placement, RoundRobinPlacement};
pub use scheduler::{DispatchSummary, DispatchedMote, Scheduler};
pub use worker::WorkerId;

#[cfg(test)]
mod tests;
