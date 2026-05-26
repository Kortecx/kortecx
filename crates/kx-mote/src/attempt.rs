//! Per-attempt lifecycle state machine: [`AttemptState`] +
//! [`IllegalTransition`] + [`transition`] + [`is_legal_transition`] +
//! [`ALL_ATTEMPT_STATES`]. The in-memory model only — the journal carries
//! the durable per-attempt facts.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Lifecycle state machine (attempt-scoped, D3)
// ---------------------------------------------------------------------------

/// The per-attempt lifecycle state of a Mote (`mote.md` §7).
///
/// **Attempt-scoped, not Mote-scoped.** The Mote's *identity* (the [`crate::MoteId`])
/// may have many attempts in the journal — e.g., `Failed`, `Failed`,
/// `Committed`. The journal records all attempts; the projection collapses
/// them to a per-identity current state with the precedence rules in
/// `projection.md` §4. This enum describes ONE attempt.
///
/// Stable u8 representations are not assigned here — `AttemptState` is an
/// in-memory model only. The journal carries the *fact of an attempt's
/// outcome* (`Proposed` / `Committed` / `Failed` / `Repudiated` entries),
/// not the running state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AttemptState {
    /// The Mote exists in the DAG but the scheduler has not selected it.
    Pending,
    /// The scheduler has selected the Mote for placement; a `Proposed`
    /// journal entry has been written.
    Scheduled,
    /// A worker has accepted the Mote and begun execution. **Not durable** —
    /// "currently running" is an intent, tracked in worker memory only.
    Running,
    /// The atomic journal txn writing the `Committed` entry has landed.
    Committed,
    /// The attempt reached a terminal failure (typed error, retries
    /// exhausted, validator rejection). A `Failed` journal entry has been
    /// written. Future attempts under the same identity are independent.
    Failed,
    /// The committed result has been explicitly invalidated (operator action,
    /// critic verdict, upstream cascade per D22). A `Repudiated` journal
    /// entry referencing the original `Committed` has been written. Terminal
    /// for this attempt; the log is append-only.
    Repudiated,
}

/// An illegal-transition error from [`transition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("illegal Mote attempt transition: {from:?} → {to:?}")]
pub struct IllegalTransition {
    /// The state the attempt was in.
    pub from: AttemptState,
    /// The state the caller attempted to move to.
    pub to: AttemptState,
}

/// Validate a per-attempt lifecycle transition.
///
/// Returns `Ok(to)` if the transition is one of the five legal transitions
/// (`Pending → Scheduled`, `Scheduled → Running`, `Running → Committed`,
/// `Running → Failed`, `Committed → Repudiated`); otherwise returns
/// [`IllegalTransition`]. Every other from→to pair in the 6×6 state matrix
/// is illegal, including same-state self-loops, `Failed → *`, and
/// `Repudiated → *` (both terminal).
///
/// This function is the single source of truth for the transition rules.
/// `kx-executor` (P1.9) and `kx-coordinator` (P2.2) call it before writing
/// any journal entry that would advance an attempt's state.
///
/// # Examples
///
/// ```
/// use kx_mote::{transition, AttemptState};
///
/// // Legal: Pending → Scheduled
/// assert_eq!(
///     transition(AttemptState::Pending, AttemptState::Scheduled).unwrap(),
///     AttemptState::Scheduled
/// );
///
/// // Illegal: Pending → Running (must go through Scheduled first)
/// assert!(transition(AttemptState::Pending, AttemptState::Running).is_err());
///
/// // Illegal: Committed → Failed (Committed only transitions to Repudiated)
/// assert!(transition(AttemptState::Committed, AttemptState::Failed).is_err());
///
/// // Illegal: same-state self-loop
/// assert!(transition(AttemptState::Running, AttemptState::Running).is_err());
/// ```
pub fn transition(from: AttemptState, to: AttemptState) -> Result<AttemptState, IllegalTransition> {
    use AttemptState::{Committed, Failed, Pending, Repudiated, Running, Scheduled};
    let legal = matches!(
        (from, to),
        (Pending, Scheduled)
            | (Scheduled, Running)
            | (Running, Committed)
            | (Running, Failed)
            | (Committed, Repudiated)
    );
    if legal {
        Ok(to)
    } else {
        Err(IllegalTransition { from, to })
    }
}

/// All six [`AttemptState`] variants, for exhaustive iteration in tests and
/// debug tools. Stable order; changes to this constant signal a schema-level
/// adjustment.
pub const ALL_ATTEMPT_STATES: [AttemptState; 6] = [
    AttemptState::Pending,
    AttemptState::Scheduled,
    AttemptState::Running,
    AttemptState::Committed,
    AttemptState::Failed,
    AttemptState::Repudiated,
];

/// Returns `true` for the five legal per-attempt transitions; `false`
/// otherwise. Pure helper around [`transition`] for code paths that prefer a
/// boolean check (e.g., precondition assertions in tests and debug panes).
#[must_use]
pub fn is_legal_transition(from: AttemptState, to: AttemptState) -> bool {
    transition(from, to).is_ok()
}
