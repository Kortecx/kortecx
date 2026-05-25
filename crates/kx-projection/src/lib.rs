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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kx_content::ContentRef;
use kx_journal::{Journal, JournalEntry, ParentEntry};
use kx_mote::{EdgeKind, EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Public state / outcome types
// ---------------------------------------------------------------------------

/// Per-Mote state, derived from the log via the precedence rules in `projection.md`
/// §4. A Mote registered but with no journal entry yet is [`MoteState::Pending`].
///
/// **v2 (PR 7) adds [`MoteState::Inconsistent`]** — the cell-8 anomaly state for
/// `EffectStaged` + `Repudiated` without an intervening `Committed`. Per STEP 5.3
/// of PR 4.5: the fold does NOT abort on this anomaly; it quarantines the affected
/// Mote and surfaces it via [`Projection::anomaly_motes`] so an operator decides
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
    /// re-dispatch; consult [`Projection::can_redispatch_world_effect`].
    Failed,
    /// A `Committed` entry exists AND a `Repudiated` entry targeting it has landed.
    Repudiated,
    /// **v2 (PR 7): cell-8 anomaly.** An `EffectStaged` entry exists for the Mote
    /// AND a `Repudiated` entry references it WITHOUT an intervening `Committed`.
    /// Repudiated normally targets a Committed; an EffectStaged-then-Repudiated-
    /// without-Committed sequence is a journal-consistency error per STEP 5.3.
    /// Surfaced via [`Projection::anomaly_motes`]; never re-dispatched.
    Inconsistent,
}

/// Categorical anomaly kind surfaced by [`Projection::anomaly_motes`].
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

