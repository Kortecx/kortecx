//! Internal state machinery for the projection: the per-Mote info records
//! (CommittedInfo + MoteInfo + DeclaredInfo) and the State container with its
//! fold logic. All types here are `pub(crate)` — they are not part of the
//! crate's public API; [`crate::Projection`] and [`crate::Snapshot`] are the
//! exposed handles.

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_journal::{FailureReason, ParentEntry};
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use smallvec::SmallVec;

use crate::enums::{AnomalyKind, MoteState};

#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// **v6 (M2.3b, D105.4 / D65).** `true` if a terminal
    /// `Failed { reason_class: QuarantinedAtLeastOnce }` was folded for this
    /// MoteId — recovery quarantined a staged-uncommitted at-most-once effect
    /// (no closing mechanism, compensation unsupported) rather than risk a
    /// double-fire. **Prefix-monotonic-true** — never reset. Distinct from
    /// `inconsistent` (the cell-8 Repudiated anomaly); a quarantined Mote is
    /// terminal `Failed` (via `terminal_failure_observed`), surfaced via
    /// [`crate::AnomalyKind::QuarantinedAtLeastOnceEffect`].
    pub(crate) quarantined: bool,
    /// **PR-3 (AL2).** The journaled [`FailureReason`] of the FIRST *terminal*
    /// `Failed` folded for this MoteId (i.e. NOT a pre-commit-crash per
    /// [`kx_journal::is_pre_commit_crash`]) — retained so a model-driven re-plan
    /// (and an operator) can read WHY a step dead-lettered, not merely THAT it did.
    /// **Prefix-monotonic** (first-terminal-reason-wins; set in the same `Failed`
    /// arm that sets `terminal_failure_observed`, never reset). Pure read-side
    /// annotation: it is NEVER an input to identity, `digest_projection`,
    /// `ready_set`, or any commit — the canonical demo never folds a `Failed`
    /// entry, so it stays `None` there and the digest is byte-unchanged. Surfaced
    /// via [`crate::Projection::failure_reason_of`] / [`crate::Snapshot::failure_reason_of`].
    pub(crate) failure_reason: Option<FailureReason>,
}

#[derive(Debug, Clone, PartialEq)]
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

/// The registered, journaled run identity (v3, M1.1, D63/D64). Established when
/// the `RunRegistered` entry (seq=1) is folded; **read on replay, never
/// recomputed**. Off the Mote-DAG — folding it touches no Mote and never
/// rebuilds the children index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunRegistration {
    /// The per-run nonce — the registered run identity (and token root).
    pub(crate) instance_id: [u8; kx_journal::INSTANCE_ID_LEN],
    /// The recipe fingerprint (discovery/dedup only; never identity).
    pub(crate) recipe_fingerprint: [u8; 32],
}

/// One resolved-version run-metadata record (v4, M1.2, D79), folded from a
/// `RunVersionsResolved` entry. **Audit/lineage metadata, never identity** — no
/// scheduling/identity/digest decision reads it. Off the Mote-DAG. Surfaced by
/// [`crate::Projection::run_resolved_versions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResolvedVersions {
    /// The run this metadata is attached to.
    pub instance_id: [u8; kx_journal::INSTANCE_ID_LEN],
    /// The warrant resolved under (`blake3(canonical_bincode(WarrantSpec))`).
    pub warrant_ref: ContentRef,
    /// The resolved model id (opaque audit identifier).
    pub model_id: String,
    /// The resolved capability, or `None` for a zero-grant warrant.
    pub capability: Option<kx_journal::ResolvedCapabilityRecord>,
}

/// One re-plan-round run-metadata record (v7, PR-2c-2, re-plan-live), folded from a
/// `ReplanRound` entry. **Audit/lineage + recovery metadata, never identity** — no
/// scheduling/identity/digest decision reads it. Off the Mote-DAG. The coordinator's
/// `recover_replan_chain` reads these to rebuild the round's shaper Mote
/// deterministically from committed facts. Surfaced by
/// [`crate::Projection::replan_rounds`] / [`crate::Projection::latest_replan_round`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplanRoundRecord {
    /// The re-plan round index (`0` = the run's initial-plan anchor; `1..` corrective).
    pub round: u32,
    /// The round's shaper `MoteId`.
    pub shaper_mote_id: MoteId,
    /// `ContentRef` of the run's immutable base planning prompt.
    pub base_prompt_ref: ContentRef,
    /// `ContentRef` of this round's corrected planning prompt.
    pub corrected_prompt_ref: ContentRef,
    /// The round's shaper `warrant_ref` (run-fixed ceiling).
    pub warrant_ref: ContentRef,
    /// The resolved model id the round's shaper runs.
    pub model_id: String,
    /// The step ids that failed and triggered this round (MoteId-byte-sorted, frozen).
    pub failed_steps: Vec<MoteId>,
    /// `Some(ref)` iff the model escalated (flag-a-human) for this round.
    pub escalation_reason_ref: Option<ContentRef>,
    /// The entry's journal seq (audit/order).
    pub seq: u64,
}

