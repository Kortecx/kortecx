//! Commit-protocol error surface + `StandardCommitProtocol` impl for the
//! atomic stage→commit path.
//!
//! **PR 9b-2** shipped the closed `CommitProtocolError` vocabulary
//! (R-11 / R-12 / R-13 per `docs/design/validate-then-commit.md` §7 + D38
//! §2b + D39 §a/§c/§d) plus the `CommitProtocol` trait scaffolding.
//!
//! **PR 9b-3** (this PR) ships `StandardCommitProtocol<S, J, B>` — the
//! per-`EffectPattern` impl for the `IdempotentByConstruction` path:
//! `broker.dispatch → R-11 verify → journal.append(Committed)` (per D39
//! §a/§c). The `StageThenCommit` and `ValidateThenCommit` paths are
//! intentionally unimplemented in this slice (return
//! `CommitProtocolError::Internal { reason }`); they ship in PR 9b-4 and
//! PR 9b-5.
//!
//! **PR 9b-4+ scope** (NOT in this PR): the remaining `EffectPattern`
//! bodies, lifecycle wiring, 9-cell recovery cross-product, Test A/B,
//! and WORLD-MUTATING Mote E2E.
//!
//! # Why a separate error type from `SubmissionRefusal` and `MoteExecutorError`
//!
//! - `SubmissionRefusal` (`refusal.rs`) is **submission-time** — the Mote
//!   hasn't dispatched yet. R-1..R-10 fire here.
//! - `MoteExecutorError` (`executor_trait.rs`) is **body-runtime** — the
//!   body process was spawned and either failed to start, exited non-zero,
//!   or exceeded wall-clock. The body's effect on the world is the
//!   question.
//! - `CommitProtocolError` (this module) is **post-body, pre-commit** —
//!   the body succeeded but the executor refuses to journal `Committed`,
//!   either because the content store didn't durably accept the result
//!   bytes (R-11), or because a higher-level recovery decision refuses
//!   re-dispatch (R-13), or because the protocol detected an attempt to
//!   treat `Committed` as proof-of-validity (R-12 sentinel).
//!
//! Mixing the three would entangle the failure semantics; the closed
//! vocabularies + matched-arm exhaustiveness make the contract auditable
//! at the call sites.

use kx_capability::{CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore};
use kx_journal::{Journal, JournalEntry};
use kx_mote::{EffectPattern, Mote, MoteDefHash, MoteId, ToolName};
use kx_warrant::WarrantSpec;
use smallvec::SmallVec;
use thiserror::Error;

