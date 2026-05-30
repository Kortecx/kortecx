// SPDX-License-Identifier: Apache-2.0
//! [`verify_pure_rerun`] — re-test a committed PURE fact by re-running it.
//!
//! The runtime's identity mechanism IS the verification mechanism: a PURE Mote
//! re-derives byte-identically, so re-running its body and comparing the produced
//! `result_ref` to the committed one confirms (or refutes) reproducibility with
//! almost no new machinery.
//!
//! # PURE-ONLY (load-bearing guard)
//!
//! Verification by re-run is legal **only** for [`NdClass::Pure`]. A
//! `ReadOnlyNondet` re-run draws a *different sample* (a mismatch would be expected,
//! not informative); a `WorldMutating` re-run is the forbidden *double-effect*.
//! [`verify_pure_rerun`] refuses both with [`VerifyError::NotPure`] **before**
//! touching the resource manager or executor.
//!
//! # No journal side effects
//!
//! Unlike [`crate::run_pure_mote`] — whose P0.4 gate would serve the cached result
//! instead of re-running once the Mote is `Committed` — this entrypoint appends
//! nothing to the journal. It runs the body via the executor and compares against
//! the previously-committed `result_ref` the caller read from the journal.

use kx_content::ContentRef;
use kx_mote::{Mote, NdClass};
use kx_warrant::WarrantSpec;

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use crate::resource_manager::{ResourceError, ResourceManager};

/// Outcome of a verify-by-rerun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The re-run produced a byte-identical `result_ref` — the committed fact
    /// reproduces.
    Confirmed {
        /// The (matching) re-derived ref.
        result_ref: ContentRef,
    },
    /// The re-run produced a DIFFERENT `result_ref` — the committed fact does not
    /// reproduce (a non-reproducible "PURE" body, a changed artifact, or a corrupt
    /// committed ref).
    Diverged {
        /// The committed ref the caller expected.
        expected: ContentRef,
        /// The ref the re-run actually produced.
        observed: ContentRef,
    },
}

impl VerifyOutcome {
    /// `true` iff the re-run reproduced the committed ref.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }
}

/// Errors that prevent a verify-by-rerun from producing a verdict.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// Verify-by-rerun is PURE-only. `ReadOnlyNondet` resamples; `WorldMutating`
    /// would double-fire the effect. Both are refused before any execution.
    #[error(
        "verify-by-rerun is PURE-only; refused {0:?} \
         (ReadOnlyNondet resamples; WorldMutating would double-fire)"
    )]
    NotPure(NdClass),

    /// Acquiring or releasing the resource slot failed.
    #[error("resource manager error during verify: {0:?}")]
    Resource(ResourceError),

    /// The executor failed to run the body.
    #[error("executor run failed during verify: {0:?}")]
    ExecutorRun(MoteExecutorError),
}

/// Re-run a committed PURE Mote and compare its produced `result_ref` to `expected`
/// (the committed ref the caller read from the journal).
///
/// Refuses non-PURE Motes with [`VerifyError::NotPure`]; journals nothing.
///
/// # Errors
///
/// - [`VerifyError::NotPure`] — `mote.nd_class() != NdClass::Pure`.
/// - [`VerifyError::Resource`] — slot acquire/release failed.
/// - [`VerifyError::ExecutorRun`] — the body failed to run.
pub fn verify_pure_rerun<R, E>(
    mote: &Mote,
    warrant: &WarrantSpec,
    expected: ContentRef,
    resource_manager: &R,
    executor: &E,
) -> Result<VerifyOutcome, VerifyError>
where
    R: ResourceManager + ?Sized,
    E: MoteExecutor + ?Sized,
{
    // PURE-only guard FIRST — before any resource acquisition or execution.
    if mote.nd_class() != NdClass::Pure {
        return Err(VerifyError::NotPure(mote.nd_class()));
    }

    let slot = resource_manager
        .acquire(&warrant.resource_ceiling)
        .map_err(VerifyError::Resource)?;

    let run_result = executor.run(mote, warrant, None::<Rootfs>);
    let release_result = resource_manager.release(slot);

    // A run failure is the most informative error; surface it first.
    let MoteExecutionResult { result_ref, .. } = run_result.map_err(VerifyError::ExecutorRun)?;
    release_result.map_err(VerifyError::Resource)?;

    Ok(if result_ref == expected {
        VerifyOutcome::Confirmed { result_ref }
    } else {
        VerifyOutcome::Diverged {
            expected,
            observed: result_ref,
        }
    })
}