/// One ReAct-turn run-metadata record (v8, PR-2d-1, react-substrate), folded from
/// a `ReactRound` entry. **Audit/lineage + recovery metadata, never identity** — no
/// scheduling/identity/digest decision reads it. Off the Mote-DAG. The coordinator's
/// `settle_react_rounds` / `recover_react_chain` read these to settle the latest
/// turn and rebuild an in-flight turn's Mote deterministically from committed
/// facts; budget counters are RE-DERIVED by folding the recorded branches (never
/// an in-memory count). Surfaced by [`crate::Projection::react_rounds`] /
/// [`crate::Projection::latest_react_round`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactRoundRecord {
    /// The turn index (`0` = the run's submit anchor).
    pub turn: u32,
    /// The turn's (run-salted) `MoteId`.
    pub turn_mote_id: MoteId,
    /// The registered run identity (the run-salt) — keys every settle/recover
    /// query in serve's shared journal.
    pub instance_id: [u8; kx_journal::INSTANCE_ID_LEN],
    /// `ContentRef` of the run's immutable base instruction prompt.
    pub base_prompt_ref: ContentRef,
    /// The run-fixed turn `warrant_ref`.
    pub warrant_ref: ContentRef,
    /// The resolved model id the turns run.
    pub model_id: String,
    /// The turn's settled branch, frozen at append.
    pub branch: kx_journal::ReactBranch,
    /// The run's durable turn cap (recorded on the anchor).
    pub max_turns: u32,
    /// The run's durable tool-call cap (recorded on the anchor).
    pub max_tool_calls: u32,
    /// The entry's journal seq (audit/order).
    pub seq: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct State {
    /// Per-MoteId info — declared, committed, and any in-flight state.
    pub(crate) motes: BTreeMap<MoteId, MoteInfo>,
    /// child → parents adjacency (derived from `MoteInfo.declared.parents` or
    /// `committed.parents_in_entry`). Computed by `parents_of`.
    /// We also maintain a reverse index for fast `children_of`.
    pub(crate) children: BTreeMap<MoteId, Vec<(MoteId, EdgeMeta)>>,
    /// The largest `seq` value applied so far.
    pub(crate) last_seq: u64,
    /// The registered run identity (D64), or `None` until a `RunRegistered`
    /// entry is folded. Off-DAG; O(1) to set and read.
    pub(crate) run_registration: Option<RunRegistration>,
    /// Resolved-version run metadata (D79), appended as each `RunVersionsResolved`
    /// entry folds (one per resolved capability). Off-DAG; O(1) per append.
    /// Audit/lineage only — never an identity/scheduling/digest input.
    pub(crate) run_resolved_versions: Vec<RunResolvedVersions>,
    /// Re-plan-round metadata (PR-2c-2), appended as each `ReplanRound` entry folds.
    /// Off-DAG; O(1) per append. Recovery + audit only — never an
    /// identity/scheduling/digest input.
    pub(crate) replan_rounds: Vec<ReplanRoundRecord>,
    /// ReAct-turn metadata (PR-2d-1), appended as each `ReactRound` entry folds.
    /// Off-DAG; O(1) per append. Recovery + audit only — never an
    /// identity/scheduling/digest input. Emptiness is the `has_react_turn`
    /// sentinel that keeps react-free runs (and the demo) zero-cost.
    pub(crate) react_rounds: Vec<ReactRoundRecord>,
    /// PR-2d-2 — DERIVED per-instance index over `react_rounds`: `instance_id`
    /// → indices into the Vec, in append (= seq) order. `react_rounds` only
    /// ever GROWS in serve's shared journal, so without this every per-chain
    /// settle/recover/trajectory query is a scan over EVERY chain's facts
    /// forever (the PR-2d-1 O(runs²) adversarial finding — the coordinator's
    /// `ReactSettleCache` bounds the settle; this bounds the per-chain reads).
    /// Maintained by the fold; RE-DERIVED on checkpoint load — NEVER serialized
    /// (it is not in `CheckpointState`), so `encode_state`, the
    /// `state_content_digest`, and the v4 checkpoint format are byte-unchanged.
    pub(crate) react_index: BTreeMap<[u8; kx_journal::INSTANCE_ID_LEN], Vec<usize>>,
    /// PR-2d-2 — DERIVED set of every react TURN's `MoteId` (each
    /// `ReactRound.turn_mote_id`). O(log n) membership for the coordinator's
    /// lease-time "is this Mote's parent a react turn?" check (the observation
    /// args-or-skip rule). Same derived/never-serialized contract as
    /// [`Self::react_index`].
    pub(crate) react_turn_motes: BTreeSet<MoteId>,
}