/// Commit-time + recovery-time refusal vocabulary.
///
/// All variants are reachable from the future `CommitProtocol::commit`
/// impl (9b-3+) and from the recovery-path consultation of
/// `kx_projection::can_redispatch_world_effect`. The vocabulary is closed
/// at PR 9b; future extensions land via new variants behind a closed-enum
/// match on every call site.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommitProtocolError {
    /// **R-11** (D39 §a/§c). The `result_ref` does not exist in the content
    /// store at the time the executor would call
    /// `journal.append(Committed)`, OR `ContentStore::get(result_ref)`
    /// returns incomplete bytes. The commit protocol MUST NOT short-circuit
    /// a re-`put` on the assumption that an existing ref implies a complete
    /// object.
    ///
    /// `put` is atomic per D39 §c — the content store's contract says a
    /// returned `ref` MUST point at the full bytes. R-11 fires when the
    /// executor proves the contract was violated (mock content store in
    /// tests, or a hostile real content store).
    #[error("R-11: result_ref {result_ref:?} for Mote {mote_id:?} is missing or incomplete in the content store; cannot append Committed (D39 §a/§c)")]
    R11ResultRefIncomplete {
        /// The Mote being committed.
        mote_id: MoteId,
        /// The reference that should have pointed at complete bytes.
        result_ref: ContentRef,
    },

    /// **R-12** (D39 §d). Sentinel variant — used by call sites that would
    /// otherwise treat `Committed` as proof-of-validity. Validity is
    /// established by the ABSENCE of a later `Repudiated` entry in the
    /// folded log. The commit protocol MUST NOT encode any assumption that
    /// `Committed` is terminal.
    ///
    /// This variant is constructed at the audit-trail boundary: any code
    /// path that would use `Committed` as a validity proof gets refactored
    /// to consult the projection's repudiation-tail check instead, and the
    /// previous broken path emits this refusal to make the regression
    /// visible.
    #[error("R-12: Mote {mote_id:?}: Committed is NOT proof-of-validity; consult Projection::repudiation_tail (D39 §d)")]
    R12CommittedNotProofOfValidity {
        /// The Mote whose Committed entry was about to be misused.
        mote_id: MoteId,
        /// Diagnostic context for the operator.
        context: String,
    },

    /// **R-13** (D38 §2b + STEP 5.2 + STEP 5.4). At recovery time, the
    /// executor consulted
    /// `Projection::can_redispatch_world_effect(mote_id)` and got `false`
    /// (terminal_failure_observed or inconsistent). Re-dispatch of the
    /// WORLD-MUTATING tool effect is REFUSED.
    ///
    /// The `reason` field carries the projection's diagnostic so the
    /// operator can distinguish the two refusal cases (terminal failure
    /// observed vs. inconsistent state).
    #[error("R-13: WORLD-MUTATING re-dispatch refused for Mote {mote_id:?}: {reason} (D38 §2b)")]
    R13WmReDispatchRefused {
        /// The Mote whose re-dispatch was refused.
        mote_id: MoteId,
        /// Projection-supplied diagnostic
        /// (`terminal_failure_observed` / `inconsistent`).
        reason: String,
    },

    /// The capability broker's `dispatch` call returned a typed error. The
    /// body has not run; no `Committed` entry is appended. The lifecycle
    /// layer surfaces this as a `Failed` journal entry.
    #[error("broker dispatch failed for Mote {mote_id:?}: {reason}")]
    BrokerDispatchFailed {
        /// The Mote whose broker call failed.
        mote_id: MoteId,
        /// Diagnostic from the broker.
        reason: String,
    },

    /// `ContentStore::put` returned an error. The broker may have already
    /// dispatched the effect (in which case the WM double-effect window is
    /// open until recovery consults `can_redispatch_world_effect`). The
    /// lifecycle layer MUST emit a `Failed` entry; recovery uses R-13 +
    /// the journal fold to decide re-dispatch safety.
    #[error("content store put failed for Mote {mote_id:?}: {reason}")]
    ContentStorePutFailed {
        /// The Mote whose put failed.
        mote_id: MoteId,
        /// Diagnostic from the content store.
        reason: String,
    },

    /// The journal's `append(Committed)` call returned an error after the
    /// `put` succeeded. This is a recovery scenario — the result_ref is
    /// durably stored but the Committed entry didn't land. Restart of the
    /// executor re-folds the journal; if the Committed entry was never
    /// written, the recovery decision falls to R-13's
    /// `can_redispatch_world_effect` consultation (re-dispatch permitted
    /// for idempotent or stage-then-commit paths; refused for terminal-
    /// failure-observed paths).
    #[error("journal append(Committed) failed for Mote {mote_id:?}: {reason}")]
    JournalAppendCommittedFailed {
        /// The Mote whose Committed append failed.
        mote_id: MoteId,
        /// Diagnostic from the journal.
        reason: String,
    },

    /// Anything else — fail-closed catch-all. Operator-facing diagnostic
    /// surfaces the root cause.
    #[error("commit protocol internal error for Mote {mote_id:?}: {reason}")]
    Internal {
        /// The Mote whose commit raised the internal error.
        mote_id: MoteId,
        /// Operator-facing diagnostic.
        reason: String,
    },
}

impl CommitProtocolError {
    /// Returns the `MoteId` carried by every variant. Useful for the
    /// lifecycle layer's `Failed` journal entry construction.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_content::ContentRef;
    /// use kx_executor::CommitProtocolError;
    /// use kx_mote::MoteId;
    ///
    /// let mote_id = MoteId::from_bytes([0x42; 32]);
    /// let err = CommitProtocolError::R11ResultRefIncomplete {
    ///     mote_id,
    ///     result_ref: ContentRef::from_bytes([0; 32]),
    /// };
    /// assert_eq!(err.mote_id(), mote_id);
    /// ```
    #[must_use]
    pub fn mote_id(&self) -> MoteId {
        match self {
            Self::R11ResultRefIncomplete { mote_id, .. }
            | Self::R12CommittedNotProofOfValidity { mote_id, .. }
            | Self::R13WmReDispatchRefused { mote_id, .. }
            | Self::BrokerDispatchFailed { mote_id, .. }
            | Self::ContentStorePutFailed { mote_id, .. }
            | Self::JournalAppendCommittedFailed { mote_id, .. }
            | Self::Internal { mote_id, .. } => *mote_id,
        }
    }

