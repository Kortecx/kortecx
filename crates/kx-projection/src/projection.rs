//! [`Projection`] — the live in-memory state. Apply [`Projection::register_mote`]
//! for workflow-declared Motes, then [`Projection::fold`] each journal entry
//! in `seq` order; query via the 7-method read API.

use kx_content::ContentRef;
use kx_journal::{FailureReason, Journal, JournalEntry};
use kx_mote::{EdgeMeta, MoteId, NdClass};
use smallvec::SmallVec;

use crate::checkpoint::{CheckpointOutcome, FoldCheckpoint, FullFoldReason};
use crate::enums::{AnomalyKind, MoteState, PromotionState};
use crate::errors::ProjectionError;
use crate::helpers::{promotion_state_impl, ready_set_impl, transitive_consumers_impl};
use crate::materializer::TopologyMaterializer;
use crate::register::RegisterMote;
use crate::snapshot::Snapshot;
use crate::state::{CommittedInfo, DeclaredInfo, State};

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
#[derive(Default)]
pub struct Projection {
    state: State,
    /// **P1.11 / D48 + D49.** Optional topology materializer invoked on
    /// every `Committed` journal entry. If `Some`, the fold invokes
    /// `try_materialize` to decode any shaper's `TopologyDecision`
    /// payload and register the materialized children. If `None`, shaper
    /// commits fold as ordinary Committed entries with no child
    /// materialization (the legacy / test path).
    ///
    /// Production callers MUST set this via
    /// [`Projection::with_materializer`]. Tests that don't exercise
    /// topology may use [`Projection::new`].
    materializer: Option<Box<dyn TopologyMaterializer>>,
}

// Manual Debug so we don't require `Debug` on the materializer trait
// (which would foreclose blanket `Box<dyn Fn ...>`-style impls).
impl std::fmt::Debug for Projection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projection")
            .field("state", &self.state)
            .field(
                "materializer",
                &self.materializer.as_ref().map(|_| "<materializer>"),
            )
            .finish()
    }
}

