//! PURE-Mote lifecycle orchestration. **PR 9a scope**: end-to-end PURE Mote
//! execution given a `MoteExecutor` (production backends return
//! `BackendUnsupported`; the integration test path passes a
//! [`TestMoteExecutor`] that returns a deterministic ref).
//!
//! The six-step lifecycle from `02-crate-specs.md` §`kx-executor`:
//! 1. Resolve warrant from `Proposed.warrant_ref` (PR 9a: caller-provided
//!    since 9a doesn't ship the journal-fold path).
//! 2. `ResourceManager::acquire` under `warrant.resource_ceiling`.
//! 3. Submission-time refusal (R-1..R-9 + R-8b + ValidatorTypeError +
//!    AttemptedWiden).
//! 4. `MoteExecutor::run` (selects backend per `warrant.executor_class`).
//! 5. Commit `result_ref` in ONE journal txn (PR 9a: Proposed + Committed
//!    pair for PURE Motes; PR 9b adds the EffectStaged-then-Committed
//!    protocol for non-PURE).
//! 6. `ResourceManager::release`.
//!
//! `kx-memoizer` / `kx-context-assembler` / `kx-inference` / `kx-capability`
//! integration is reserved for PR 9b (the commit-protocol PR).

use std::collections::BTreeMap;
use std::sync::Arc;

use kx_capability::EffectRequest;
use kx_content::ContentRef;
use kx_journal::{Journal, JournalEntry};
use kx_mote::{Mote, MoteId, NdClass, ToolName};
use kx_warrant::WarrantSpec;
use smallvec::SmallVec;
use thiserror::Error;

use kx_refusal::SubmissionRefusal;

use crate::commit_protocol::{CommitInput, CommitProtocol, CommitProtocolError};
use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use crate::resource_manager::{ResourceError, ResourceManager};

/// Top-level lifecycle errors. The PURE-Mote happy path returns `Ok(commit)`;
/// every other shape maps to a typed variant.
#[derive(Debug, Error)]
pub enum LifecycleError {
    /// Submission-time refusal — R-1..R-9 + R-8b + ValidatorTypeError +
    /// AttemptedWiden. The caller MUST write a single `Failed::
    /// UnsafeWorldMutatingConstruction` entry to the journal; this module
    /// does not write the entry itself (separation of concerns: refusal
    /// production vs journal write).
    #[error("submission refusal: {0:?}")]
    Refused(SubmissionRefusal),

    /// `ResourceManager::acquire` failed.
    #[error("resource acquisition: {0}")]
    ResourceAcquire(#[from] ResourceError),

    /// `MoteExecutor::run` failed.
    #[error("executor run: {0}")]
    ExecutorRun(#[from] MoteExecutorError),

    /// Journal append failed.
    #[error("journal append: {0}")]
    JournalAppend(String),

    /// Commit-protocol returned a typed error (R-11 / R-13 / broker /
    /// content-store / journal failures during the commit step). New in
    /// PR 9b-6; surfaces the commit-protocol vocabulary up to the caller.
    #[error("commit protocol: {0:?}")]
    CommitProtocol(CommitProtocolError),

    /// Catch-all internal error.
    #[error("lifecycle internal: {0}")]
    Internal(String),
}

impl From<CommitProtocolError> for LifecycleError {
    fn from(err: CommitProtocolError) -> Self {
        Self::CommitProtocol(err)
    }
}

/// Successful PURE-Mote lifecycle result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleCommit {
    /// The Committed entry's assigned seq.
    pub committed_seq: u64,
    /// The result_ref the Mote produced.
    pub result_ref: ContentRef,
    /// The Mote's identity.
    pub mote_id: MoteId,
}

/// Successful WORLD-MUTATING Mote lifecycle result. Extends
/// `LifecycleCommit` with the optional `critic_proposed_seq` field that
/// PR 9b-6's `run_wm_mote` populates when the producer's
/// `EffectPattern::ValidateThenCommit` triggers critic-Mote child
/// scheduling.
///
/// **PR 9b-6 scope**: this slice writes the critic's `Proposed` entry
/// to the journal (the scheduling intent is durably recorded). The
/// scheduler (PR 10) dispatches the critic to a worker; the recovery
/// fold reads the Proposed entry to know the critic was queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WmLifecycleCommit {
    /// The producer Mote's `Committed` entry seq.
    pub committed_seq: u64,
    /// The producer's `result_ref` (the broker's staged response).
    pub result_ref: ContentRef,
    /// The producer Mote's identity.
    pub mote_id: MoteId,
    /// For `ValidateThenCommit` producers with a sibling critic Mote in
    /// the submission, the critic's `Proposed` entry seq. `None` for
    /// `IdempotentByConstruction` / `StageThenCommit` paths (no critic).
    pub critic_proposed_seq: Option<u64>,
}