    /// Returns `true` iff this variant denotes a recovery-time refusal
    /// (R-13). Used by recovery paths to distinguish "do not re-dispatch
    /// this WM effect" from "dispatch failed; consult the recovery state."
    ///
    /// # Example
    ///
    /// ```
    /// use kx_executor::CommitProtocolError;
    /// use kx_mote::MoteId;
    ///
    /// let err = CommitProtocolError::R13WmReDispatchRefused {
    ///     mote_id: MoteId::from_bytes([0x42; 32]),
    ///     reason: "terminal_failure_observed".into(),
    /// };
    /// assert!(err.is_recovery_refusal());
    ///
    /// let other = CommitProtocolError::Internal {
    ///     mote_id: MoteId::from_bytes([0x42; 32]),
    ///     reason: "something else".into(),
    /// };
    /// assert!(!other.is_recovery_refusal());
    /// ```
    #[must_use]
    pub fn is_recovery_refusal(&self) -> bool {
        matches!(self, Self::R13WmReDispatchRefused { .. })
    }
}

/// Trait surface for the per-`EffectPattern` commit protocol.
///
/// **PR 9b-2 scope**: trait declaration + Send + Sync + object-safety
/// constraints only. The body is `unimplemented!()` in this slice; the
/// per-pattern bodies land in 9b-3+.
///
/// # Object safety
///
/// `CommitProtocol` is object-safe (no generics; no associated types). The
/// future commit-protocol consumer holds `Arc<dyn CommitProtocol>`, matching
/// the `BodyResolver` / `MoteExecutor` / `CapabilityBroker` indirection
/// pattern already in shipped code.
///
/// # The three patterns
///
/// - `EffectPattern::IdempotentByConstruction` →
///   `broker.dispatch → put → append(Committed)` (D39 §a).
/// - `EffectPattern::StageThenCommit` →
///   `journal.append(EffectStaged) → broker.dispatch → put →
///   append(Committed)` (D38 §2b).
/// - `EffectPattern::ValidateThenCommit` →
///   `broker.dispatch → put → append(Committed)` + critic child Mote
///   dispatched per D20.
///
/// All three end at `journal.append(Committed)`; the differences are the
/// pre-dispatch ordering + the critic-child Mote scheduling for
/// `ValidateThenCommit`. R-11/R-12/R-13 enforce the invariants across all
/// three paths.
pub trait CommitProtocol: Send + Sync {
    /// Run the commit protocol for one Mote per its `EffectPattern`.
    /// Returns the journal sequence number of the `Committed` entry on
    /// success; returns `CommitProtocolError` on any refusal.
    ///
    /// **PR 9b-2 scope**: this method is unimplemented in the current slice;
    /// the per-pattern bodies land in PR 9b-3 (commit_protocol per-pattern
    /// impl) and PR 9b-4 (lifecycle integration).
    ///
    /// # Errors
    ///
    /// Returns a `CommitProtocolError` variant for any refusal or failure
    /// in the commit path. See variant docs for the per-case semantics.
    fn commit(&self, input: CommitInput<'_>) -> Result<u64, CommitProtocolError>;
}

/// Input bundle for `CommitProtocol::commit`. Carries the Mote being
/// committed plus the body's `result_ref` (the BLAKE3 of the body output
/// that should be in the content store) plus the recovery context.
///
/// **Extended in PR 9b-3** to carry the dispatch context that the
/// `StandardCommitProtocol` needs to run `broker.dispatch` + build the
/// `Committed` journal entry. The trait surface remains the same — only
/// the carrier shape grows.
///
/// **Pre-alpha field-set discipline**: not `#[non_exhaustive]` — consumers
/// constructing `CommitInput` directly will need to update when fields
/// are added. Will be revisited when the surface stabilizes for SDK
/// callers (P4).
#[derive(Debug, Clone)]
pub struct CommitInput<'a> {
    /// The Mote being committed. Carries `effect_pattern`, `nd_class`,
    /// `tool_contract`, etc.
    pub mote: &'a Mote,
    /// The active warrant. The broker's per-call contract checks reference
    /// this; the resulting `Committed` entry's `warrant_ref` is derived
    /// from it.
    pub warrant: &'a WarrantSpec,
    /// Which tool in `mote.def.tool_contract` is being dispatched. Must be
    /// present in the Mote's `tool_contract`; the broker enforces this.
    pub capability: ToolName,
    /// The effect request payload (built from the Mote's body output).
    /// Carries the per-call scopes (subset of warrant scopes) + the
    /// idempotency token (D38 §1) when applicable.
    pub effect_request: EffectRequest,
    /// `warrant_ref` for the `Committed` entry's `warrant_ref` field.
    /// Pre-computed by the lifecycle layer via
    /// `kx_warrant::warrant_ref_of(&warrant)`.
    pub warrant_ref: ContentRef,
    /// `mote_def_hash` for the `Committed` entry's non-canonical metadata
    /// (D22 `list_committed_by_mote_def_hash`). Pre-computed by the
    /// lifecycle layer.
    pub mote_def_hash: MoteDefHash,
    /// Per-call idempotency key for the `Committed` entry. Derived per
    /// `idempotency.md`; the journal's dedup index uses this to enforce
    /// "at most one Committed per `idempotency_key`."
    pub idempotency_key: [u8; 32],
    /// Declared parents with edge metadata. Carried into the `Committed`
    /// entry's `parents` field.
    pub parents: SmallVec<[kx_journal::ParentEntry; 4]>,
    /// Operator-facing context string (carried into error variants when
    /// commit refuses). Lifecycle layer constructs from
    /// `(workflow_id, mote_id.to_hex())`.
    pub diagnostic_context: &'a str,
}