impl Projection {
    /// Construct an empty projection with NO topology materializer.
    ///
    /// Shaper commits fold without materializing children. Suitable for
    /// tests that don't exercise topology and for the legacy code path
    /// pre-PR-11. Production callers — especially those that may see
    /// shaper-committed entries on cold re-fold — MUST use
    /// [`Projection::with_materializer`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an empty projection wired with a topology materializer
    /// (D48 + D49 / P1.11).
    ///
    /// On every `Committed` journal entry the fold invokes
    /// `materializer.try_materialize(...)`. If the materializer
    /// determines the entry is a shaper commit, the resolved children
    /// are immediately registered via [`Projection::register_mote`].
    /// Replay-faithfulness (R49) — re-folding the same log produces
    /// bit-identical children — is the materializer's responsibility;
    /// see `docs/design/decisions.md` §D49 (private corpus) and the
    /// `tests/cold_refold_topology.rs` P1+P2+P3+P4 proof.
    #[must_use]
    pub fn with_materializer(materializer: Box<dyn TopologyMaterializer>) -> Self {
        Self {
            state: State::default(),
            materializer: Some(materializer),
        }
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

    /// Build a projection by reading every entry from `journal` in `seq` order,
    /// wired with a topology materializer (D48 + D49 / P1.11).
    ///
    /// The cold-re-fold equivalent of [`Projection::with_materializer`] +
    /// [`Projection::fold_many`]. **Load-bearing for replay-faithfulness**: a
    /// re-fold from journal must produce bit-identical children to a live fold;
    /// the R49 proof (`tests/cold_refold_topology.rs`) anchors this.
    pub fn from_journal_with_materializer<J: Journal>(
        journal: &J,
        materializer: Box<dyn TopologyMaterializer>,
    ) -> Result<Self, ProjectionError> {
        let mut p = Self::with_materializer(materializer);
        let max_seq = journal.current_seq()?;
        let entries = journal.read_entries_by_seq(0..(max_seq + 1))?;
        for entry in entries {
            p.fold(&entry)?;
        }
        Ok(p)
    }

    /// Cold recovery that **resumes from a discardable [`FoldCheckpoint`]** when
    /// one is usable (D92(b), M2.2): seed the folded state from the checkpoint and
    /// fold only the tail `(checkpoint_offset, current]` instead of `(0, current]`.
    ///
    /// **Fail-safe.** The checkpoint is *never authoritative* — on any anomaly
    /// (failed integrity check, unsupported version/codec, decode failure, an
    /// offset past the journal head, an encoded `last_seq` that disagrees with the
    /// offset, a run-id that does not match the journal's `RunRegistered`, or —
    /// M2.2c — a seeded digest that is not anchored by a matching journaled
    /// `DigestSealed` seal) this silently **discards the checkpoint and runs the
    /// full fold**. Passing `None` is always a full fold. The result is
    /// bit-identical to [`Self::from_journal`] either way — the checkpoint only
    /// changes *how much* is re-folded, never the outcome.
    pub fn from_journal_with_checkpoint<J: Journal>(
        journal: &J,
        checkpoint: Option<&FoldCheckpoint>,
    ) -> Result<Self, ProjectionError> {
        Self::build_from_journal(journal, checkpoint, None).map(|(p, _)| p)
    }

    /// As [`Self::from_journal_with_checkpoint`], but also returns a
    /// [`CheckpointOutcome`] recording whether the checkpoint seeded the fold
    /// (and the tail length) or was discarded (and why). The folded projection is
    /// identical to the non-reported variant — the outcome is purely diagnostic,
    /// for recovery observability and tests.
    pub fn from_journal_with_checkpoint_reported<J: Journal>(
        journal: &J,
        checkpoint: Option<&FoldCheckpoint>,
    ) -> Result<(Self, CheckpointOutcome), ProjectionError> {
        Self::build_from_journal(journal, checkpoint, None)
    }

    /// The materializer-wired counterpart of [`Self::from_journal_with_checkpoint`]
    /// (the cold-re-fold equivalent of [`Self::from_journal_with_materializer`]).
    ///
    /// On a checkpoint hit the materialized children committed at `seq ≤ offset`
    /// are restored from the seeded state; the materializer fires **only** for
    /// shaper commits in the tail `(offset, current]`, so no child is
    /// re-materialized. On any anomaly it falls back to a full
    /// [`Self::from_journal_with_materializer`].
    pub fn from_journal_with_checkpoint_with_materializer<J: Journal>(
        journal: &J,
        materializer: Box<dyn TopologyMaterializer>,
        checkpoint: Option<&FoldCheckpoint>,
    ) -> Result<Self, ProjectionError> {
        Self::build_from_journal(journal, checkpoint, Some(materializer)).map(|(p, _)| p)
    }

    /// As [`Self::from_journal_with_checkpoint_with_materializer`], but also returns
    /// the [`CheckpointOutcome`] (see [`Self::from_journal_with_checkpoint_reported`]).
    pub fn from_journal_with_checkpoint_with_materializer_reported<J: Journal>(
        journal: &J,
        materializer: Box<dyn TopologyMaterializer>,
        checkpoint: Option<&FoldCheckpoint>,
    ) -> Result<(Self, CheckpointOutcome), ProjectionError> {
        Self::build_from_journal(journal, checkpoint, Some(materializer))
    }

    /// Shared body for the checkpoint-aware cold-recovery entry points: try to
    /// seed from the checkpoint, then fold the remaining tail; fall back to a
    /// full fold (`start_exclusive = 0`) when the checkpoint is unusable. Also
    /// returns the [`CheckpointOutcome`] (Seeded vs FullFold + reason).
    fn build_from_journal<J: Journal>(
        journal: &J,
        checkpoint: Option<&FoldCheckpoint>,
        materializer: Option<Box<dyn TopologyMaterializer>>,
    ) -> Result<(Self, CheckpointOutcome), ProjectionError> {
        let current = journal.current_seq()?;
        let seed = match checkpoint {
            Some(cp) => Self::try_seed_state(journal, cp, current)?,
            None => Err(FullFoldReason::NoCheckpoint),
        };
        let (mut p, start_exclusive, outcome) = match seed {
            Ok(state) => {
                let offset = state.last_seq;
                (
                    Self {
                        state,
                        materializer,
                    },
                    offset,
                    CheckpointOutcome::Seeded {
                        offset,
                        tail_entries: current.saturating_sub(offset),
                    },
                )
            }
            Err(reason) => (
                Self {
                    state: State::default(),
                    materializer,
                },
                0,
                CheckpointOutcome::FullFold { reason },
            ),
        };
        for entry in journal.read_entries_by_seq((start_exclusive + 1)..(current + 1))? {
            p.fold(&entry)?;
        }
        Ok((p, outcome))
    }

    /// Validate a checkpoint against the journal and decode its seeded state, or
    /// return `Ok(Err(reason))` to signal "discard and full-fold" (the reason
    /// names the gate that rejected it). Journal I/O errors propagate; every
    /// *checkpoint* defect is a graceful `Err(FullFoldReason)`.
    fn try_seed_state<J: Journal>(
        journal: &J,
        cp: &FoldCheckpoint,
        current: u64,
    ) -> Result<Result<State, FullFoldReason>, ProjectionError> {
        // (1) integrity — the envelope digest must verify.
        if !cp.verify() {
            return Ok(Err(FullFoldReason::IntegrityFailed));
        }
        // (2) the offset must not run past the journal head (stale / truncated log).
        if cp.journal_offset() > current {
            return Ok(Err(FullFoldReason::OffsetAheadOfHead));
        }
        // (3) decode the payload; a malformed/hostile blob is discarded.
        let Ok(state) = cp.decode_state() else {
            return Ok(Err(FullFoldReason::DecodeFailed));
        };
        // (4) the encoded frontier must equal the declared offset (consistency).
        if state.last_seq != cp.journal_offset() {
            return Ok(Err(FullFoldReason::OffsetMismatch));
        }
        // (5) wrong-run guard (best effort): if both name a run, the ids must match.
        if let Some(reg) = state.run_registration {
            if let Some(journal_instance) = Self::journal_run_instance(journal)? {
                if journal_instance != reg.instance_id {
                    return Ok(Err(FullFoldReason::WrongRun));
                }
            }
        }
        // (6) **M2.2c (D103.2) — unforgeability anchor.** The seeded state's
        // digest MUST equal a `state_digest` journaled IN the trust root. The
        // seed's digest at frontier `S = cp.journal_offset()` (== `state.last_seq`,
        // gate 4) is `blake3(payload)` (`payload_state_digest`) — the canonical
        // encoding the writer also sealed; no re-encode of the decoded state, and
        // it is `last_seq`-correct because the payload was captured at frontier S.
        // We compare it to the `DigestSealed{through_seq == S}` the writer
        // co-committed at `S + 1`. A missing or mismatched seal discards the
        // checkpoint and full-folds — a forged-but-self-consistent sidecar (the
        // D103.1 residual) cannot seed a wrong base state, because forging the seal
        // requires forging the journal. Fail-closed; recovery is bit-identical to
        // a full fold either way.
        let offset = cp.journal_offset();
        match Self::journal_seal_at(journal, offset)? {
            None => return Ok(Err(FullFoldReason::SealMissing)),
            Some(sealed_digest) => {
                if sealed_digest != cp.payload_state_digest() {
                    return Ok(Err(FullFoldReason::SealMismatch));
                }
            }
        }
        Ok(Ok(state))
    }

    /// The `state_digest` journaled by a `DigestSealed{through_seq == through}`
    /// seal, if one is present at `seq = through + 1` (where the single-writer
    /// runtime co-commits it with the checkpoint at frontier `through`). `None`
    /// if no matching seal is there. Used by the M2.2c unforgeability gate.
    fn journal_seal_at<J: Journal>(
        journal: &J,
        through: u64,
    ) -> Result<Option<[u8; 32]>, ProjectionError> {
        for entry in journal.read_entries_by_seq((through + 1)..(through + 2))? {
            if let JournalEntry::DigestSealed {
                through_seq,
                state_digest,
                ..
            } = entry
            {
                if through_seq == through {
                    return Ok(Some(state_digest));
                }
            }
        }
        Ok(None)
    }

    /// The journal's registered run instance id, read from the `RunRegistered`
    /// fact (M1.1 establishes it at `seq = 1`). `None` if the journal carries no
    /// such fact (e.g. a test/legacy log) — then the wrong-run guard is skipped.
    fn journal_run_instance<J: Journal>(
        journal: &J,
    ) -> Result<Option<[u8; kx_journal::INSTANCE_ID_LEN]>, ProjectionError> {
        for entry in journal.read_entries_by_seq(1..2)? {
            if let JournalEntry::RunRegistered { instance_id, .. } = entry {
                return Ok(Some(instance_id));
            }
        }
        Ok(None)
    }

    /// Register a workflow-declared Mote.
    ///
    /// Adds the Mote to the projection's expected set; its state becomes
    /// [`MoteState::Pending`] until the first journal entry lands. Re-registration
    /// of the same `MoteId` is permitted and overwrites the declared info (the
    /// workflow author may update parents before submission; the journal-side
    /// dedupe-by-key path is the authoritative arbiter for identity equality).
    pub fn register_mote(&mut self, reg: RegisterMote) {
        let declared = DeclaredInfo {
            nd_class: reg.nd_class,
            effect_pattern: reg.effect_pattern,
            critic_for: reg.critic_for,
            is_topology_shaper: reg.is_topology_shaper,
            parents: reg.parents,
            warrant_ref: reg.warrant_ref,
        };
        // D92 / M2.1: incremental children-index update (was a full O(n)
        // `rebuild_children_index`). `set_declared` captures the prior declared
        // parent set before overwrite so a re-registration that changes parents
        // removes the stale edges.
        self.state.set_declared(reg.mote_id, declared);
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
                // D92 / M2.1: capture the child's CURRENT effective parents
                // (the edges it contributes to the index now) BEFORE setting
                // `committed`, so the incremental re-index can diff old→new. For
                // a Mote already declared this is the declared set (a no-op
                // re-derive, declared keeps precedence); for a committed-without-
                // declare Mote (pure recovery) it is empty and the committed
                // parents are inserted fresh.
                let old_effective = self.state.parents_of_id(mote_id);
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
                // D92 / M2.1: incrementally fold THIS Mote's edges into the
                // reverse index (was a full O(n) `rebuild_children_index` per
                // commit — the resume-availability O(n²) wall). Reads parents via
                // `parents_of_id` (declared precedence + the same `to_parent_ref`
                // filter as the rebuild) and diffs against `old_effective`.
                self.state.index_committed(*mote_id, &old_effective);

                // **P1.11 / D48 + D49 + PR 11.5 KG-1-close.** Topology
                // materializer hook. If a materializer is wired AND the
                // Mote is a shaper, fetch + decode the TopologyDecision
                // payload + the shaper's WarrantSpec from the content
                // store, narrow each child's warrant per D30
                // intersect(shaper.warrant, role.spec), and register
                // every materialized child with its per-role-narrowed
                // warrant_ref. R49 requires this to be deterministic
                // across replay; the materializer's purity guarantee
                // delivers that (registry lookups are stable per the
                // RoleRegistry contract).
                let materialized = match self.materializer.as_ref() {
                    Some(m) => {
                        m.try_materialize(*mote_id, *mote_def_hash, *result_ref, *warrant_ref)?
                    }
                    None => None,
                };
                if let Some(children) = materialized {
                    for reg in children {
                        self.register_mote(reg);
                    }
                }
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
                        // **PR-3 (AL2).** Retain the terminal reason (first-wins,
                        // prefix-monotonic) so a model-driven re-plan can read WHY
                        // a step dead-lettered. Pure read-side: off-digest, off-id,
                        // off-DAG (the demo never folds a terminal Failed, so this
                        // stays None and the canonical digest is byte-unchanged).
                        if info.failure_reason.is_none() {
                            info.failure_reason = Some(*reason_class);
                        }
                    }
                    // **v6 (M2.3b, D105.4).** A recovery-time quarantine of a
                    // staged-uncommitted at-most-once effect: mark it so
                    // `anomaly_motes` surfaces it for operator review. Set-once;
                    // terminal_failure_observed is already set above (the variant
                    // is not pre-commit-crash), so it is non-redispatchable too.
                    if *reason_class == kx_journal::FailureReason::QuarantinedAtLeastOnce {
                        info.quarantined = true;
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
            // Off-DAG run-metadata facts (extracted to keep `fold` under the
            // line budget): each records an O(1) field and NEVER touches
            // `rebuild_children_index` (the per-mutation O(n²) D92 path).
            // `DigestSealed` (M2.2c) is a pure `last_seq`-only frontier advance —
            // it names no Mote, registers no `MoteInfo`, and is verified at
            // recovery (in `try_seed_state`), never materialized into state.
            // `ReplanRound` (PR-2c-2) appends a recovery/audit record + advances
            // `last_seq`; it is NOT folded into any digest. `ReactRound` (PR-2d-1)
            // is its ReAct-chain sibling — same off-DAG, never-a-digest-input law.
            JournalEntry::RunRegistered { .. }
            | JournalEntry::RunVersionsResolved { .. }
            | JournalEntry::DigestSealed { .. }
            | JournalEntry::ReplanRound { .. }
            | JournalEntry::ReactRound { .. } => {
                self.fold_run_metadata(entry);
            }
        }
        Ok(prev)
    }