/// The **P0.4 hard gate** (P3.3): if `mote_id` is already `Committed` in the journal,
/// return its committed `(seq, result_ref)` so the caller serves the committed fact and
/// **never re-runs the Mote's logic**. For a non-deterministic Mote (ReadOnlyNondet /
/// WorldMutating) this is a *correctness* invariant — re-running would re-sample a
/// different observation or fire a second world effect, breaking exactly-once; for PURE
/// it is a harmless-but-wasteful re-run avoided. The committed entry is the source of
/// truth (`vision §`: "a non-deterministic step is never recomputed once committed;
/// recovery reads what it did"). Repudiation supersession of a committed result is the
/// cascade's concern (P3.5); the single-node engine already excludes committed +
/// repudiated Motes from dispatch, so this gate is the executor-level defense-in-depth
/// that makes the guarantee hold for *any* caller (engine, worker, future SDK).
fn serve_if_committed<J: Journal + ?Sized>(
    journal: &J,
    mote_id: &MoteId,
) -> Result<Option<(u64, ContentRef)>, LifecycleError> {
    match journal
        .read_committed(mote_id)
        .map_err(|e| LifecycleError::JournalAppend(format!("read Committed (P0.4 gate): {e:?}")))?
    {
        Some(JournalEntry::Committed {
            seq, result_ref, ..
        }) => Ok(Some((seq, result_ref))),
        _ => Ok(None),
    }
}