/// The standard `CommitProtocol` implementation — wires
/// `CapabilityBroker` + `ContentStore` + `Journal` together via the
/// per-`EffectPattern` paths defined in D38 + D39.
///
/// **PR 9b-3 scope**: only the `IdempotentByConstruction` path is
/// implemented (D39 §a — `broker.dispatch → R-11 verify →
/// journal.append(Committed)`). `StageThenCommit` returns
/// `CommitProtocolError::Internal { reason: "PR 9b-4 ..." }` and
/// `ValidateThenCommit` returns
/// `CommitProtocolError::Internal { reason: "PR 9b-5 ..." }`.
///
/// **Generic over** `(S, J, B)` — concrete impls of `ContentStore` /
/// `Journal` / `CapabilityBroker`. Held by `Arc<S>` / `Arc<J>` / `Arc<B>`
/// so the protocol can be cloned cheaply + shared across threads.
///
/// **Object safety**: the outer `CommitProtocol` trait is object-safe.
/// `StandardCommitProtocol<S, J, B>` carries the generics; callers hold
/// `Arc<dyn CommitProtocol>` to erase them.
#[derive(Debug)]
pub struct StandardCommitProtocol<S, J, B> {
    store: std::sync::Arc<S>,
    journal: std::sync::Arc<J>,
    broker: std::sync::Arc<B>,
}

impl<S, J, B> StandardCommitProtocol<S, J, B> {
    /// Construct a new `StandardCommitProtocol` carrying the three seams.
    /// The protocol holds `Arc`s so it can be cloned cheaply.
    #[must_use]
    pub fn new(
        store: std::sync::Arc<S>,
        journal: std::sync::Arc<J>,
        broker: std::sync::Arc<B>,
    ) -> Self {
        Self {
            store,
            journal,
            broker,
        }
    }
}

impl<S, J, B> CommitProtocol for StandardCommitProtocol<S, J, B>
where
    S: ContentStore + Send + Sync,
    J: Journal + Send + Sync,
    B: CapabilityBroker + Send + Sync,
{
    fn commit(&self, input: CommitInput<'_>) -> Result<u64, CommitProtocolError> {
        let pattern = input.mote.def.effect_pattern;
        match pattern {
            EffectPattern::IdempotentByConstruction => self.commit_idempotent(input),
            EffectPattern::StageThenCommit => Err(CommitProtocolError::Internal {
                mote_id: input.mote.id,
                reason: "StageThenCommit path lands in PR 9b-4 (journal.append(EffectStaged) → broker.dispatch → put → append(Committed))".into(),
            }),
            EffectPattern::ValidateThenCommit => Err(CommitProtocolError::Internal {
                mote_id: input.mote.id,
                reason: "ValidateThenCommit path lands in PR 9b-5 (broker.dispatch → put → append(Committed) + critic-Mote child scheduling per D20)".into(),
            }),
        }
    }
}