/// Workflow-author declaration of a Mote before any journal entry exists for it.
///
/// Submitted at workflow-compile time by the executor (P1.9) or directly by tests.
/// Adds the Mote to the projection's "expected" set so it appears as
/// [`MoteState::Pending`] in `state_of` and is eligible for `ready_set` once its
/// parents commit.
#[derive(Debug, Clone)]
pub struct RegisterMote {
    /// The Mote's identity.
    pub mote_id: MoteId,
    /// The Mote's non-determinism tag (drives recovery semantics, scheduling priority).
    pub nd_class: NdClass,
    /// Which effect pattern this Mote uses (D20). Determines whether `ready_set`
    /// gates downstream consumers on a critic verdict (3c only).
    pub effect_pattern: EffectPattern,
    /// If this Mote is a critic of another Mote, the producer's identity.
    pub critic_for: Option<MoteId>,
    /// If this Mote is a topology shaper, set to `true` (P1.11 will use this).
    pub is_topology_shaper: bool,
    /// The Mote's declared parents with edge metadata. Up to 4 inline; spill heap.
    pub parents: SmallVec<[ParentRef; 4]>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors raised by [`Projection`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// The fold detected two `Committed` entries for the same `MoteId` — this is
    /// a journal-layer bug (the dedupe-by-key path failed). Surfaced loudly per
    /// `projection.md` §4 ("if it does, that is a journal-impl bug, not a precedence
    /// question — surface it loudly").
    #[error("two Committed entries for MoteId {0} (journal dedupe-by-key bug)")]
    DuplicateCommitted(MoteId),

    /// Wraps an underlying [`kx_journal::JournalError`] surfaced while folding from
    /// a `Journal` instance.
    #[error(transparent)]
    Journal(#[from] kx_journal::JournalError),
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CommittedInfo {
    seq: u64,
    result_ref: ContentRef,
    nondeterminism: NdClass,
    parents_in_entry: SmallVec<[ParentEntry; 4]>,
    /// The warrant under which this commit was performed. NEW in v2 (D36).
    /// Stored so consumers (executor recovery, audit log walkers) can read
    /// it via the projection's API without re-decoding the journal entry.
    /// Not yet read in P1.5; will be consumed by P1.9's submission-time
    /// refusal predicates.
    #[allow(dead_code)]
    warrant_ref: ContentRef,
    /// Retained for the D22 `list_committed_by_mote_def_hash`-driven cascade
    /// (operator-level definition repudiation surfaces the def_hash; consumers
    /// reach for it here when constructing cascade sets). Not yet read in P1.5;
    /// will be consumed by the executor-side flow that initiates definition-
    /// level cascades.
    #[allow(dead_code)]
    mote_def_hash: MoteDefHash,
    repudiated: bool,
}

// MoteInfo has 5 bools because the 9-cell recovery cross-product needs each
// flag's semantics distinct: `has_proposed` + `failed_pending_reattempt` are
// non-monotonic per-attempt markers; `effect_staged_observed` +
// `terminal_failure_observed` + `inconsistent` are prefix-monotonic-true
// recovery-contract flags. Consolidating to a bitfield or enum would obscure
// the per-flag invariants the fold + state_of_id depend on. Each flag's
// reset/no-reset semantics is named at its field-level doc comment.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
struct MoteInfo {
    /// Workflow-author-declared properties (when registered).
    declared: Option<DeclaredInfo>,
    /// Committed-entry info (when a `Committed` entry has been folded).
    committed: Option<CommittedInfo>,
    /// `true` if at least one `Proposed` entry has been folded for this MoteId.
    has_proposed: bool,
    /// `true` if at least one `Failed` entry has been folded with no later `Proposed`.
    /// We track this directly because `Failed → Proposed` is a valid sequence
    /// (`mote.md` §7 + `journal-entry.md` §7.5). **NOT prefix-monotonic** —
    /// reset to `false` by a subsequent `Proposed`. Distinct from
    /// `terminal_failure_observed` below.
    failed_pending_reattempt: bool,
    /// **v2 (PR 7).** `true` if at least one `EffectStaged` entry has been
    /// folded for this MoteId. **Prefix-monotonic-true** — never reset by any
    /// fold branch. Set in the `EffectStaged` arm.
    effect_staged_observed: bool,
    /// **v2 (PR 7).** `true` if at least one `Failed` entry has been folded
    /// with a terminal `reason_class` (i.e., NOT pre-commit-crash per
    /// [`kx_journal::is_pre_commit_crash`]). **Prefix-monotonic-true** — never
    /// reset. This is the LOAD-BEARING flag that closes the cell-5 WM
    /// double-effect hazard per STEP 5.2 of PR 4.5.
    terminal_failure_observed: bool,
    /// **v2 (PR 7).** `true` if the cell-8 anomaly was observed: a
    /// `Repudiated` entry referenced this Mote while an `EffectStaged` had
    /// been folded but no `Committed` was ever folded in between.
    /// **Prefix-monotonic-true** — never reset. Quarantines the Mote per
    /// STEP 5.3.
    inconsistent: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
// `nd_class`, `effect_pattern`, `critic_for`, `is_topology_shaper` are stored at
// registration but unread in P1.5. P1.9 (the executor) consumes them via a MoteDef
// registry lookup to compute the full 3c promotion behavior + the topology-shaper
// materialization at P1.11.
struct DeclaredInfo {
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    is_topology_shaper: bool,
    parents: SmallVec<[ParentRef; 4]>,
}

#[derive(Debug, Clone, Default)]
struct State {
    /// Per-MoteId info — declared, committed, and any in-flight state.
    motes: BTreeMap<MoteId, MoteInfo>,
    /// child → parents adjacency (derived from `MoteInfo.declared.parents` or
    /// `committed.parents_in_entry`). Computed by `parents_of`.
    /// We also maintain a reverse index for fast `children_of`.
    children: BTreeMap<MoteId, Vec<(MoteId, EdgeMeta)>>,
    /// The largest `seq` value applied so far.
    last_seq: u64,
}

impl State {
    fn moteinfo_mut(&mut self, id: &MoteId) -> &mut MoteInfo {
        self.motes.entry(*id).or_default()
    }

    /// Compute the per-identity state per `projection.md` §4 (v2 derivation
    /// per STEP 5.1 of PR 4.5).
    ///
    /// **Terminal-before-Staged ordering invariant (LOAD-BEARING).** Per STEP
    /// 5.1: `terminal_failure_observed` MUST be checked BEFORE
    /// `effect_staged_observed`. Swapping reopens the WM double-effect window
    /// (cell 5 flips from "Failed-terminal — do NOT redispatch" to
    /// "Pending-in-flight — OK to redispatch"). Reordering is a recovery-
    /// correctness regression and is forbidden. Regression test:
    /// `kx-projection/tests/cross_product.rs::
    /// cell_5_terminal_failure_under_effect_staged_no_redispatch`.
    ///
    /// **v2 derivation order**:
    /// 1. `info.inconsistent` → `Inconsistent` (highest priority; cell-8 anomaly)
    /// 2. `committed.is_some() && repudiated` → `Repudiated`
    /// 3. `committed.is_some()` → `Committed`
    /// 4. `info.terminal_failure_observed` → `Failed` ← MUST precede branch 5
    /// 5. `info.effect_staged_observed` → `Pending` (in-flight; redispatch OK)
    /// 6. `info.failed_pending_reattempt` → `Failed` (retry-allowed)
    /// 7. `info.has_proposed` → `Scheduled`
    /// 8. else → `Pending`
    fn state_of_id(&self, id: &MoteId) -> MoteState {
        match self.motes.get(id) {
            None => MoteState::Pending,
            Some(info) => {
                // 1. Anomaly takes priority over every other state (STEP 5.3).
                if info.inconsistent {
                    MoteState::Inconsistent
                } else if let Some(c) = &info.committed {
                    if c.repudiated {
                        MoteState::Repudiated
                    } else {
                        MoteState::Committed
                    }
                }
                // INVARIANT: terminal_failure_observed MUST be checked BEFORE
                // effect_staged_observed. Swapping reopens the WM double-effect
                // window (see projection.md §"Terminal-before-Staged ordering
                // invariant" and journal-txn.md fold cross-product cell 5).
                // Regression test:
                //   kx-projection/tests/cross_product.rs::
                //   cell_5_terminal_failure_under_effect_staged_no_redispatch
                else if info.terminal_failure_observed {
                    MoteState::Failed
                } else if info.effect_staged_observed {
                    // Cells 2 + 3: EffectStaged with no Committed and no
                    // terminal Failed → in-flight; redispatch permitted by
                    // can_redispatch_world_effect().
                    MoteState::Pending
                } else if info.failed_pending_reattempt {
                    MoteState::Failed
                } else if info.has_proposed {
                    MoteState::Scheduled
                } else {
                    MoteState::Pending
                }
            }
        }
    }

    /// `can_redispatch_world_effect` predicate (v2 / STEP 5.3 / R-13).
    ///
    /// Returns `true` iff the executor's recovery-time re-dispatch is safe
    /// for this Mote. Returns `false` for: `inconsistent` (cell 8 anomaly),
    /// `terminal_failure_observed` (cell 5 — terminal failure under
    /// EffectStaged), or `committed.is_some()` (cell 4 — done; never re-dispatch).
    ///
    /// **Returns `true` only when** an `EffectStaged` was observed AND no
    /// terminal failure AND no inconsistency AND no Committed yet — the
    /// in-flight case (cells 2 + 3) where the broker's tool-boundary
    /// idempotency closes the window.
    fn can_redispatch_world_effect_id(&self, id: &MoteId) -> bool {
        match self.motes.get(id) {
            None => false,
            Some(info) => {
                if info.inconsistent {
                    return false;
                }
                if info.terminal_failure_observed {
                    return false;
                }
                if info.committed.is_some() {
                    return false;
                }
                info.effect_staged_observed
            }
        }
    }

    /// Enumerate every Mote currently flagged anomalous, with its anomaly kind.
    fn anomaly_motes_iter(&self) -> Vec<(MoteId, AnomalyKind)> {
        self.motes
            .iter()
            .filter_map(|(id, info)| {
                if info.inconsistent {
                    Some((*id, AnomalyKind::EffectStagedThenRepudiatedNoCommitted))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Return the declared-or-committed parent list for `id`. Declared takes
    /// precedence if both exist (they SHOULD be identical — the executor passes the
    /// same `parents` list to register_mote and to the journal entry).
    fn parents_of_id(&self, id: &MoteId) -> SmallVec<[(MoteId, EdgeMeta); 4]> {
        let Some(info) = self.motes.get(id) else {
            return SmallVec::new();
        };
        if let Some(d) = &info.declared {
            return d.parents.iter().map(|p| (p.parent_id, p.edge)).collect();
        }
        if let Some(c) = &info.committed {
            return c
                .parents_in_entry
                .iter()
                .filter_map(|p| p.to_parent_ref().map(|pr| (pr.parent_id, pr.edge)))
                .collect();
        }
        SmallVec::new()
    }

    /// Rebuild the child→parent reverse index from the declared-or-committed
    /// adjacency. Cheap given typical workflow sizes; recomputed on graph mutation.
    fn rebuild_children_index(&mut self) {
        let mut idx: BTreeMap<MoteId, Vec<(MoteId, EdgeMeta)>> = BTreeMap::new();
        let ids: Vec<MoteId> = self.motes.keys().copied().collect();
        for id in ids {
            for (parent_id, edge) in self.parents_of_id(&id) {
                idx.entry(parent_id).or_default().push((id, edge));
            }
        }
        // Stable ordering: sort each adjacency list by child id for determinism.
        for v in idx.values_mut() {
            v.sort_by(|a, b| a.0.cmp(&b.0));
        }
        self.children = idx;
    }
}

// ---------------------------------------------------------------------------
// The Projection — the live in-memory state
// ---------------------------------------------------------------------------

/// The journal's read-side projection.
///
/// Apply [`Projection::register_mote`] for workflow-declared Motes, then
/// [`Projection::fold`] each journal entry in `seq` order; query via the 7-method
/// read API. [`Projection::snapshot`] returns an immutable point-in-time view that
/// implements the same read API with stable snapshot semantics.
///
/// # Examples
///
/// Fold a Committed entry and inspect the resulting state:
///
/// ```
/// use kx_journal::JournalEntry;
/// use kx_mote::{MoteDefHash, MoteId, NdClass};
/// use kx_projection::{MoteState, Projection};
/// use kx_content::ContentRef;
/// use smallvec::SmallVec;
///
/// let mut p = Projection::new();
/// assert!(p.is_empty());
///
/// let entry = JournalEntry::Committed {
///     mote_id: MoteId::from_bytes([1u8; 32]),
///     idempotency_key: [1u8; 32],
///     seq: 1,
///     nondeterminism: NdClass::Pure,
///     result_ref: ContentRef::from_bytes([7u8; 32]),
///     parents: SmallVec::new(),
///     warrant_ref: ContentRef::from_bytes([0xaa; 32]),
///     mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
/// };
/// p.fold(&entry).unwrap();
///
/// assert_eq!(p.current_seq(), 1);
/// assert_eq!(p.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Committed);
/// assert_eq!(p.committed_count(), 1);
/// ```
#[derive(Debug, Default)]
pub struct Projection {
    state: State,
}

impl Projection {
    /// Construct an empty projection.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a projection by reading every entry from `journal` in `seq` order.
    ///
    /// Convenience for tests and for cold-start replay. Production callers
    /// typically construct an empty projection, [`Projection::register_mote`] for
    /// each workflow-declared Mote, and [`Projection::fold`] incrementally as new
    /// journal entries land.
    pub fn from_journal<J: Journal>(journal: &J) -> Result<Self, ProjectionError> {
        let mut p = Self::new();
        let max_seq = journal.current_seq()?;
        // current_seq returns 0 for empty; use saturating range.
        let entries = journal.read_entries_by_seq(0..(max_seq + 1))?;
        for entry in entries {
            p.fold(&entry)?;
        }
        Ok(p)
    }

    /// Register a workflow-declared Mote.
    ///
    /// Adds the Mote to the projection's expected set; its state becomes
    /// [`MoteState::Pending`] until the first journal entry lands. Re-registration
    /// of the same `MoteId` is permitted and overwrites the declared info (the
    /// workflow author may update parents before submission; the journal-side
    /// dedupe-by-key path is the authoritative arbiter for identity equality).
    pub fn register_mote(&mut self, reg: RegisterMote) {
        let info = self.state.moteinfo_mut(&reg.mote_id);
        info.declared = Some(DeclaredInfo {
            nd_class: reg.nd_class,
            effect_pattern: reg.effect_pattern,
            critic_for: reg.critic_for,
            is_topology_shaper: reg.is_topology_shaper,
            parents: reg.parents,
        });
        self.state.rebuild_children_index();
    }

    /// Apply one journal entry. **Caller must invoke in `seq` order**; the fold is
    /// `seq`-order-dependent for correctness (per `projection.md` §3 — the
    /// determinism contract assumes log-order folding).
    ///
    /// Returns the previous `last_seq` for diagnostics; callers may ignore the
    /// return value. A duplicate `Committed` for the same `MoteId` surfaces
    /// [`ProjectionError::DuplicateCommitted`] — that is a journal-impl bug per
    /// `projection.md` §4.
    pub fn fold(&mut self, entry: &JournalEntry) -> Result<u64, ProjectionError> {
        let prev = self.state.last_seq;
        match entry {
            JournalEntry::Proposed { mote_id, seq, .. } => {
                let info = self.state.moteinfo_mut(mote_id);
                info.has_proposed = true;
                // A new Proposed clears any prior pending-failure marker (failed→proposed
                // is a valid sequence per mote.md §7).
                // NOTE: `terminal_failure_observed` and `inconsistent` are
                // **prefix-monotonic-true** per STEP 5.2 + STEP 5.3 of PR 4.5
                // — they are NEVER reset, here or anywhere.
                info.failed_pending_reattempt = false;
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::Committed {
                mote_id,
                seq,
                nondeterminism,
                result_ref,
                parents,
                warrant_ref,
                mote_def_hash,
                ..
            } => {
                let info = self.state.moteinfo_mut(mote_id);
                if info.committed.is_some() {
                    return Err(ProjectionError::DuplicateCommitted(*mote_id));
                }
                info.committed = Some(CommittedInfo {
                    seq: *seq,
                    result_ref: *result_ref,
                    nondeterminism: *nondeterminism,
                    parents_in_entry: parents.clone(),
                    warrant_ref: *warrant_ref,
                    mote_def_hash: *mote_def_hash,
                    repudiated: false,
                });
                self.state.last_seq = self.state.last_seq.max(*seq);
                // Rebuild children index since this Committed entry may introduce
                // parent→child edges not previously visible (e.g., entry written
                // for a Mote that wasn't pre-registered).
                self.state.rebuild_children_index();
            }
            JournalEntry::Failed {
                mote_id,
                seq,
                reason_class,
                ..
            } => {
                let info = self.state.moteinfo_mut(mote_id);
                if info.committed.is_none() {
                    info.failed_pending_reattempt = true;
                    // **v2 (STEP 5.2 + STEP 6.2).** Read reason_class to set
                    // terminal_failure_observed. Prefix-monotonic-true: once
                    // set, NEVER reset — even by a subsequent Proposed (the
                    // monotonicity contract that closes cell 5 of the 9-cell
                    // cross-product). The canonical classifier
                    // `is_pre_commit_crash` is the single source of class truth.
                    if !kx_journal::is_pre_commit_crash(*reason_class) {
                        info.terminal_failure_observed = true;
                    }
                }
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::Repudiated {
                target_mote_id,
                seq,
                target_committed_seq,
                ..
            } => {
                let info = self.state.moteinfo_mut(target_mote_id);
                // Only flip the repudiated flag if the target Committed entry is
                // actually present in the projection AND its seq matches. Per
                // `projection.md` §5, a Repudiated naming a non-existent target is
                // recorded as a fact (the cascade walker can list repudiation
                // entries from the journal directly) but does NOT create a phantom
                // marker in `state_of` — keeps `state_of` semantics clean.
                if let Some(c) = info.committed.as_mut() {
                    if c.seq == *target_committed_seq {
                        c.repudiated = true;
                    }
                } else if info.effect_staged_observed {
                    // **v2 (STEP 5.3): cell-8 anomaly.** EffectStaged was
                    // folded for this Mote, but a Repudiated arrived BEFORE
                    // any Committed. Repudiated normally targets a Committed;
                    // this is a journal-consistency error. We quarantine via
                    // `info.inconsistent = true` (prefix-monotonic-true; never
                    // reset) rather than aborting the fold. One anomalous Mote
                    // must not take down the entire recovery; surface via
                    // `anomaly_motes()` for operator review.
                    info.inconsistent = true;
                }
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::EffectStaged { mote_id, seq, .. } => {
                // **v2 (D38 §2b): EffectStaged recovery hint.** Set the
                // prefix-monotonic-true flag; the recovery fold combines this
                // with subsequent Committed/Failed/Repudiated entries via
                // `state_of_id` (Terminal-before-Staged ordering invariant)
                // and `can_redispatch_world_effect_id`.
                let info = self.state.moteinfo_mut(mote_id);
                info.effect_staged_observed = true;
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
        }
        Ok(prev)
    }

    /// The largest `seq` applied so far. `0` for an empty projection.
    #[inline]
    #[must_use]
    pub fn current_seq(&self) -> u64 {
        self.state.last_seq
    }

    /// Number of Motes the projection knows about (registered + entry-introduced).
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.motes.len()
    }

    /// `true` when the projection has no registered or entry-introduced Motes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.motes.is_empty()
    }

    /// Capture an immutable point-in-time view. Subsequent folds against the
    /// `Projection` do not affect the returned `Snapshot` — this is the
    /// snapshot-isolation contract per D16 / `projection.md` §6.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state.clone(),
        }
    }

    // ----- Read API (delegates to State; same surface as Snapshot) -----

    /// The per-identity state per `projection.md` §4.
    #[must_use]
    pub fn state_of(&self, mote_id: &MoteId) -> MoteState {
        self.state.state_of_id(mote_id)
    }

    /// Direct parents with edge metadata.
    #[must_use]
    pub fn parents_of(&self, mote_id: &MoteId) -> SmallVec<[(MoteId, EdgeMeta); 4]> {
        self.state.parents_of_id(mote_id)
    }

    /// Direct children with edge metadata.
    #[must_use]
    pub fn children_of(&self, mote_id: &MoteId) -> Vec<(MoteId, EdgeMeta)> {
        self.state
            .children
            .get(mote_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The transitive closure of downstream consumers reachable via data edges
    /// (always) and non-opted-out control edges (per `control-edge-cascade-default.md`).
    /// BFS with visited-set termination — cycle-safe.
    #[must_use]
    pub fn transitive_consumers(&self, mote_id: &MoteId) -> Vec<MoteId> {
        transitive_consumers_impl(&self.state, mote_id)
    }

    /// The Mote's committed `result_ref`, if any.
    #[must_use]
    pub fn result_ref_of(&self, mote_id: &MoteId) -> Option<ContentRef> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.result_ref))
    }

    /// Motes whose parents are all `Committed-and-not-Repudiated` AND whose
    /// WORLD-MUTATING parents have promotion_state ∈ {NotApplicable, Promoted} AND
    /// that are themselves in `Pending` state.
    ///
    /// The WORLD-MUTATING promotion filter is in the contract here, not in the
    /// caller — per `projection.md` §7. (In P1 the filter is a no-op because
    /// promotion_state always returns NotApplicable; the filter activates once
    /// P1.9 wires the MoteDef registry.)
    #[must_use]
    pub fn ready_set(&self) -> Vec<MoteId> {
        ready_set_impl(&self.state)
    }

    /// 3c promotion state for the producer.
    ///
    /// **P1 default**: returns `NotApplicable` for every Mote (per D18). Full 3c
    /// semantics activate when the executor (P1.9) wires a `MoteDef` lookup that
    /// lets the projection observe critic-of-producer relationships.
    #[must_use]
    pub fn promotion_state(&self, mote_id: &MoteId) -> PromotionState {
        promotion_state_impl(&self.state, mote_id)
    }

    /// `true` when a Repudiated entry targeting this Mote's committed entry has
    /// been folded.
    #[must_use]
    pub fn is_repudiated(&self, mote_id: &MoteId) -> bool {
        matches!(self.state_of(mote_id), MoteState::Repudiated)
    }

    /// The `seq` of the Mote's `Committed` entry, if present. Useful for callers
    /// constructing a `Repudiated` entry that needs to reference the target.
    #[must_use]
    pub fn committed_seq_of(&self, mote_id: &MoteId) -> Option<u64> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.seq))
    }

    /// **v2 (PR 7, STEP 5.3 + R-13).** Recovery-time predicate: can the
    /// executor safely re-dispatch a WORLD-MUTATING effect for this Mote?
    ///
    /// Returns `true` iff: `info.effect_staged_observed` AND NOT
    /// `info.terminal_failure_observed` AND NOT `info.inconsistent` AND
    /// `info.committed.is_none()`. This is the in-flight case (cells 2 + 3
    /// of the 9-cell cross-product) where the broker's tool-boundary
    /// idempotency closes the window.
    ///
    /// Returns `false` for `inconsistent` (cell 8 anomaly),
    /// `terminal_failure_observed` (cell 5 — terminal failure under
    /// EffectStaged; the WM double-effect hazard), `committed.is_some()`
    /// (cells 4 + 6 — done; never re-dispatch), and for Motes with no
    /// `EffectStaged` observed (no in-flight effect to re-dispatch).
    ///
    /// **Prefix-monotonic refusal** (STEP 5.2): once this returns `false`
    /// for a given Mote, it returns `false` at every longer log prefix.
    /// Proven by `prop_terminal_refusal_is_prefix_monotonic`.
    #[must_use]
    pub fn can_redispatch_world_effect(&self, mote_id: &MoteId) -> bool {
        self.state.can_redispatch_world_effect_id(mote_id)
    }

    /// **v2 (PR 7, STEP 5.3).** Enumerate every Mote currently flagged
    /// anomalous, with its anomaly kind. Operator-facing diagnostic API;
    /// NOT on any hot recovery path.
    ///
    /// Today returns only [`AnomalyKind::EffectStagedThenRepudiatedNoCommitted`]
    /// (cell 8). Future fold-cell anomalies extend [`AnomalyKind`] via
    /// additive variants.
    #[must_use]
    pub fn anomaly_motes(&self) -> Vec<(MoteId, AnomalyKind)> {
        self.state.anomaly_motes_iter()
    }

    /// Apply a sequence of journal entries in order. Convenience over calling
    /// [`Self::fold`] in a loop. Stops on the first error and returns it; the
    /// projection's state at that point reflects every entry applied up to
    /// (but not including) the failing one.
    ///
    /// # Errors
    /// First [`ProjectionError`] from any contained entry (typically a
    /// duplicate-Committed surfaced by [`Self::fold`]).
    pub fn fold_many<I>(&mut self, entries: I) -> Result<u64, ProjectionError>
    where
        I: IntoIterator<Item = JournalEntry>,
    {
        let mut last = self.state.last_seq;
        for entry in entries {
            self.fold(&entry)?;
            last = self.state.last_seq;
        }
        Ok(last)
    }

    /// Iterate every known Mote with its current state. Iteration order is by
    /// `MoteId` ascending (stable via the underlying `BTreeMap`).
    ///
    /// Used by the executor (P1.9) to enumerate the workflow's status; used by
    /// debugging tools to dump the projection. Allocates nothing per item.
    pub fn iter_motes(&self) -> impl Iterator<Item = (MoteId, MoteState)> + '_ {
        self.state
            .motes
            .keys()
            .map(move |id| (*id, self.state.state_of_id(id)))
    }

    /// Iterate every Mote currently in `state`. Iteration order is by `MoteId`
    /// ascending.
    pub fn iter_motes_in_state(&self, state: MoteState) -> impl Iterator<Item = MoteId> + '_ {
        self.state
            .motes
            .keys()
            .filter(move |id| self.state.state_of_id(id) == state)
            .copied()
    }

    /// Count of Motes currently in `MoteState::Committed`.
    #[must_use]
    pub fn committed_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Committed).count()
    }