    /// Fold an off-DAG run-metadata fact (`RunRegistered` / `RunVersionsResolved`
    /// / `DigestSealed` / `ReplanRound` / `ReactRound`).
    ///
    /// None of these name a Mote, so this registers NO `MoteInfo` and does NOT
    /// call `rebuild_children_index` — O(1), off the Mote-DAG. The data is
    /// **metadata, never identity**: no scheduling/identity/digest decision reads
    /// it (`DigestSealed` in particular is invisible to the run-identity product
    /// digest, which folds only `Committed` Motes).
    fn fold_run_metadata(&mut self, entry: &JournalEntry) {
        match entry {
            JournalEntry::RunRegistered {
                instance_id,
                recipe_fingerprint,
                seq,
                ..
            } => {
                // v3 (M1.1, D63/D64). Idempotent on replay: the seq=1 entry
                // replays the same bytes, so re-folding sets the same value.
                // `ts` is audit-only and ignored here.
                self.state.run_registration = Some(crate::state::RunRegistration {
                    instance_id: *instance_id,
                    recipe_fingerprint: *recipe_fingerprint,
                });
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::RunVersionsResolved {
                instance_id,
                warrant_ref,
                model_id,
                capability,
                seq,
            } => {
                // v4 (M1.2, D79). Append-many: a run accrues one record per
                // resolved capability. Replay rebuilds the same Vec from scratch
                // (each journaled entry folds exactly once).
                self.state
                    .run_resolved_versions
                    .push(crate::state::RunResolvedVersions {
                        instance_id: *instance_id,
                        warrant_ref: *warrant_ref,
                        model_id: model_id.clone(),
                        capability: capability.clone(),
                    });
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::DigestSealed { seq, .. } => {
                // v5 (M2.2c, D103.2). A pure frontier advance: the seal writes
                // NOTHING into `State` (so it never enters `state_digest()` — that
                // would be a chicken-and-egg cycle — and never the product digest).
                // Its digest is verified at recovery in `try_seed_state`, not here.
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::ReplanRound {
                round,
                shaper_mote_id,
                base_prompt_ref,
                corrected_prompt_ref,
                warrant_ref,
                model_id,
                failed_steps,
                escalation_reason_ref,
                seq,
            } => {
                // v7 (PR-2c-2). Append-many: a run accrues one record per re-plan
                // round. Replay rebuilds the same Vec from scratch (each journaled
                // entry folds exactly once). Off-DAG: names a shaper Mote but
                // registers NO `MoteInfo` and touches NO children index — it is pure
                // recovery/audit metadata, never an identity/scheduling/digest input.
                self.state
                    .replan_rounds
                    .push(crate::state::ReplanRoundRecord {
                        round: *round,
                        shaper_mote_id: *shaper_mote_id,
                        base_prompt_ref: *base_prompt_ref,
                        corrected_prompt_ref: *corrected_prompt_ref,
                        warrant_ref: *warrant_ref,
                        model_id: model_id.clone(),
                        failed_steps: failed_steps.to_vec(),
                        escalation_reason_ref: *escalation_reason_ref,
                        seq: *seq,
                    });
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            JournalEntry::ReactRound {
                turn,
                turn_mote_id,
                instance_id,
                base_prompt_ref,
                warrant_ref,
                model_id,
                branch,
                max_turns,
                max_tool_calls,
                step_salt,
                seq,
            } => {
                // v8 (PR-2d-1). Append-many: a run accrues one record per turn
                // anchor/settle/advance. Replay rebuilds the same Vec from scratch
                // (each journaled entry folds exactly once). Off-DAG: names a turn
                // Mote but registers NO `MoteInfo` and touches NO children index —
                // pure recovery/audit metadata, never an identity/scheduling/digest
                // input. Vec non-emptiness IS the `has_react_turn` sentinel.
                self.state
                    .react_rounds
                    .push(crate::state::ReactRoundRecord {
                        turn: *turn,
                        turn_mote_id: *turn_mote_id,
                        instance_id: *instance_id,
                        base_prompt_ref: *base_prompt_ref,
                        warrant_ref: *warrant_ref,
                        model_id: model_id.clone(),
                        branch: branch.clone(),
                        max_turns: *max_turns,
                        max_tool_calls: *max_tool_calls,
                        step_salt: *step_salt,
                        seq: *seq,
                    });
                // PR-2d-2: maintain the DERIVED per-instance index + turn-Mote
                // set (never serialized — checkpoint/digest byte-unchanged).
                self.state.index_last_react_round();
                self.state.last_seq = self.state.last_seq.max(*seq);
            }
            _ => unreachable!("fold_run_metadata called with a non-run-metadata kind"),
        }
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

    /// The canonical **full-state digest** of the current fold — a deterministic
    /// blake3 over a canonical encoding of the entire projection state (every
    /// Mote's declared/committed/flag fields, the children index, `last_seq`, and
    /// the run metadata).
    ///
    /// This is the digest a [`FoldCheckpoint`] embeds, and the digest the roadmap
    /// journaled seal (M2.2c) will store + verify recovery against. It is
    /// **distinct** from `kx-runtime`'s committed-facts *product* digest (the
    /// canonical run-identity `7d22d4bd…`): this one covers the *whole* state for
    /// recovery integrity, never run identity. Exact-equality only (SN-8).
    #[must_use]
    pub fn state_digest(&self) -> [u8; 32] {
        crate::checkpoint::state_content_digest(&self.state)
    }

    /// Capture a discardable [`FoldCheckpoint`] of the current fold (D92(b), M2.2).
    ///
    /// **Caller invariant:** only checkpoint a projection that has been folded
    /// **contiguously over `[1, last_seq]`** (no skipped `seq ≤ last_seq`, no entry
    /// `seq > last_seq` applied). [`Self::from_journal`] /
    /// [`Self::from_journal_with_materializer`] always satisfy this; an incremental
    /// caller must checkpoint only at a drained frontier. The checkpoint's
    /// `journal_offset` is `last_seq`; recovery folds `(offset, current]` on top.
    #[must_use]
    pub fn fold_checkpoint(&self) -> FoldCheckpoint {
        let cp = FoldCheckpoint::from_state(&self.state);
        // Capture-time oracle (compiled out in release): the checkpoint must
        // decode back to this exact state.
        debug_assert!(
            matches!(cp.decode_state(), Ok(s) if s == self.state),
            "fold_checkpoint round-trip diverged from the source State"
        );
        cp
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

    /// The Mote's committed non-determinism tag ([`NdClass`]), if it has a
    /// `Committed` entry; `None` otherwise.
    ///
    /// Mirrors [`Projection::result_ref_of`]. The tag drives P1.12 tag-driven
    /// storage tiering: `kx-tiering` joins this with `result_ref_of` to decide
    /// evictability (PURE payloads are droppable + recomputable; READ-ONLY-NONDET
    /// and WORLD-MUTATING are always persisted) without exposing `CommittedInfo`.
    #[must_use]
    pub fn nondeterminism_of(&self, mote_id: &MoteId) -> Option<NdClass> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.nondeterminism))
    }

    /// The Mote's terminal [`FailureReason`] if it dead-lettered (a TERMINAL
    /// `Failed` was folded — not a pre-commit-crash); `None` otherwise (incl. a
    /// never-failed or a committed Mote, and a pre-commit-crash `Failed`).
    ///
    /// **PR-3 (AL2).** The minimal read-side surface a model-driven re-plan reads
    /// to learn WHY a step failed (corrected-context), and an operator reads for
    /// triage. Mirrors [`Projection::result_ref_of`] — a pure O(1) lookup. Returns
    /// ONLY the closed, low-entropy reason enum: never the failed Mote's result
    /// bytes or warrant secrets (a safe action-selection input, SN-8 / D77).
    #[must_use]
    pub fn failure_reason_of(&self, mote_id: &MoteId) -> Option<FailureReason> {
        self.state.motes.get(mote_id).and_then(|i| i.failure_reason)
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

    /// The ready set with the **P4.2-3 deterministic-critic promotion gate**
    /// active: a WORLD-MUTATING producer's consumers are withheld until a
    /// committed critic (declared `critic_for = producer`) returns a `Valid`
    /// [`kx_critic_types::CriticVerdict`], read by content-address through
    /// `verdicts`. This is the **P4 EXIT GATE** — a deterministic critic gating
    /// a world-mutating step. Additive to [`Self::ready_set`] (which keeps the
    /// P1 `NotApplicable` default).
    #[must_use]
    pub fn ready_set_promoted(
        &self,
        verdicts: &dyn crate::promotion::VerdictLookup,
    ) -> Vec<MoteId> {
        crate::helpers::ready_set_impl_with(&self.state, &|s, id| {
            crate::promotion::promotion_state_with(s, id, verdicts)
        })
    }

    /// The ready set with the **P4.2-3 critic exit-gate auto-activated**
    /// (PR-2c-3 critic-live — the live `kx serve` lease + ReadySet entry point).
    ///
    /// - **No critic declared in the run** ⇒ exactly [`Self::ready_set`] (the P1
    ///   `NotApplicable` default): a critic-free DAG NEVER pays the verdict scan, so
    ///   the canonical demo + every critic-free run are byte-for-byte unchanged.
    /// - **A critic IS declared, `Some(verdicts)`** ⇒ [`Self::ready_set_promoted`]
    ///   (the live exit gate, fed by a content-addressed verdict lookup).
    /// - **A critic IS declared, `None`** ⇒ FAIL-CLOSED: every critic-gated consumer
    ///   is withheld (a critic exists but no store can resolve its verdict, so the
    ///   gate cannot prove `Valid`). Gating on the *folded* `has_declared_critic`
    ///   (not on whether a store handle is present) keeps the ready set a pure,
    ///   deterministic fold of the journal — it can never fail OPEN (B2).
    #[must_use]
    pub fn ready_set_auto(
        &self,
        verdicts: Option<&dyn crate::promotion::VerdictLookup>,
    ) -> Vec<MoteId> {
        if !self.state.has_declared_critic() {
            return self.ready_set();
        }
        match verdicts {
            Some(v) => self.ready_set_promoted(v),
            None => self.ready_set_promoted(&crate::promotion::NoVerdicts),
        }
    }

    /// `true` iff any declared Mote is a deterministic critic (`critic_for =
    /// Some`). Gates [`Self::ready_set_auto`] so a critic-free run pays zero
    /// exit-gate cost. A pure fold of the journal (PR-2c-3 critic-live).
    #[must_use]
    pub fn has_declared_critic(&self) -> bool {
        self.state.has_declared_critic()
    }

    /// The verdict-resolved promotion state of `producer_id` (P4.2-3). Unlike
    /// [`Self::promotion_state`] (the P1 `NotApplicable` stub), this reads
    /// committed critic verdicts via `verdicts`.
    #[must_use]
    pub fn promotion_state_resolved(
        &self,
        producer_id: &MoteId,
        verdicts: &dyn crate::promotion::VerdictLookup,
    ) -> PromotionState {
        crate::promotion::promotion_state_with(&self.state, producer_id, verdicts)
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

    /// The Mote's [`ContentRef`]-keyed warrant ref.
    ///
    /// **PR 11.5 / KG-1-close.** Returns the declared `warrant_ref` if the
    /// Mote was registered (via [`Projection::register_mote`] or the
    /// topology materializer's child registration), else the committed
    /// entry's `warrant_ref` if any, else `None`.
    ///
    /// The dispatch path (executor, P1.9+) reads this to look up the
    /// per-Mote [`kx_warrant::WarrantSpec`] from the content store. For
    /// shaper-materialized children, this ref points at the *narrowed*
    /// warrant computed via D30's `intersect(shaper.warrant, role.spec)`
    /// — closing `topology.md` §13 KG-1's verbatim-inheritance gap.
    #[must_use]
    pub fn warrant_ref_of(&self, mote_id: &MoteId) -> Option<ContentRef> {
        self.state.motes.get(mote_id).and_then(|i| {
            i.declared
                .as_ref()
                .map(|d| d.warrant_ref)
                .or_else(|| i.committed.as_ref().map(|c| c.warrant_ref))
        })
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

    /// The registered run identity (D64) as `(instance_id, recipe_fingerprint)`,
    /// or `None` if no `RunRegistered` entry has been folded.
    ///
    /// Set when the run's seq=1 `RunRegistered` entry is folded; read on replay,
    /// never recomputed. Off the Mote-DAG (does not gate scheduling); for M1.1
    /// it is a queryable run-identity marker (M1.2 metadata + the catalog build
    /// on it).
    #[must_use]
    pub fn run_registration(&self) -> Option<([u8; kx_journal::INSTANCE_ID_LEN], [u8; 32])> {
        self.state
            .run_registration
            .map(|r| (r.instance_id, r.recipe_fingerprint))
    }

    /// The resolved-version run metadata (D79) folded so far — one record per
    /// `RunVersionsResolved` entry (one per resolved capability; a zero-grant
    /// warrant contributes one with `capability == None`).
    ///
    /// **Audit/lineage metadata, never identity.** Off the Mote-DAG: no
    /// scheduling/identity/digest decision reads it, so it can never move the
    /// projection digest. Reconstructed verbatim on replay.
    #[must_use]
    pub fn run_resolved_versions(&self) -> &[crate::state::RunResolvedVersions] {
        &self.state.run_resolved_versions
    }

    /// The re-plan-round metadata (PR-2c-2) folded so far — one record per
    /// `ReplanRound` entry, in journal (round) order.
    ///
    /// **Recovery + audit metadata, never identity.** Off the Mote-DAG: no
    /// scheduling/identity/digest decision reads it, so it can never move the
    /// projection digest. Reconstructed verbatim on replay. The coordinator's
    /// `recover_replan_chain` reads these to rebuild each round's shaper Mote
    /// deterministically from committed facts.
    #[must_use]
    pub fn replan_rounds(&self) -> &[crate::state::ReplanRoundRecord] {
        &self.state.replan_rounds
    }

    /// The highest-`round` `ReplanRound` record folded so far, or `None` if the run
    /// has driven no re-plan rounds. The live coordinator uses this for the
    /// next-round budget check + the submitted-but-uncommitted dedup window; on a
    /// tie (a defensive duplicate `round`) the lowest-`seq` record wins so recovery
    /// is deterministic.
    #[must_use]
    pub fn latest_replan_round(&self) -> Option<&crate::state::ReplanRoundRecord> {
        self.state
            .replan_rounds
            .iter()
            .max_by(|a, b| a.round.cmp(&b.round).then(b.seq.cmp(&a.seq)))
    }

    /// The ReAct-turn metadata (PR-2d-1) folded so far — one record per
    /// `ReactRound` entry, in journal (seq) order.
    ///
    /// **Recovery + audit metadata, never identity.** Off the Mote-DAG: no
    /// scheduling/identity/digest decision reads it, so it can never move the
    /// projection digest. Reconstructed verbatim on replay. The coordinator's
    /// `settle_react_rounds` / `recover_react_chain` read these to settle the
    /// latest turn, re-derive budget counters from branches, and rebuild an
    /// in-flight turn's Mote deterministically from committed facts.
    #[must_use]
    pub fn react_rounds(&self) -> &[crate::state::ReactRoundRecord] {
        &self.state.react_rounds
    }

    /// The `ReactRound` records of ONE chain (`instance_id`), in journal (seq)
    /// order — served off the DERIVED per-instance index (PR-2d-2), so a
    /// per-chain read costs O(that chain's facts), never a scan over every
    /// chain in serve's shared journal (the PR-2d-1 O(runs²) finding).
    pub fn react_rounds_of(
        &self,
        instance_id: &[u8; kx_journal::INSTANCE_ID_LEN],
    ) -> impl Iterator<Item = &crate::state::ReactRoundRecord> + '_ {
        self.state
            .react_index
            .get(instance_id)
            .into_iter()
            .flatten()
            .map(|&idx| &self.state.react_rounds[idx])
    }

    /// The distinct `instance_id`s with folded react facts, ascending — each an
    /// independent chain in serve's SHARED journal. Served off the index keys
    /// (PR-2d-2): O(chains), not O(total facts).
    pub fn react_instances(&self) -> impl Iterator<Item = &[u8; kx_journal::INSTANCE_ID_LEN]> + '_ {
        self.state.react_index.keys()
    }

    /// `true` iff `id` is a react TURN's `MoteId` (some folded `ReactRound`
    /// names it as its `turn_mote_id`). O(log n) off the derived set (PR-2d-2)
    /// — the coordinator's lease-time observation check.
    #[must_use]
    pub fn is_react_turn_mote(&self, id: &kx_mote::MoteId) -> bool {
        self.state.react_turn_motes.contains(id)
    }

    /// The highest-`turn` `ReactRound` record for `instance_id` folded so far, or
    /// `None` if that run has anchored no ReAct chain. Scoped by `instance_id`
    /// (the run-salt) because serve's journal is SHARED across runs. On a turn
    /// tie the highest-`seq` record wins — the LATEST fact for a turn is its
    /// settled branch (anchor `Pending` then a resolution), so recovery reads
    /// the freshest decision deterministically.
    #[must_use]
    pub fn latest_react_round(
        &self,
        instance_id: &[u8; kx_journal::INSTANCE_ID_LEN],
    ) -> Option<&crate::state::ReactRoundRecord> {
        self.react_rounds_of(instance_id)
            .max_by(|a, b| a.turn.cmp(&b.turn).then(a.seq.cmp(&b.seq)))
    }

    /// `true` iff any `ReactRound` fact has folded (PR-2d-1). Gates the
    /// coordinator's react settle/recover/F-7 special-cases so a react-free run
    /// pays zero cost — the `has_declared_critic` precedent, but O(1).
    #[must_use]
    pub fn has_react_turn(&self) -> bool {
        self.state.has_react_turn()
    }

    /// The durable resolved [`kx_journal::IdempotencyClassTag`] for a tool, folded
    /// from the run's `RunVersionsResolved` metadata (M2.3b, D105.4). `None` if no
    /// resolved record names this tool (e.g. a run that journaled no resolution —
    /// the single-node demo path — or a tool outside the run's grants).
    ///
    /// This is the durable source the class-aware recovery decision reads: the
    /// resolved class is otherwise transient (used at submit for the R-10 refusal,
    /// then dropped), so a crash-recovered run could only safely re-dispatch
    /// Token-class effects without it. A tool resolves to exactly one class per
    /// run; the first matching record wins (records for one run share a class per
    /// tool by construction).
    ///
    /// **Audit/lineage metadata, never identity** — like [`Self::run_resolved_versions`],
    /// reading it moves no digest and gates no scheduling/identity decision (it
    /// only informs the recovery action).
    #[must_use]
    pub fn idempotency_class_for_tool(
        &self,
        tool_id: &str,
    ) -> Option<kx_journal::IdempotencyClassTag> {
        self.state
            .run_resolved_versions
            .iter()
            .filter_map(|r| r.capability.as_ref())
            .find(|cap| cap.tool_id == tool_id)
            .map(|cap| cap.idempotency_class)
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