impl<S, J, B> StandardCommitProtocol<S, J, B>
where
    S: ContentStore + Send + Sync,
    J: Journal + Send + Sync,
    B: CapabilityBroker + Send + Sync,
{
    /// `IdempotentByConstruction` path (D39 §a):
    /// `broker.dispatch → R-11 verify → journal.append(Committed)`.
    ///
    /// `IdempotentByConstruction` is the safe path for token-class tools:
    /// the remote API's idempotency contract backstops the dispatch, so
    /// the executor can dispatch without a journal entry first. The
    /// post-dispatch R-11 verify is the executor's defense against a
    /// hostile broker that returns a `staged_ref` without actually
    /// staging the bytes.
    fn commit_idempotent(&self, input: CommitInput<'_>) -> Result<u64, CommitProtocolError> {
        let mote_id = input.mote.id;

        // Step 1: broker dispatch. The broker runs its per-call contract
        // checks (capability in tool_contract, supported pattern, capability
        // in warrant.tool_grants, request scopes ⊆ warrant scopes) and
        // routes to the named capability.
        let handle = self
            .broker
            .dispatch(
                input.mote,
                input.warrant,
                &input.capability,
                input.effect_request,
            )
            .map_err(|e| CommitProtocolError::BrokerDispatchFailed {
                mote_id,
                reason: format!("{e:?}"),
            })?;
        let result_ref = handle.staged_ref;

        // Step 2: R-11 verify (D39 §a/§c). The broker claims it staged the
        // response payload; the executor verifies the ref is durable in
        // the store before journaling `Committed`. Two failure modes:
        // (a) the ref isn't in the store (`contains` is false); (b) `get`
        // returns NotFound. Both indicate a hostile / buggy broker that
        // violated the staging contract.
        enforce_r11(&*self.store, mote_id, &result_ref)?;

        // Step 3: Journal append Committed. The journal assigns the
        // monotonic `seq` and dedup-by-`idempotency_key` enforces at-most-
        // one Committed per identity.
        let entry = JournalEntry::Committed {
            mote_id,
            idempotency_key: input.idempotency_key,
            seq: 0, // journal-assigned
            nondeterminism: input.mote.def.nd_class,
            result_ref,
            parents: input.parents,
            warrant_ref: input.warrant_ref,
            mote_def_hash: input.mote_def_hash,
        };
        let written = self.journal.append(entry).map_err(|e| {
            CommitProtocolError::JournalAppendCommittedFailed {
                mote_id,
                reason: format!("{e:?}"),
            }
        })?;

        Ok(written.seq())
    }
}

/// R-11 enforcement helper. Verifies a `result_ref` is durable in the
/// content store before the caller appends `Committed` to the journal.
///
/// Two failure cases both surface as `R11ResultRefIncomplete`:
/// - `store.contains(&result_ref)` is `false` (the ref isn't in the store)
/// - `store.get(&result_ref)` returns `NotFound` (the ref is registered
///   but the backing bytes are missing or have been reclaimed)
///
/// Exposed as `pub(crate)` so the other per-pattern paths (PR 9b-4 /
/// PR 9b-5) reuse the same check.
///
/// # Errors
///
/// Returns `CommitProtocolError::R11ResultRefIncomplete { mote_id,
/// result_ref }` when either check fails.
pub(crate) fn enforce_r11<S: ContentStore + ?Sized>(
    store: &S,
    mote_id: MoteId,
    result_ref: &ContentRef,
) -> Result<(), CommitProtocolError> {
    if !store.contains(result_ref) {
        return Err(CommitProtocolError::R11ResultRefIncomplete {
            mote_id,
            result_ref: *result_ref,
        });
    }
    if store.get(result_ref).is_err() {
        return Err(CommitProtocolError::R11ResultRefIncomplete {
            mote_id,
            result_ref: *result_ref,
        });
    }
    Ok(())
}
