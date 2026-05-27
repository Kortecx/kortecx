//! Internal state machinery for the projection: the per-Mote info records
//! (CommittedInfo + MoteInfo + DeclaredInfo) and the State container with its
//! fold logic. All types here are `pub(crate)` — they are not part of the
//! crate's public API; [`crate::Projection`] and [`crate::Snapshot`] are the
//! exposed handles.

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_journal::ParentEntry;
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use smallvec::SmallVec;

use crate::enums::{AnomalyKind, MoteState};

#[derive(Debug, Clone)]
pub(crate) struct CommittedInfo {
    pub(crate) seq: u64,
    pub(crate) result_ref: ContentRef,
    pub(crate) nondeterminism: NdClass,
    pub(crate) parents_in_entry: SmallVec<[ParentEntry; 4]>,
    /// The warrant under which this commit was performed. NEW in v2 (D36).
    /// Stored so consumers (executor recovery, audit log walkers) can read
    /// it via the projection's API without re-decoding the journal entry.
    /// Not yet read in P1.5; will be consumed by P1.9's submission-time
    /// refusal predicates.
    #[allow(dead_code)]
    pub(crate) warrant_ref: ContentRef,
    /// Retained for the D22 `list_committed_by_mote_def_hash`-driven cascade
    /// (operator-level definition repudiation surfaces the def_hash; consumers
    /// reach for it here when constructing cascade sets). Not yet read in P1.5;
    /// will be consumed by the executor-side flow that initiates definition-
    /// level cascades.
    #[allow(dead_code)]
    pub(crate) mote_def_hash: MoteDefHash,
    pub(crate) repudiated: bool,
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
pub(crate) struct MoteInfo {
    /// Workflow-author-declared properties (when registered).
    pub(crate) declared: Option<DeclaredInfo>,
    /// Committed-entry info (when a `Committed` entry has been folded).
    pub(crate) committed: Option<CommittedInfo>,
    /// `true` if at least one `Proposed` entry has been folded for this MoteId.
    pub(crate) has_proposed: bool,
    /// `true` if at least one `Failed` entry has been folded with no later `Proposed`.
    /// We track this directly because `Failed → Proposed` is a valid sequence
    /// (`mote.md` §7 + `journal-entry.md` §7.5). **NOT prefix-monotonic** —
    /// reset to `false` by a subsequent `Proposed`. Distinct from
    /// `terminal_failure_observed` below.
    pub(crate) failed_pending_reattempt: bool,
    /// **v2 (PR 7).** `true` if at least one `EffectStaged` entry has been
    /// folded for this MoteId. **Prefix-monotonic-true** — never reset by any
    /// fold branch. Set in the `EffectStaged` arm.
    pub(crate) effect_staged_observed: bool,
    /// **v2 (PR 7).** `true` if at least one `Failed` entry has been folded
    /// with a terminal `reason_class` (i.e., NOT pre-commit-crash per
    /// [`kx_journal::is_pre_commit_crash`]). **Prefix-monotonic-true** — never
    /// reset. This is the LOAD-BEARING flag that closes the cell-5 WM
    /// double-effect hazard per STEP 5.2 of PR 4.5.
    pub(crate) terminal_failure_observed: bool,
    /// **v2 (PR 7).** `true` if the cell-8 anomaly was observed: a
    /// `Repudiated` entry referenced this Mote while an `EffectStaged` had
    /// been folded but no `Committed` was ever folded in between.
    /// **Prefix-monotonic-true** — never reset. Quarantines the Mote per
    /// STEP 5.3.
    pub(crate) inconsistent: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
// `nd_class`, `effect_pattern`, `critic_for`, `is_topology_shaper`,
// `warrant_ref` are stored at registration but unread in P1.5. P1.9 (the
// executor) consumes them via a MoteDef registry lookup to compute the full
// 3c promotion behavior + the topology-shaper materialization at P1.11.
// `warrant_ref` is the PR-11.5 KG-1-close payload — the per-child narrowed
// warrant ref the executor will dispatch under.
pub(crate) struct DeclaredInfo {
    pub(crate) nd_class: NdClass,
    pub(crate) effect_pattern: EffectPattern,
    pub(crate) critic_for: Option<MoteId>,
    pub(crate) is_topology_shaper: bool,
    pub(crate) parents: SmallVec<[ParentRef; 4]>,
    pub(crate) warrant_ref: ContentRef,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct State {
    /// Per-MoteId info — declared, committed, and any in-flight state.
    pub(crate) motes: BTreeMap<MoteId, MoteInfo>,
    /// child → parents adjacency (derived from `MoteInfo.declared.parents` or
    /// `committed.parents_in_entry`). Computed by `parents_of`.
    /// We also maintain a reverse index for fast `children_of`.
    pub(crate) children: BTreeMap<MoteId, Vec<(MoteId, EdgeMeta)>>,
    /// The largest `seq` value applied so far.
    pub(crate) last_seq: u64,
}

impl State {
    pub(crate) fn moteinfo_mut(&mut self, id: &MoteId) -> &mut MoteInfo {
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
    pub(crate) fn state_of_id(&self, id: &MoteId) -> MoteState {
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
    pub(crate) fn can_redispatch_world_effect_id(&self, id: &MoteId) -> bool {
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
    pub(crate) fn anomaly_motes_iter(&self) -> Vec<(MoteId, AnomalyKind)> {
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
    pub(crate) fn parents_of_id(&self, id: &MoteId) -> SmallVec<[(MoteId, EdgeMeta); 4]> {
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
    pub(crate) fn rebuild_children_index(&mut self) {
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
