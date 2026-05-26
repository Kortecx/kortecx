#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::range_plus_one,
    clippy::elidable_lifetime_names
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-projection — the log's read-side fold
//!
//! The journal (`kx_journal`) is the durable truth; **the projection is its in-memory
//! read view**. Folding `JournalEntry` records in `seq` order produces the per-Mote
//! state (Pending → Scheduled → Committed | Failed → Repudiated), the dependency graph,
//! the `ready_set` the scheduler consumes, and the `transitive_consumers` set the
//! poison-cascade (D22) walks.
//!
//! ## Hard rules (from `projection.md`)
//!
//! - **Pure function of the log.** Two folds of the same log prefix produce
//!   bit-equivalent state. The projection is never durably stored as a mutable graph;
//!   on restart it is re-folded from the log.
//! - **Read-only against the journal.** `kx-projection` never calls
//!   `Journal::append`. Single-writer-per-run (D13) is preserved by construction —
//!   `kx-projection` does not depend on `Journal` as a *mut* surface.
//! - **Snapshot isolation** (D16). Each `snapshot()` returns a stable point-in-time
//!   view. Subsequent log appends are not visible mid-read.
//! - **Cycle tolerant.** Cycles in the dependency graph do not crash, hang, or
//!   corrupt the fold. Traversals (`transitive_consumers`) use visited-sets.
//!
//! ## Topology-shaper materialization is deferred to P1.11
//!
//! `projection.md` §5 specifies that the projection materializes shaper-declared
//! children when a `Committed` entry's Mote has `is_topology_shaper == true`. This
//! requires decoding a `TopologyDecision` payload from the content store. P1.5 lays
//! the framework (the `MoteInfo` carries metadata about each Mote; new children can be
//! added to the graph mid-fold); P1.11 wires the content-store-side decoder and the
//! child-edge materialization algorithm.
//!
//! ## 3c promotion state — P1 default
//!
//! Per D18, `promotion_state` defaults to `NotApplicable` for non-WORLD-MUTATING and
//! for WORLD-MUTATING-without-observable-critic-relationship. Until the executor
//! (P1.9) wires a `MoteDef` lookup into the projection so the projection can read
//! each Committed Mote's `critic_for`, the projection treats all WORLD-MUTATING Motes
//! as `NotApplicable` — matching D18's "3a/3b workflows run normally in P1; 3c
//! workflows are expressible but unsafe until P0.8 binds the critic." The
//! `promotion_state` method is exposed today; full 3c behavior activates when the
//! executor populates the `MoteDef` registry the projection consults.
//!
//! ## What lives here
//!
//! - [`MoteState`] — the per-identity state machine (§4 of `projection.md`).
//! - [`PromotionState`] — Promoted / Unpromoted / NotApplicable.
//! - [`RegisterMote`] — workflow-author declaration of a Mote's parents + properties
//!   before any journal entry exists for it.
//! - [`Projection`] — the in-memory fold state; mutate via `register_mote` + `fold`.
//! - [`Snapshot`] — an immutable point-in-time view of the projection.
//! - The 7-method read API surface (`state_of`, `parents_of`, `children_of`,
//!   `transitive_consumers`, `result_ref_of`, `ready_set`, `promotion_state`).

mod enums;
mod errors;
mod helpers;
mod projection;
mod register;
mod snapshot;
mod state;

pub use enums::{AnomalyKind, MoteState, PromotionState};
pub use errors::ProjectionError;
pub use projection::Projection;
pub use register::RegisterMote;
pub use snapshot::Snapshot;

#[cfg(test)]
mod tests;