/// Run a single PURE Mote end-to-end. PR 9a's contract:
/// - The Mote MUST have `nd_class = Pure`. Non-PURE Motes return
///   `LifecycleError::Internal` because the broker / commit-protocol path
///   isn't wired in PR 9a (that's PR 9b).
/// - The caller provides the `warrant` (already-resolved). PR 9b will
///   replace this with a journal-fold step that re-derives the warrant from
///   `Proposed.warrant_ref`.
/// - The `MoteExecutor` argument is normally `default_executor()` (which on
///   PR 9a returns `BackendUnsupported`); integration tests pass a
///   [`TestMoteExecutor`] that returns a deterministic ref.
///
/// # Errors
///
/// See [`LifecycleError`] variants. The caller must write a `Failed` journal
/// entry when `LifecycleError::Refused` is returned (this module does not).
///
/// **P0.4 hard gate (P3.3):** before running, if `mote` is already `Committed` in the
/// journal, recovery READS the committed `result_ref` and the Mote's logic is **never
/// re-run**. For PURE this is a free optimization; the gate is shared with
/// [`run_wm_mote`], where it is a correctness invariant.
pub fn run_pure_mote<J, R, E>(
    mote: &Mote,
    warrant: &WarrantSpec,
    journal: &J,
    resource_manager: &R,
    executor: &E,
) -> Result<LifecycleCommit, LifecycleError>
where
    J: Journal + ?Sized,
    R: ResourceManager + ?Sized,
    E: MoteExecutor + ?Sized,
{
    if mote.nd_class() != NdClass::Pure {
        return Err(LifecycleError::Internal(format!(
            "PR 9a lifecycle handles PURE Motes only; got {:?}",
            mote.nd_class()
        )));
    }

    // P0.4 hard gate (P3.3): already committed → serve the committed result, never re-run.
    if let Some((committed_seq, result_ref)) = serve_if_committed(journal, &mote.id)? {
        tracing::debug!(mote = ?mote.id, committed_seq, "P0.4 gate: already committed — serving result, not re-running");
        return Ok(LifecycleCommit {
            committed_seq,
            result_ref,
            mote_id: mote.id,
        });
    }

    // Step 2: acquire resource slot.
    let slot = resource_manager.acquire(&warrant.resource_ceiling)?;

    // Step 4: run the body via the platform backend. PR 9a: the production
    // backends return `BackendUnsupported`; integration tests pass a
    // TestMoteExecutor.
    let run_result = executor.run(mote, warrant, None::<Rootfs>);
    let MoteExecutionResult {
        result_ref,
        started_at_epoch_ms: _,
        finished_at_epoch_ms: _,
    } = match run_result {
        Ok(r) => r,
        Err(e) => {
            // Release the slot before propagating.
            let _ = resource_manager.release(slot);
            return Err(LifecycleError::ExecutorRun(e));
        }
    };

    // Step 5: commit. PR 9a writes the Proposed + Committed pair atomically
    // (journal trait guarantees atomicity per-append). For PURE Motes there
    // is no broker dispatch; the lifecycle commits the body's result_ref
    // directly.
    let warrant_ref = kx_warrant::warrant_ref_of(warrant);
    let proposed = JournalEntry::Proposed {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq: 0, // journal assigns
        nondeterminism: kx_mote::NdClass::Pure,
        placement_hint: 0,
        warrant_ref,
    };
    let _proposed_entry = journal
        .append(proposed)
        .map_err(|e| LifecycleError::JournalAppend(format!("Proposed: {e:?}")))?;

    let committed = JournalEntry::Committed {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq: 0, // journal assigns
        nondeterminism: kx_mote::NdClass::Pure,
        result_ref,
        parents: SmallVec::new(),
        warrant_ref,
        mote_def_hash: mote.def.hash(),
    };
    let committed_entry = journal
        .append(committed)
        .map_err(|e| LifecycleError::JournalAppend(format!("Committed: {e:?}")))?;
    let committed_seq = committed_entry.seq();

    // Step 6: release the slot.
    resource_manager.release(slot)?;

    Ok(LifecycleCommit {
        committed_seq,
        result_ref,
        mote_id: mote.id,
    })
}

/// Recovery-time oracle consulted by [`redispatch_wm_mote`] before any
/// re-dispatch of a WORLD-MUTATING tool effect. Returns `true` iff the
/// re-dispatch is safe (D38 §2b + STEP 5.2 + STEP 5.4 + R-13).
///
/// **Returns `false` for**: `inconsistent` (cell 8 anomaly),
/// `terminal_failure_observed` (cell 5 — terminal failure under
/// EffectStaged), `committed.is_some()` (cells 4 + 6 — done; never
/// re-dispatch), and Motes with no observed EffectStaged (nothing to
/// re-dispatch).
///
/// `kx_projection::Projection` implements this trait directly; tests
/// stub it.
pub trait WmRedispatchOracle: Send + Sync {
    /// Returns `true` iff re-dispatch of `mote_id`'s WM effect is safe.
    fn can_redispatch_world_effect(&self, mote_id: &MoteId) -> bool;
}

impl WmRedispatchOracle for kx_projection::Projection {
    fn can_redispatch_world_effect(&self, mote_id: &MoteId) -> bool {
        kx_projection::Projection::can_redispatch_world_effect(self, mote_id)
    }
}