impl State {
    pub(crate) fn moteinfo_mut(&mut self, id: &MoteId) -> &mut MoteInfo {
        self.motes.entry(*id).or_default()
    }

    /// Index the LAST-pushed `react_rounds` record (PR-2d-2 — called by the
    /// fold right after the push, and by the checkpoint-load re-derivation, so
    /// both construction paths produce identical derived state and
    /// `State: PartialEq` comparisons hold).
    pub(crate) fn index_last_react_round(&mut self) {
        let Some(idx) = self.react_rounds.len().checked_sub(1) else {
            return;
        };
        let record = &self.react_rounds[idx];
        self.react_index
            .entry(record.instance_id)
            .or_default()
            .push(idx);
        self.react_turn_motes.insert(record.turn_mote_id);
    }

    /// `true` iff any declared Mote is a deterministic critic (`critic_for =
    /// Some`). Gates the P4.2-3 exit gate in `kx serve` so a critic-free run pays
    /// zero verdict-scan cost (PR-2c-3 critic-live). A pure fold read — `O(M)`,
    /// consulted once per lease poll, far cheaper than the `O(M²)` it guards.
    pub(crate) fn has_declared_critic(&self) -> bool {
        self.motes
            .values()
            .any(|i| i.declared.as_ref().is_some_and(|d| d.critic_for.is_some()))
    }