    /// Count of Motes currently in `MoteState::Repudiated`.
    #[must_use]
    pub fn repudiated_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Repudiated).count()
    }

    /// Count of Motes currently in `MoteState::Pending`.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Pending).count()
    }

    /// Count of Motes currently in `MoteState::Failed`.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Failed).count()
    }

    /// Count of Motes currently in `MoteState::Scheduled`.
    #[must_use]
    pub fn scheduled_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Scheduled).count()
    }
}

// ---------------------------------------------------------------------------
// Snapshot — immutable point-in-time view
// ---------------------------------------------------------------------------

/// An immutable point-in-time view of a [`Projection`].
///
/// Returned by [`Projection::snapshot`]. Subsequent folds against the source
/// projection do not affect this snapshot — the snapshot-isolation contract from
/// D16 / `projection.md` §6 is provided by cloning the underlying state.
///
/// # Examples
///
/// A snapshot remains stable while the projection mutates underneath:
///
/// ```
/// use kx_journal::{FailureReason, JournalEntry, RepudiationReason};
/// use kx_mote::{MoteDefHash, MoteId, NdClass};
/// use kx_projection::{MoteState, Projection};
/// use kx_content::ContentRef;
/// use smallvec::SmallVec;
///
/// let mut p = Projection::new();
/// p.fold(&JournalEntry::Committed {
///     mote_id: MoteId::from_bytes([1u8; 32]),
///     idempotency_key: [1u8; 32],
///     seq: 1,
///     nondeterminism: NdClass::Pure,
///     result_ref: ContentRef::from_bytes([7u8; 32]),
///     parents: SmallVec::new(),
///     warrant_ref: ContentRef::from_bytes([0xaa; 32]),
///     mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
/// }).unwrap();
///
/// let snap = p.snapshot();
/// assert_eq!(snap.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Committed);
/// assert_eq!(snap.seq(), 1);
///
/// // Mutate the projection; snapshot stays at seq 1.
/// p.fold(&JournalEntry::Repudiated {
///     target_mote_id: MoteId::from_bytes([1u8; 32]),
///     idempotency_key: [9u8; 32],
///     seq: 2,
///     target_committed_seq: 1,
///     reason_class: RepudiationReason::OperatorAction,
///     repudiator_id: 0,
/// }).unwrap();
///
/// assert_eq!(snap.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Committed);
/// assert_eq!(p.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Repudiated);
/// ```
#[derive(Debug, Clone)]
pub struct Snapshot {
    state: State,
}

