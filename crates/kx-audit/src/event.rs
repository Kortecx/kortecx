//! [`AuditEvent`] — the time-free, float-free, off-truth-path lifecycle event.
//!
//! This is a pure typed value: it derives `Clone, Debug, PartialEq, Eq` (so the
//! [`crate::InMemoryAuditSink`] can be asserted on deterministically) and it
//! deliberately carries **no `Serialize`** — the on-disk JSONL shape lives in
//! `crate::wire`, which adds the wall-clock stamp + sequence number + optional
//! principal. Keeping time out of this type makes "timestamps never feed the
//! digest" a *structural* guarantee, not a discipline.

use kx_content::ContentRef;
use kx_mote::{MoteId, NdClass};

/// Which dispatch path the orchestrator took for a Mote (echoed from the run
/// loop's `Action` arm — no recomputation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchKind {
    /// A PURE, recomputable body.
    Pure,
    /// A native deterministic-critic check (PURE, but the body is the check, not
    /// an executor spawn).
    Critic,
    /// The first dispatch of a WORLD-MUTATING / READ-ONLY-NONDET Mote.
    WmFresh,
    /// A re-dispatch of an in-flight WM/ROND Mote after a crash (recovery path;
    /// the broker's idempotency key keeps the external effect exactly-once).
    WmRecovery,
}

impl DispatchKind {
    /// The stable lowercase tag used on the JSONL wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::Critic => "critic",
            Self::WmFresh => "wm_fresh",
            Self::WmRecovery => "wm_recovery",
        }
    }
}

/// One off-truth-path runtime lifecycle event.
///
/// Every field is an ALREADY-DERIVED join key (a `MoteId`/`ContentRef` hash, an
/// `NdClass`, an integer count, or the 32-byte product digest). The sink echoes
/// runtime state — it NEVER recomputes a `MoteId` (SN-8). There is **no float and
/// no timestamp** anywhere here: time is added by the sink at the wire layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditEvent {
    /// The drive loop is starting with `runnable` declared workflow Motes.
    RunStarted {
        /// Number of declared workflow Motes in the runnable set at start.
        runnable: u32,
    },
    /// A resume folded an existing journal before the loop began.
    Recovered {
        /// Number of already-committed Motes folded from the journal on resume.
        committed_through: u32,
        /// The journal sequence number the fold reached.
        folded_through: u64,
    },
    /// A committed topology shaper materialized its children into the runnable set.
    ChildrenDerived {
        /// The shaper Mote whose committed decision produced the children.
        shaper: MoteId,
        /// Number of children materialized.
        children: u32,
    },
    /// A Mote was dispatched for execution.
    MoteDispatched {
        /// The dispatched Mote.
        mote_id: MoteId,
        /// Its non-determinism class.
        nd_class: NdClass,
        /// Which dispatch path was taken.
        kind: DispatchKind,
    },
    /// A Mote reached the terminal `Committed` state (its action is durable).
    MoteCommitted {
        /// The committed Mote.
        mote_id: MoteId,
        /// The committed action's content-addressed result ref.
        result_ref: ContentRef,
        /// Its non-determinism class.
        nd_class: NdClass,
    },
    /// A Mote reached the terminal `Failed` state (no later `Committed`).
    MoteFailed {
        /// The failed Mote.
        mote_id: MoteId,
    },
    /// A committed Mote was later `Repudiated` (a repudiation targeting it landed).
    MoteRepudiated {
        /// The repudiated Mote.
        mote_id: MoteId,
    },
    /// A journal-consistency anomaly: an `EffectStaged`-then-`Repudiated` sequence
    /// with no intervening `Committed` (surfaced for operator review).
    MoteInconsistent {
        /// The Mote in the inconsistent state.
        mote_id: MoteId,
    },
    /// The drive loop finished.
    RunCompleted {
        /// Number of Motes in a `Committed` (non-repudiated) state.
        committed: u32,
        /// Total declared workflow Motes.
        total: u32,
        /// The deterministic product-identity digest of the committed-result set.
        digest: [u8; 32],
    },
}