    /// `true` iff any `ReactRound` fact has folded — the run is (or was) a live
    /// ReAct chain. Gates the coordinator's `settle_react_rounds` and the F-7
    /// react trajectory special-case so react-free runs (and the demo) pay ZERO
    /// cost (PR-2d-1 react-substrate). O(1) — a Vec emptiness read.
    pub(crate) fn has_react_turn(&self) -> bool {
        !self.react_rounds.is_empty()
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
    ///
    /// A Mote can carry more than one anomaly (the cell-8 `inconsistent` flag and
    /// the M2.3b `quarantined` flag are independent, set-once facts), so each is
    /// emitted as its own `(MoteId, AnomalyKind)` entry. Entries are grouped by
    /// `MoteId` (the map iterates in key order); the per-Mote kind order is
    /// `inconsistent` then `quarantined`.
    pub(crate) fn anomaly_motes_iter(&self) -> Vec<(MoteId, AnomalyKind)> {
        self.motes
            .iter()
            .flat_map(|(id, info)| {
                let mut kinds: SmallVec<[AnomalyKind; 2]> = SmallVec::new();
                if info.inconsistent {
                    kinds.push(AnomalyKind::EffectStagedThenRepudiatedNoCommitted);
                }
                if info.quarantined {
                    kinds.push(AnomalyKind::QuarantinedAtLeastOnceEffect);
                }
                kinds.into_iter().map(move |k| (*id, k))
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

    /// Rebuild the entire child→parent reverse index from scratch — O(n) over
    /// every Mote. **No longer on the hot path** (D92, M2.1): `set_declared` and
    /// the `Committed` fold now use the incremental [`Self::reindex_child_edges`]
    /// helper, which touches only the edges of the Mote it introduces. This full
    /// rebuild is retained as the **differential oracle** — the
    /// `debug_assert!` in `reindex_child_edges` compares the incrementally-
    /// maintained index against a fresh rebuild on every mutation, and the
    /// inline unit + property tests assert byte-equality. Compiled only under
    /// `test` / `debug_assertions`.
    #[cfg(any(test, debug_assertions))]
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

    /// Insert one `(child, edge)` into `parent`'s adjacency list, preserving the
    /// sort-by-child-`MoteId` order the cascade walk
    /// ([`crate::helpers::transitive_consumers_impl`], the D22 poison-cascade)
    /// depends on. `partition_point(<= child)` returns the index one past the
    /// last entry whose child id is `<= child`, so the insert lands **after**
    /// any existing equal-child entry — matching the *stable* `sort_by(child)`
    /// in [`Self::rebuild_children_index`] byte-for-byte (the only source of
    /// equal-child entries is a child that declares the SAME parent twice with
    /// different `EdgeMeta`, e.g. a Data and a Control edge). Not idempotent on
    /// its own; callers go through [`Self::reindex_child_edges`], which clears a
    /// child's existing entries first.
    fn insert_child_edge(&mut self, parent: MoteId, child: MoteId, edge: EdgeMeta) {
        let v = self.children.entry(parent).or_default();
        let pos = v.partition_point(|probe| probe.0 <= child);
        v.insert(pos, (child, edge));
    }

    /// Remove every entry for `child` from `parent`'s adjacency list, dropping
    /// the parent key entirely if its list becomes empty — a from-scratch
    /// rebuild never leaves an empty-`Vec` key, so this keeps the map
    /// byte-identical to one.
    fn remove_child_entries(&mut self, parent: MoteId, child: MoteId) {
        if let Some(v) = self.children.get_mut(&parent) {
            v.retain(|(c, _)| *c != child);
            if v.is_empty() {
                self.children.remove(&parent);
            }
        }
    }

    /// Incrementally re-derive `child_id`'s outgoing edges in the reverse index
    /// after a state change — the O(parents·k) replacement for the per-mutation
    /// O(n) [`Self::rebuild_children_index`] (D92, M2.1).
    ///
    /// `old_effective` is [`Self::parents_of_id`] captured **before** the change
    /// — the edges `child_id` currently contributes to the index. This method
    /// removes exactly those, then inserts `child_id`'s NEW effective edges
    /// (`parents_of_id` read after the change). Because a child's effective
    /// parents change only via the declared-vs-committed precedence (a fresh
    /// `set_declared`, or a `Committed` for a Mote with no declared info), the
    /// before/after diff captures every edge transition the full rebuild would —
    /// including the register-after-commit case where the source flips from
    /// committed parents to declared parents. Inserts preserve parents-list
    /// order (stable, matching the rebuild). A `debug_assert!` verifies the
    /// result equals a full rebuild on every call (compiled out in release).
    fn reindex_child_edges(&mut self, child_id: MoteId, old_effective: &[(MoteId, EdgeMeta)]) {
        for (parent_id, _) in old_effective {
            self.remove_child_entries(*parent_id, child_id);
        }
        for (parent_id, edge) in self.parents_of_id(&child_id) {
            self.insert_child_edge(parent_id, child_id, edge);
        }
        #[cfg(debug_assertions)]
        debug_assert!(
            self.children_index_matches_full_rebuild(),
            "M2.1 invariant: incremental children index diverged from full rebuild"
        );
    }

    /// Set (overwrite) the declared info for `mote_id`, then incrementally update
    /// the reverse index. Captures the child's CURRENT effective parents (which
    /// may be declared- OR committed-derived) **before** the overwrite, so a
    /// re-registration that drops/changes a parent — or a registration that
    /// arrives after a committed-without-declare and flips the precedence to the
    /// new declared set — removes the stale edges (the full rebuild handled this
    /// implicitly).
    pub(crate) fn set_declared(&mut self, mote_id: MoteId, declared: DeclaredInfo) {
        let old_effective = self.parents_of_id(&mote_id);
        self.moteinfo_mut(&mote_id).declared = Some(declared);
        self.reindex_child_edges(mote_id, &old_effective);
    }

    /// Fold a `Committed` entry's edges into the reverse index incrementally.
    /// `old_effective` is the child's effective parents captured **before**
    /// `committed` was set. When the Mote was already declared, `parents_of_id`
    /// keeps returning the declared set (precedence) so this is a no-op re-derive;
    /// when it was committed-without-declare (pure `from_journal` recovery), the
    /// effective set flips from empty to the committed parents and those edges
    /// are inserted.
    pub(crate) fn index_committed(
        &mut self,
        mote_id: MoteId,
        old_effective: &[(MoteId, EdgeMeta)],
    ) {
        self.reindex_child_edges(mote_id, old_effective);
    }

    /// Differential oracle: does the incrementally-maintained `children` index
    /// equal a from-scratch [`Self::rebuild_children_index`]? Used ONLY inside
    /// the `debug_assert!` in [`Self::reindex_child_edges`] (compiled out in
    /// release — the scale test + bench run `--release` and pay zero). Clones to
    /// avoid mutating `self`. Compiled under `test` too so the inline + property
    /// tests can use it as an explicit oracle even under `cargo test --release`.
    #[cfg(any(test, debug_assertions))]
    fn children_index_matches_full_rebuild(&self) -> bool {
        let mut oracle = self.clone();
        oracle.rebuild_children_index();
        oracle.children == self.children
    }
}

#[cfg(test)]
mod incremental_index_tests {
    use super::{
        ContentRef, DeclaredInfo, EdgeMeta, EffectPattern, MoteId, NdClass, ParentRef, SmallVec,
        State,
    };

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    fn parent(id: u8, edge: EdgeMeta) -> ParentRef {
        ParentRef {
            parent_id: mid(id),
            edge,
        }
    }

    fn declared_with(parents: SmallVec<[ParentRef; 4]>) -> DeclaredInfo {
        DeclaredInfo {
            nd_class: NdClass::Pure,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            parents,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        }
    }

    fn parents(refs: &[ParentRef]) -> SmallVec<[ParentRef; 4]> {
        refs.iter().copied().collect()
    }

    #[test]
    fn insert_keeps_children_sorted_by_child_id() {
        let mut s = State::default();
        s.insert_child_edge(mid(1), mid(30), EdgeMeta::data());
        s.insert_child_edge(mid(1), mid(10), EdgeMeta::data());
        s.insert_child_edge(mid(1), mid(20), EdgeMeta::control());
        let ids: Vec<u8> = s.children[&mid(1)]
            .iter()
            .map(|(c, _)| c.as_bytes()[0])
            .collect();
        assert_eq!(
            ids,
            vec![10, 20, 30],
            "sorted by child MoteId regardless of insert order"
        );
    }

    #[test]
    fn insert_duplicate_child_appends_after_equal_stable() {
        // The only equal-child case: the same parent declared twice with
        // different edge meta (a Data and a Control edge to one parent).
        let mut s = State::default();
        s.insert_child_edge(mid(1), mid(10), EdgeMeta::data());
        s.insert_child_edge(mid(1), mid(10), EdgeMeta::control());
        assert_eq!(
            s.children[&mid(1)],
            vec![(mid(10), EdgeMeta::data()), (mid(10), EdgeMeta::control())],
            "stable: first-inserted precedes second, matching rebuild's stable sort"
        );
    }

    #[test]
    fn remove_drops_all_matching_and_empties_key() {
        let mut s = State::default();
        s.insert_child_edge(mid(1), mid(10), EdgeMeta::data());
        s.insert_child_edge(mid(1), mid(10), EdgeMeta::control());
        s.insert_child_edge(mid(1), mid(20), EdgeMeta::data());
        s.remove_child_entries(mid(1), mid(10));
        assert_eq!(s.children[&mid(1)], vec![(mid(20), EdgeMeta::data())]);
        s.remove_child_entries(mid(1), mid(20));
        assert!(
            !s.children.contains_key(&mid(1)),
            "empty parent key dropped to stay byte-identical to a fresh rebuild"
        );
    }

    #[test]
    fn set_declared_matches_full_rebuild() {
        let mut s = State::default();
        s.set_declared(
            mid(10),
            declared_with(parents(&[
                parent(1, EdgeMeta::data()),
                parent(2, EdgeMeta::control()),
            ])),
        );
        s.set_declared(
            mid(20),
            declared_with(parents(&[parent(1, EdgeMeta::data())])),
        );
        assert_eq!(
            s.children[&mid(1)],
            vec![(mid(10), EdgeMeta::data()), (mid(20), EdgeMeta::data())]
        );
        assert_eq!(s.children[&mid(2)], vec![(mid(10), EdgeMeta::control())]);
        assert!(s.children_index_matches_full_rebuild());
    }

    #[test]
    fn re_registration_with_changed_parents_removes_stale_edge() {
        let mut s = State::default();
        s.set_declared(
            mid(10),
            declared_with(parents(&[parent(1, EdgeMeta::data())])),
        );
        assert!(s.children.contains_key(&mid(1)));
        // Re-register child 10 with a DIFFERENT parent (2 replaces 1).
        s.set_declared(
            mid(10),
            declared_with(parents(&[parent(2, EdgeMeta::data())])),
        );
        assert!(
            !s.children.contains_key(&mid(1)),
            "stale edge under the dropped parent removed"
        );
        assert_eq!(s.children[&mid(2)], vec![(mid(10), EdgeMeta::data())]);
        assert!(s.children_index_matches_full_rebuild());
    }

    #[test]
    fn re_register_same_parents_is_idempotent() {
        let mut s = State::default();
        let p = parents(&[parent(1, EdgeMeta::data()), parent(2, EdgeMeta::control())]);
        s.set_declared(mid(10), declared_with(p.clone()));
        let before = s.children.clone();
        s.set_declared(mid(10), declared_with(p));
        assert_eq!(
            s.children, before,
            "re-registering identical parents is a no-op on the index"
        );
        assert!(s.children_index_matches_full_rebuild());
    }
}
