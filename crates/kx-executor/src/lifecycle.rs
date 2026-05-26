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

use std::sync::Arc;

use kx_content::ContentRef;
use kx_journal::{Journal, JournalEntry};
use kx_mote::{Mote, MoteId, NdClass};
use kx_warrant::WarrantSpec;
use smallvec::SmallVec;
use thiserror::Error;

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use crate::refusal::SubmissionRefusal;
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

    /// Catch-all internal error.
    #[error("lifecycle internal: {0}")]
    Internal(String),
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