/// Recovery-time WORLD-MUTATING Mote re-dispatch path. **PR 9b-7
/// scope**: consults a [`WmRedispatchOracle`] BEFORE invoking
/// `commit_protocol.commit()` and refuses re-dispatch with
/// `LifecycleError::CommitProtocol(CommitProtocolError::R13WmReDispatchRefused
/// { ... })` when the oracle returns `false`.
///
/// Two semantic differences from [`run_wm_mote`]:
/// 1. The oracle's veto fires R-13 BEFORE the broker is consulted.
///    The lifecycle does not append a fresh `Proposed` entry on
///    refusal (the journal already carries the previous attempt's
///    Proposed; this is a recovery scenario, not fresh dispatch).
/// 2. On Ok-path (oracle approves), the journal already has the
///    previous attempt's `Proposed` + `EffectStaged` entries. This
///    function does NOT re-append Proposed; the broker's
///    tool-boundary idempotency (D38 §1 token-class) closes the
///    double-dispatch window — the remote API dedupes on
///    `idempotency_key`. `commit_protocol.commit()` proceeds to call
///    `broker.dispatch`, verify R-11, and append `Committed`.
///
/// # Errors
///
/// - `LifecycleError::CommitProtocol(R13WmReDispatchRefused)` when the
///   oracle returns `false`.
/// - `LifecycleError::ResourceAcquire(_)` if `acquire` fails.
/// - `LifecycleError::CommitProtocol(_)` for downstream commit-protocol
///   failures.
/// - `LifecycleError::JournalAppend(_)` for critic-Proposed write failures.
/// - `LifecycleError::Internal(_)` for PURE Motes (caller bug) or
///   missing Committed-readback after the protocol returned Ok.
#[allow(clippy::too_many_arguments)]
pub fn redispatch_wm_mote<J, R, CP, O>(
    mote: &Mote,
    warrant: &WarrantSpec,
    capability: ToolName,
    effect_request: EffectRequest,
    submission_motes: &BTreeMap<MoteId, Mote>,
    journal: &J,
    resource_manager: &R,
    commit_protocol: &CP,
    oracle: &O,
) -> Result<WmLifecycleCommit, LifecycleError>
where
    J: Journal + ?Sized,
    R: ResourceManager + ?Sized,
    CP: CommitProtocol + ?Sized,
    O: WmRedispatchOracle + ?Sized,
{
    if mote.nd_class() == NdClass::Pure {
        return Err(LifecycleError::Internal(format!(
            "redispatch_wm_mote handles WM/ReadOnlyNondet only; got Pure mote {:?}",
            mote.id
        )));
    }

    // Step 0 (NEW in 9b-7): R-13 recovery consultation. If the oracle
    // refuses, propagate without touching journal/broker/resource.
    if !oracle.can_redispatch_world_effect(&mote.id) {
        return Err(LifecycleError::CommitProtocol(
            CommitProtocolError::R13WmReDispatchRefused {
                mote_id: mote.id,
                reason: "WmRedispatchOracle returned false (terminal_failure_observed / inconsistent / already committed / no effect_staged_observed)".into(),
            },
        ));
    }

    // Step 2: acquire resource slot.
    let slot = resource_manager.acquire(&warrant.resource_ceiling)?;

    let warrant_ref = kx_warrant::warrant_ref_of(warrant);
    let mote_def_hash = mote.def.hash();
    let idempotency_key = *mote.id.as_bytes();

    // Note: NO fresh Proposed entry on the recovery path. The previous
    // attempt's Proposed (+ optionally EffectStaged) is already in the
    // journal — appending another would double-record the dispatch
    // intent. The broker's idempotency_key dedup is the load-bearing
    // safety here.

    // Step 4: commit_protocol routes per effect_pattern.
    let commit_input = CommitInput {
        mote,
        warrant,
        capability,
        effect_request,
        warrant_ref,
        mote_def_hash,
        idempotency_key,
        parents: SmallVec::new(),
        diagnostic_context: "lifecycle::redispatch_wm_mote",
    };
    let committed_seq = match commit_protocol.commit(commit_input) {
        Ok(seq) => seq,
        Err(e) => {
            let _ = resource_manager.release(slot);
            return Err(LifecycleError::CommitProtocol(e));
        }
    };

    // Step 5: critic-Mote child scheduling (same as run_wm_mote).
    let critic_proposed_seq =
        if mote.def.effect_pattern == kx_mote::EffectPattern::ValidateThenCommit {
            let critic = submission_motes
                .values()
                .find(|sibling| sibling.def.critic_for == Some(mote.id));
            match critic {
                Some(critic_mote) => {
                    let critic_proposed = JournalEntry::Proposed {
                        mote_id: critic_mote.id,
                        idempotency_key: *critic_mote.id.as_bytes(),
                        seq: 0,
                        nondeterminism: critic_mote.def.nd_class,
                        placement_hint: 0,
                        warrant_ref,
                    };
                    match journal.append(critic_proposed) {
                        Ok(entry) => Some(entry.seq()),
                        Err(e) => {
                            let _ = resource_manager.release(slot);
                            return Err(LifecycleError::JournalAppend(format!(
                                "critic Proposed (recovery): {e:?}"
                            )));
                        }
                    }
                }
                None => None,
            }
        } else {
            None
        };

    let result_ref = journal
        .read_committed(&mote.id)
        .map_err(|e| LifecycleError::JournalAppend(format!("read Committed: {e:?}")))?
        .and_then(|e| match e {
            JournalEntry::Committed { result_ref, .. } => Some(result_ref),
            _ => None,
        })
        .ok_or_else(|| {
            LifecycleError::Internal(format!(
                "commit_protocol returned Ok({committed_seq}) but no Committed entry visible"
            ))
        })?;

    resource_manager.release(slot)?;

    Ok(WmLifecycleCommit {
        committed_seq,
        result_ref,
        mote_id: mote.id,
        critic_proposed_seq,
    })
}