impl Snapshot {
    /// The `seq` at which this snapshot was captured.
    #[inline]
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.state.last_seq
    }

    /// Number of Motes in this snapshot.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.motes.len()
    }

    /// `true` when the snapshot is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.motes.is_empty()
    }

    /// Per-identity state. See [`Projection::state_of`].
    #[must_use]
    pub fn state_of(&self, mote_id: &MoteId) -> MoteState {
        self.state.state_of_id(mote_id)
    }

    /// Direct parents. See [`Projection::parents_of`].
    #[must_use]
    pub fn parents_of(&self, mote_id: &MoteId) -> SmallVec<[(MoteId, EdgeMeta); 4]> {
        self.state.parents_of_id(mote_id)
    }

    /// Direct children. See [`Projection::children_of`].
    #[must_use]
    pub fn children_of(&self, mote_id: &MoteId) -> Vec<(MoteId, EdgeMeta)> {
        self.state
            .children
            .get(mote_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Transitive consumers via data + non-opted-out control edges.
    #[must_use]
    pub fn transitive_consumers(&self, mote_id: &MoteId) -> Vec<MoteId> {
        transitive_consumers_impl(&self.state, mote_id)
    }

    /// Committed result_ref. See [`Projection::result_ref_of`].
    #[must_use]
    pub fn result_ref_of(&self, mote_id: &MoteId) -> Option<ContentRef> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.result_ref))
    }

    /// The ready set at the snapshot's `seq`.
    #[must_use]
    pub fn ready_set(&self) -> Vec<MoteId> {
        ready_set_impl(&self.state)
    }

    /// Promotion state (P1 default: `NotApplicable`).
    #[must_use]
    pub fn promotion_state(&self, mote_id: &MoteId) -> PromotionState {
        promotion_state_impl(&self.state, mote_id)
    }

    /// `true` when the Mote's committed entry has been Repudiated by `seq`.
    #[must_use]
    pub fn is_repudiated(&self, mote_id: &MoteId) -> bool {
        matches!(self.state_of(mote_id), MoteState::Repudiated)
    }

    /// **v2 (PR 7, STEP 5.3 + R-13).** Snapshot mirror of
    /// [`Projection::can_redispatch_world_effect`].
    #[must_use]
    pub fn can_redispatch_world_effect(&self, mote_id: &MoteId) -> bool {
        self.state.can_redispatch_world_effect_id(mote_id)
    }

    /// **v2 (PR 7, STEP 5.3).** Snapshot mirror of [`Projection::anomaly_motes`].
    #[must_use]
    pub fn anomaly_motes(&self) -> Vec<(MoteId, AnomalyKind)> {
        self.state.anomaly_motes_iter()
    }

    /// Iterate every Mote known at snapshot time with its state.
    pub fn iter_motes(&self) -> impl Iterator<Item = (MoteId, MoteState)> + '_ {
        self.state
            .motes
            .keys()
            .map(move |id| (*id, self.state.state_of_id(id)))
    }

    /// Iterate every Mote currently in `state` at snapshot time.
    pub fn iter_motes_in_state(&self, state: MoteState) -> impl Iterator<Item = MoteId> + '_ {
        self.state
            .motes
            .keys()
            .filter(move |id| self.state.state_of_id(id) == state)
            .copied()
    }

    /// Count of Motes in `MoteState::Committed` at snapshot time.
    #[must_use]
    pub fn committed_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Committed).count()
    }

    /// Count of Motes in `MoteState::Repudiated` at snapshot time.
    #[must_use]
    pub fn repudiated_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Repudiated).count()
    }

    /// Count of Motes in `MoteState::Pending` at snapshot time.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Pending).count()
    }

    /// Count of Motes in `MoteState::Failed` at snapshot time.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Failed).count()
    }

    /// Count of Motes in `MoteState::Scheduled` at snapshot time.
    #[must_use]
    pub fn scheduled_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Scheduled).count()
    }
}

