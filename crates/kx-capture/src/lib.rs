//! `kx-capture` — the OPT-IN, OFF-TRUTH-PATH step-capture projection.
//!
//! An atomic Mote IS the agent in the kortecx runtime. This crate captures a
//! Mote's *step* — its input/context, its committed output (the **action**), and
//! optionally its reasoning + thinking — as a disposable projection keyed by
//! `MoteId`, with every payload content-addressed in blob storage (the
//! `kx-content` store). It exists so a user who opts in can keep and reuse the
//! full step-level exhaust of an agent run; the runtime itself never needs it.
//!
//! ## What is truth, and what is exhaust
//!
//! - **Truth (always):** the committed **action** — a Mote's `result_ref` on the
//!   journal + content store. The runtime reuses *that* (the recipe / the action),
//!   exact-`MoteId`-keyed, never fuzzy. Dropping this crate loses **no** truth.
//! - **Exhaust (opt-in):** conversations / reasoning / thinking captured per step.
//!   This is a [`CaptureScope::Full`] opt-in, stored as a disposable projection
//!   joined back to truth by content hash.
//!
//! ## Invariants (the wall)
//!
//! - **Off the truth path.** This projection is NEVER journaled, is NEVER an
//!   input to a `MoteId`, and NEVER gates scheduling/promotion/eviction. It is a
//!   pure projection over content-addressed blobs.
//! - **Reuse the action, never the thinking.** The only result-reuse path is the
//!   exact-`MoteId`-keyed memoizer over committed actions; nothing here feeds it.
//! - **No floats on any identity path.** Capture metadata is integer-only.
//! - **The dependency wall.** Guarantee-path crates (`kx-mote` core, `kx-journal`,
//!   `kx-scheduler`, `kx-executor`, `kx-projection`) do **not** depend on this
//!   crate. The compiler enforces the direction every build; `tests/boundary.rs`
//!   is the tripwire. Moving capture "closer" to the executor for convenience is
//!   itself the boundary violation.
//! - **Safe default.** [`CaptureScope::ActionsOnly`] is the default; retaining
//!   reasoning/thinking is opt-in-with-consent ([`CaptureScope::Full`]).

mod record;
mod scope;
mod store;

pub use record::StepRecord;
pub use scope::{CaptureConsent, CaptureScope};
pub use store::InMemoryCaptureStore;