/// Run a single WORLD-MUTATING Mote end-to-end via the commit_protocol.
/// **PR 9b-6 scope**: ships the lifecycle's invocation of
/// `CommitProtocol::commit` for non-PURE Motes + critic-Mote child
/// scheduling for `ValidateThenCommit` producers.
///
/// Steps:
/// 1. Refuse if `mote.nd_class() == NdClass::Pure` (caller uses
///    `run_pure_mote` for PURE Motes).
/// 2. `ResourceManager::acquire` under `warrant.resource_ceiling`.
/// 3. Append `Proposed` for the producer.
/// 4. Invoke `commit_protocol.commit(CommitInput { ... })`. The protocol
///    routes per `mote.def.effect_pattern`:
///    - `IdempotentByConstruction` → `broker.dispatch → R-11 → Committed`.
///    - `StageThenCommit` → `EffectStaged → broker.dispatch → R-11 →
///      Committed`.
///    - `ValidateThenCommit` → `broker.dispatch → R-11 → Committed`.
/// 5. For `ValidateThenCommit` producers: if `submission_motes` contains
///    a sibling Mote with `critic_for == Some(producer.id)`, append a
///    `Proposed` entry for the critic so the scheduler (PR 10) can pick
///    it up. The critic's `nd_class` is recorded in the Proposed entry's
///    `nondeterminism` field per `journal-entry.md`.
/// 6. `ResourceManager::release`.
///
/// **The producer's `result_ref` carried in the returned
/// `WmLifecycleCommit` is the broker's staged response ref** — what the
/// commit_protocol got back from `broker.dispatch`. The producer's body
/// is not executed via `MoteExecutor::run` in PR 9b-6 (the broker is the
/// dispatch primitive for WM Motes); body-via-executor wiring is the
/// shape-of-future-PRs question for richer Mote bodies that prepare the
/// EffectRequest payload from sandboxed compute. For PR 9b-6's scope,
/// the caller supplies the `EffectRequest` directly.
///
/// # Errors
///
/// See [`LifecycleError`] variants. Commit-protocol failures (R-11 / R-13
/// / broker / journal) surface as `LifecycleError::CommitProtocol`. The
/// caller must write a `Failed` journal entry when
/// `LifecycleError::Refused` is returned (this module does not).
#[allow(clippy::too_many_arguments)] // PR 9b-6 explicit-args design; SDK ergonomics land at P4
pub fn run_wm_mote<J, R, CP>(
    mote: &Mote,
    warrant: &WarrantSpec,
    capability: ToolName,
    effect_request: EffectRequest,
    submission_motes: &BTreeMap<MoteId, Mote>,
    journal: &J,
    resource_manager: &R,
    commit_protocol: &CP,
) -> Result<WmLifecycleCommit, LifecycleError>
where
    J: Journal + ?Sized,
    R: ResourceManager + ?Sized,
    CP: CommitProtocol + ?Sized,
{
    if mote.nd_class() == NdClass::Pure {
        return Err(LifecycleError::Internal(format!(
            "run_wm_mote handles WM/ReadOnlyNondet only; got Pure mote {:?}; caller must use run_pure_mote",
            mote.id
        )));
    }

    // P0.4 hard gate (P3.3): a committed non-deterministic Mote is NEVER re-run — serve
    // the committed result. Re-running would re-sample a different ReadOnlyNondet
    // observation, or fire a SECOND world effect for a WorldMutating Mote whose effect is
    // already done + recorded (exactly-once). No Proposed is appended, the broker /
    // commit-protocol is not invoked. (The EffectStaged-but-not-committed recovery path is
    // `redispatch_wm_mote`, gated by the oracle; here the Mote is already Committed.)
    if let Some((committed_seq, result_ref)) = serve_if_committed(journal, &mote.id)? {
        tracing::debug!(mote = ?mote.id, committed_seq, "P0.4 gate: committed nondet Mote — serving result, not re-dispatching");
        return Ok(WmLifecycleCommit {
            committed_seq,
            result_ref,
            mote_id: mote.id,
            critic_proposed_seq: None,
        });
    }

    // Step 2: acquire resource slot.
    let slot = resource_manager.acquire(&warrant.resource_ceiling)?;

    let warrant_ref = kx_warrant::warrant_ref_of(warrant);
    let mote_def_hash = mote.def.hash();
    let idempotency_key = *mote.id.as_bytes();

    // Step 3: Proposed entry for the producer. Carries the warrant_ref +
    // nd_class per D36.
    let proposed = JournalEntry::Proposed {
        mote_id: mote.id,
        idempotency_key,
        seq: 0, // journal-assigned
        nondeterminism: mote.def.nd_class,
        placement_hint: 0,
        warrant_ref,
    };
    if let Err(e) = journal.append(proposed) {
        let _ = resource_manager.release(slot);
        return Err(LifecycleError::JournalAppend(format!(
            "WM producer Proposed: {e:?}"
        )));
    }

    // Step 4: commit_protocol routes per effect_pattern.
    let commit_input = CommitInput {
        mote,
        warrant,
        capability,
        effect_request,
        warrant_ref,
        mote_def_hash,
        idempotency_key,
        parents: SmallVec::new(),
        diagnostic_context: "lifecycle::run_wm_mote",
    };
    let committed_seq = match commit_protocol.commit(commit_input) {
        Ok(seq) => seq,
        Err(e) => {
            let _ = resource_manager.release(slot);
            return Err(LifecycleError::CommitProtocol(e));
        }
    };

    // Step 5: critic-Mote child scheduling for ValidateThenCommit.
    // The submission's sibling map is the source of truth — R-2 ensured
    // a critic exists at submission time (or refused the submission).
    let critic_proposed_seq =
        if mote.def.effect_pattern == kx_mote::EffectPattern::ValidateThenCommit {
            let critic = submission_motes
                .values()
                .find(|sibling| sibling.def.critic_for == Some(mote.id));
            match critic {
                Some(critic_mote) => {
                    let critic_proposed = JournalEntry::Proposed {
                        mote_id: critic_mote.id,
                        idempotency_key: *critic_mote.id.as_bytes(),
                        seq: 0, // journal-assigned
                        nondeterminism: critic_mote.def.nd_class,
                        placement_hint: 0,
                        warrant_ref,
                    };
                    match journal.append(critic_proposed) {
                        Ok(entry) => Some(entry.seq()),
                        Err(e) => {
                            let _ = resource_manager.release(slot);
                            return Err(LifecycleError::JournalAppend(format!(
                                "critic Proposed: {e:?}"
                            )));
                        }
                    }
                }
                None => None,
            }
        } else {
            None
        };

    // The producer's result_ref is the broker's staged response. Recover
    // it by reading the Committed entry the protocol just wrote. (We
    // can't read it back cheaply without journal lookups; for the
    // PR 9b-6 scope we record committed_seq + signal completion. Callers
    // who need result_ref read it from the journal.)
    let result_ref = journal
        .read_committed(&mote.id)
        .map_err(|e| LifecycleError::JournalAppend(format!("read Committed: {e:?}")))?
        .and_then(|e| match e {
            JournalEntry::Committed { result_ref, .. } => Some(result_ref),
            _ => None,
        })
        .ok_or_else(|| {
            LifecycleError::Internal(format!(
                "commit_protocol returned Ok({committed_seq}) but no Committed entry visible"
            ))
        })?;

    // Step 6: release the slot.
    resource_manager.release(slot)?;

    Ok(WmLifecycleCommit {
        committed_seq,
        result_ref,
        mote_id: mote.id,
        critic_proposed_seq,
    })
}