// ---------------------------------------------------------------------------
// Shared read-side implementations (used by both Projection and Snapshot)
// ---------------------------------------------------------------------------

fn transitive_consumers_impl(state: &State, root: &MoteId) -> Vec<MoteId> {
    let mut visited: BTreeSet<MoteId> = BTreeSet::new();
    let mut order: Vec<MoteId> = Vec::new();
    let mut queue: VecDeque<MoteId> = VecDeque::new();
    queue.push_back(*root);

    while let Some(current) = queue.pop_front() {
        // children_of for the current node
        let children = state.children.get(&current).cloned().unwrap_or_default();
        for (child, edge) in children {
            // Cascade rule: data edges always cascade; control edges cascade unless
            // explicitly non_cascade.
            let should_walk = match edge.kind {
                EdgeKind::Data => true,
                EdgeKind::Control => !edge.non_cascade,
            };
            if !should_walk {
                continue;
            }
            if visited.insert(child) {
                order.push(child);
                queue.push_back(child);
            }
        }
    }
    order
}

fn ready_set_impl(state: &State) -> Vec<MoteId> {
    let mut out: Vec<MoteId> = Vec::new();
    for (id, info) in &state.motes {
        if state.state_of_id(id) != MoteState::Pending {
            continue;
        }
        // Parents must all be Committed-and-not-Repudiated.
        let Some(d) = info.declared.as_ref() else {
            // Pending implies declared (registered).
            continue;
        };
        let mut all_parents_committed = true;
        let mut all_wm_parents_promoted = true;
        for p in &d.parents {
            let pstate = state.state_of_id(&p.parent_id);
            if pstate != MoteState::Committed {
                all_parents_committed = false;
                break;
            }
            // WORLD-MUTATING promotion gate (per projection.md §7). In P1 default,
            // promotion_state always returns NotApplicable, so the gate passes
            // trivially. The check is in the contract so it activates when the
            // executor (P1.9) wires the MoteDef registry.
            if let Some(pinfo) = state.motes.get(&p.parent_id) {
                if let Some(committed) = &pinfo.committed {
                    if committed.nondeterminism == NdClass::WorldMutating {
                        match promotion_state_impl(state, &p.parent_id) {
                            PromotionState::Promoted | PromotionState::NotApplicable => {}
                            PromotionState::Unpromoted => {
                                all_wm_parents_promoted = false;
                                break;
                            }
                        }
                    }
                }
            }
        }
        if all_parents_committed && all_wm_parents_promoted {
            out.push(*id);
        }
    }
    out
}

