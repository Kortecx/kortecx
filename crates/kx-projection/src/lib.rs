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
//! ## Recovery: how the fold survives a crash mid-effect
//!
//! Recovery is just re-folding the log. The interesting case is a crash *inside*
//! a world-mutating step. Under the `StageThenCommit` protocol the executor writes
//! an `EffectStaged` fact **before** it touches the world and a `Committed` fact
//! **after**; the gap between them is the crash window. On re-fold, the
//! combination of facts observed for a Mote decides whether re-dispatch is safe —
//! computed by `can_redispatch_world_effect` (see `state.rs`):
//!
//! | Observed on re-fold | Meaning | Recovery decision |
//! |---|---|---|
//! | `Committed` present | the effect landed durably | **done** — serve the result, NEVER re-dispatch |
//! | `EffectStaged`, no `Committed`, `Failed` (timed-out / worker-crashed) | a *pre-commit* crash — the effect may not have landed | **re-dispatch** — the tool's idempotency closes the window |
//! | `EffectStaged`, no `Committed`, `Failed` (terminal: refused / validator-rejected / …) | a *terminal* failure | **do NOT re-dispatch** — re-firing could double-apply |
//! | `EffectStaged`, no `Committed`, nothing terminal | in-flight when the crash hit | **re-dispatch** permitted |
//! | `EffectStaged` + `Repudiated`, no `Committed` | a fact-ordering anomaly | **quarantine** (`Inconsistent`) — surfaced for an operator |
//! | no `EffectStaged`, no `Committed` | not yet attempted (or the effect carries its own idempotency) | ordinary scheduling |
//!
//! **Load-bearing ordering invariant:** a *terminal* failure is checked **before**
//! the in-flight (`EffectStaged`) case. Swapping them would let a terminally-failed
//! world effect be re-dispatched — re-opening the double-fire window. This ordering
//! is enforced in `State::state_of_id` / `can_redispatch_world_effect_id` and pinned
//! by the cross-product regression tests in `tests/cross_product.rs`. The flags it
//! reads (`effect_staged_observed`, `terminal_failure_observed`, `inconsistent`)
//! are **monotonic-true**: once set during a fold they are never reset, so a longer
//! prefix can only narrow re-dispatch, never widen it.
//!
//! ## Topology-shaper materialization (P1.11 / D48 + D49)
//!
//! `projection.md` §5/§6/§7 (post-D48/D49 amendment) specify that the projection
//! materializes shaper-declared children when a `Committed` entry's Mote has
//! `is_topology_shaper == true`. P1.11 wires this via the [`TopologyMaterializer`]
//! seam: callers pass a materializer to [`Projection::with_materializer`] and the
//! fold invokes it on every `Committed` entry. The materializer (typically
//! [`DefaultTopologyMaterializer`]) holds a [`kx_content::ContentStore`] + a
//! [`MoteDefRegistry`] + a [`ChildResolver`]; it reads the shaper's
//! `TopologyDecision` payload, resolves each child's full `MoteDef`
//! ([`InheritFromShaperResolver`] is the OSS default), derives identity per D49
//! (`shaper.MoteId.bytes ‖ child_index_u32_le`), and yields a [`RegisterMote`]
//! per child for the projection to register.
//!
//! Projections constructed via [`Projection::new`] have NO materializer and
//! silently skip shaper materialization — this preserves the existing test
//! surface where no topology is exercised. Production callers MUST construct
//! via [`Projection::with_materializer`].
//!
//! The R49 cold-re-fold property (every child `MoteId` is a deterministic
//! function of the shaper's committed entry + the child's index in
//! `TopologyDecision.children`) is **test-pinned, not prose-pinned** — see
//! `tests/cold_refold_topology.rs` for the P1+P2+P3+P4 verification.
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
//! - [`FoldCheckpoint`] — a **discardable** durable snapshot of the folded state
//!   (D92(b), M2.2). Lets cold recovery fold `(checkpoint_offset, current]`
//!   instead of `(0, current]` via [`Projection::from_journal_with_checkpoint`].
//!   Never authoritative — a corrupt/stale/wrong-run checkpoint is silently
//!   discarded and the full fold runs.
//! - The 7-method read API surface (`state_of`, `parents_of`, `children_of`,
//!   `transitive_consumers`, `result_ref_of`, `ready_set`, `promotion_state`).

mod checkpoint;
mod child_resolver;
#[cfg(test)]
mod critic_gate_tests;
mod enums;
mod errors;
mod helpers;
mod materializer;
mod mote_def_registry;
mod projection;
pub mod promotion;
mod register;
mod run_metadata;
mod snapshot;
mod state;

pub use checkpoint::{
    CheckpointError, CheckpointOutcome, FoldCheckpoint, FullFoldReason, CURRENT_FORMAT_VERSION,
    PAYLOAD_CODEC,
};
pub use child_resolver::{ChildResolver, InheritFromShaperResolver};
pub use enums::{AnomalyKind, MoteState, PromotionState};
pub use errors::ProjectionError;
pub use materializer::{derive_child_identity, DefaultTopologyMaterializer, TopologyMaterializer};
pub use mote_def_registry::{InMemoryMoteDefRegistry, MoteDefRegistry};
pub use projection::Projection;
pub use promotion::{ContentStoreVerdicts, VerdictLookup};
pub use register::RegisterMote;
pub use run_metadata::{fold_run_metadata, RunMetadata, RunMetadataFold, RunRecord};
pub use snapshot::Snapshot;
pub use state::{ReactRoundRecord, ReplanRoundRecord, RunResolvedVersions};

#[cfg(test)]
mod tests;