/// Test-only `MoteExecutor` impl that returns a deterministic `result_ref`
/// without spawning a subprocess. Exercises the PR 9a lifecycle's seams
/// without depending on bwrap/sandbox-exec being installed.
///
/// The `compute` closure produces the body's `result_ref` from the Mote +
/// warrant. PURE-Mote integration tests use this with a simple closure that
/// hashes the Mote's `mote_def_hash` to produce a stable ref.
pub struct TestMoteExecutor {
    compute: Arc<dyn Fn(&Mote, &WarrantSpec) -> ContentRef + Send + Sync>,
}

impl std::fmt::Debug for TestMoteExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestMoteExecutor").finish()
    }
}

impl TestMoteExecutor {
    /// Construct with a `compute` closure that produces the body's
    /// `result_ref` for a given `(Mote, WarrantSpec)`.
    pub fn new<F>(compute: F) -> Self
    where
        F: Fn(&Mote, &WarrantSpec) -> ContentRef + Send + Sync + 'static,
    {
        Self {
            compute: Arc::new(compute),
        }
    }

    /// Construct with the default test-compute: `result_ref` is BLAKE3 of
    /// the Mote's `mote_def_hash` bytes. Deterministic; convenient for
    /// integration tests that don't care about the exact ref value.
    #[must_use]
    pub fn deterministic() -> Self {
        Self::new(|mote, _warrant| {
            let mote_def_hash = mote.def.hash();
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"test-mote-executor-result");
            hasher.update(mote_def_hash.as_bytes());
            ContentRef::from_bytes(*hasher.finalize().as_bytes())
        })
    }
}

impl MoteExecutor for TestMoteExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        let result_ref = (self.compute)(mote, warrant);
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }

    fn supports(&self, _executor_class: kx_warrant::ExecutorClass) -> bool {
        // Test backend supports every class — the integration test fixture
        // explicitly opts in to the test backend regardless of warrant.executor_class.
        true
    }
}
