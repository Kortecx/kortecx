//! Public state and outcome enums for projection queries.

/// Per-Mote state, derived from the log via the precedence rules in `projection.md`
/// §4. A Mote registered but with no journal entry yet is [`crate::MoteState::Pending`].
///
/// **v2 (PR 7) adds [`crate::MoteState::Inconsistent`]** — the cell-8 anomaly state for
/// `EffectStaged` + `Repudiated` without an intervening `Committed`. Per STEP 5.3
/// of PR 4.5: the fold does NOT abort on this anomaly; it quarantines the affected
/// Mote and surfaces it via [`crate::Projection::anomaly_motes`] so an operator decides
/// recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MoteState {
    /// Workflow-declared but no journal entry yet.
    Pending,
    /// At least one `Proposed` entry exists; no `Committed`, no later `Failed`.
    Scheduled,
    /// A `Committed` entry exists and has not been Repudiated.
    Committed,
    /// At least one `Failed` entry exists; no later `Proposed`, no `Committed`.
    /// In v2, this includes the **terminal failure** case (a `Failed` whose
    /// `reason_class` is NOT pre-commit-crash, paired with an `EffectStaged` —
    /// cell 5 of the 9-cell cross-product). Terminal failures forbid
    /// re-dispatch; consult [`crate::Projection::can_redispatch_world_effect`].
    Failed,
    /// A `Committed` entry exists AND a `Repudiated` entry targeting it has landed.
    Repudiated,
    /// **v2 (PR 7): cell-8 anomaly.** An `EffectStaged` entry exists for the Mote
    /// AND a `Repudiated` entry references it WITHOUT an intervening `Committed`.
    /// Repudiated normally targets a Committed; an EffectStaged-then-Repudiated-
    /// without-Committed sequence is a journal-consistency error per STEP 5.3.
    /// Surfaced via [`crate::Projection::anomaly_motes`]; never re-dispatched.
    Inconsistent,
}

/// Categorical anomaly kind surfaced by [`crate::Projection::anomaly_motes`].
///
/// **v2 (PR 7).** Extensible-by-additive-variants: when a new fold cell anomaly
/// becomes possible (e.g., from a fifth journal kind), it extends this enum rather
/// than adding another `MoteState` variant — keeps state semantically minimal
/// while diagnostics remain expressive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnomalyKind {
    /// **Cell 8** of the 9-cell cross-product (`journal-txn.md` §"Recovery fold
    /// semantics"): an `EffectStaged` entry was folded for this Mote, then a
    /// `Repudiated` entry referencing it was folded, but no `Committed` was ever
    /// folded in between. Repudiated targets a Committed that doesn't exist; the
    /// fold quarantines the Mote (sets `info.inconsistent`) rather than aborting.
    EffectStagedThenRepudiatedNoCommitted,
    /// **Recovery-time quarantine (M2.3b, D105.4 / D65).** A staged-uncommitted
    /// at-most-once (`IdempotencyClass::AtLeastOnce`) WORLD-MUTATING effect could
    /// not be safely re-dispatched (no closing mechanism) and the capability does
    /// not support compensation, so recovery quarantined it: a terminal
    /// `Failed { reason_class: QuarantinedAtLeastOnce }` was appended (the Mote is
    /// `MoteState::Failed`, never re-dispatched). Surfaced here for operator review.
    QuarantinedAtLeastOnceEffect,
}

/// 3c (validate-then-commit) promotion state, per D18 + D20.
///
/// - `NotApplicable`: PURE / READ-ONLY-NONDET, OR WORLD-MUTATING with no
///   observable critic relationship in the projection (3a / 3b — effective on commit).
/// - `Unpromoted`: WORLD-MUTATING with an observed critic relationship that has not
///   yet committed `Valid` (the critic hasn't committed at all, or committed `Invalid`).
/// - `Promoted`: WORLD-MUTATING with an observed critic that committed `Valid`.
///
/// **P1 default behavior.** The projection observes no critic-of-producer
/// relationships until the executor (P1.9) wires a `MoteDef` lookup. Until then, all
/// Motes return `NotApplicable` regardless of nd_class — matching the D18 P1 default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PromotionState {
    /// Effective on commit — no critic gate applies.
    NotApplicable,
    /// 3c WORLD-MUTATING with an unsatisfied critic gate (P1: unreachable).
    Unpromoted,
    /// 3c WORLD-MUTATING with a satisfied critic gate (P1: unreachable).
    Promoted,
}