fn promotion_state_impl(_state: &State, _mote_id: &MoteId) -> PromotionState {
    // P1 default per D18: no observable critic-of-producer relationships until
    // P1.9's MoteDef registry wires the lookup. All Motes return NotApplicable.
    PromotionState::NotApplicable
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kx_journal::{FailureReason, RepudiationReason};

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    fn cref(b: u8) -> ContentRef {
        ContentRef::from_bytes([b; 32])
    }

    fn dh(b: u8) -> MoteDefHash {
        MoteDefHash::from_bytes([b; 32])
    }

    fn proposed_entry(mote_byte: u8, seq: u64) -> JournalEntry {
        JournalEntry::Proposed {
            mote_id: mid(mote_byte),
            idempotency_key: [mote_byte; 32],
            seq,
            nondeterminism: NdClass::Pure,
            placement_hint: 0,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        }
    }

    fn committed_entry(mote_byte: u8, seq: u64, nd: NdClass) -> JournalEntry {
        JournalEntry::Committed {
            mote_id: mid(mote_byte),
            idempotency_key: [mote_byte; 32],
            seq,
            nondeterminism: nd,
            result_ref: cref(mote_byte),
            parents: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: dh(mote_byte),
        }
    }

    fn failed_entry(mote_byte: u8, seq: u64) -> JournalEntry {
        JournalEntry::Failed {
            mote_id: mid(mote_byte),
            idempotency_key: [mote_byte; 32],
            seq,
            reason_class: FailureReason::TimedOut,
            reporter_id: 0,
        }
    }

    fn repudiated_entry(target_byte: u8, target_seq: u64, seq: u64) -> JournalEntry {
        JournalEntry::Repudiated {
            target_mote_id: mid(target_byte),
            idempotency_key: [0u8; 32], // would be derived; for in-memory fold the byte content doesn't matter
            seq,
            target_committed_seq: target_seq,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 0,
        }
    }

    #[test]
    fn empty_projection_is_pending_for_unknown_motes() {
        let p = Projection::new();
        assert_eq!(p.state_of(&mid(1)), MoteState::Pending);
        assert!(p.is_empty());
    }

    #[test]
    fn proposed_then_committed_collapses_to_committed() {
        let mut p = Projection::new();
        p.fold(&proposed_entry(1, 1)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Scheduled);
        p.fold(&committed_entry(1, 2, NdClass::Pure)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Committed);
    }

    #[test]
    fn failed_then_proposed_resets_to_scheduled() {
        let mut p = Projection::new();
        p.fold(&proposed_entry(1, 1)).unwrap();
        p.fold(&failed_entry(1, 2)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Failed);
        p.fold(&proposed_entry(1, 3)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Scheduled);
    }

    #[test]
    fn repudiated_only_applies_when_target_committed_seq_matches() {
        let mut p = Projection::new();
        p.fold(&committed_entry(1, 5, NdClass::Pure)).unwrap();
        // Wrong target_committed_seq — projection ignores
        p.fold(&repudiated_entry(1, 99, 6)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Committed);
        // Correct target_committed_seq
        p.fold(&repudiated_entry(1, 5, 7)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Repudiated);
    }

    #[test]
    fn duplicate_committed_for_same_mote_id_surfaces_loudly() {
        let mut p = Projection::new();
        p.fold(&committed_entry(1, 1, NdClass::Pure)).unwrap();
        let result = p.fold(&committed_entry(1, 2, NdClass::Pure));
        assert!(matches!(
            result,
            Err(ProjectionError::DuplicateCommitted(_))
        ));
    }

    #[test]
    fn last_seq_advances_monotonically() {
        let mut p = Projection::new();
        p.fold(&proposed_entry(1, 1)).unwrap();
        p.fold(&proposed_entry(2, 2)).unwrap();
        p.fold(&committed_entry(1, 3, NdClass::Pure)).unwrap();
        assert_eq!(p.current_seq(), 3);
    }

    #[test]
    fn register_mote_makes_it_pending() {
        let mut p = Projection::new();
        p.register_mote(RegisterMote {
            mote_id: mid(1),
            nd_class: NdClass::Pure,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            parents: SmallVec::new(),
        });
        assert_eq!(p.state_of(&mid(1)), MoteState::Pending);
    }

    #[test]
    fn snapshot_is_immutable_under_subsequent_folds() {
        let mut p = Projection::new();
        p.fold(&committed_entry(1, 1, NdClass::Pure)).unwrap();
        let snap = p.snapshot();
        assert_eq!(snap.state_of(&mid(1)), MoteState::Committed);
        // Mutate the projection — snapshot must NOT change
        p.fold(&repudiated_entry(1, 1, 2)).unwrap();
        assert_eq!(snap.state_of(&mid(1)), MoteState::Committed); // unchanged
        assert_eq!(p.state_of(&mid(1)), MoteState::Repudiated); // updated
    }

    #[test]
    fn promotion_state_is_not_applicable_in_p1() {
        let mut p = Projection::new();
        p.fold(&committed_entry(1, 1, NdClass::WorldMutating))
            .unwrap();
        // Per D18 P1 default — even WM motes are NotApplicable until the
        // executor (P1.9) wires the MoteDef registry.
        assert_eq!(p.promotion_state(&mid(1)), PromotionState::NotApplicable);
    }

    #[test]
    fn state_of_for_non_existent_target_of_repudiation_remains_pending() {
        let mut p = Projection::new();
        // Repudiate a MoteId that was never committed — projection records nothing
        // observable via state_of (per projection.md §5).
        p.fold(&repudiated_entry(1, 99, 1)).unwrap();
        assert_eq!(p.state_of(&mid(1)), MoteState::Pending);
    }
}
