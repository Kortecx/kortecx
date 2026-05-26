//! [`crate::EffectPattern`] — workflow-author-declared per-Mote commit pattern
//! (IdempotentByConstruction / StageThenCommit / ValidateThenCommit) per D20.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// EffectPattern — which effect/commit pattern this Mote uses
// ---------------------------------------------------------------------------

/// Declares which of the three effect/commit patterns a Mote uses
/// (`mote.md` §4, D20).
///
/// Read by the executor's submission-time refusal predicate
/// (`validate-then-commit.md` §7) to enforce the safety contract: a
/// WORLD-MUTATING Mote without an idempotency mechanism AND without a critic
/// is refused at submission. The field is REQUIRED (not `Option`); workflow
/// authors must declare a pattern explicitly.
///
/// # Examples
///
/// ```
/// use kx_mote::EffectPattern;
///
/// // The three patterns are mutually exclusive; a Mote picks exactly one.
/// let payment = EffectPattern::IdempotentByConstruction; // Stripe-style
/// let llm_output = EffectPattern::StageThenCommit;       // payload IS the effect
/// let critical_write = EffectPattern::ValidateThenCommit;// gated by a critic
///
/// assert_ne!(payment, llm_output);
/// assert_ne!(llm_output, critical_write);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EffectPattern {
    /// The effect carries an idempotency mechanism the external system honors
    /// (Stripe-style idempotency-key header, content-derived URL, deterministic
    /// resource path, unique-constraint INSERT). Safe to retry.
    IdempotentByConstruction,

    /// The effect produces a payload; the executor stages the payload into the
    /// content store and atomically commits the `result_ref`. Crashes before
    /// the txn lands leave the staged payload orphaned (GC-able). Most natural
    /// for pure-output WORLD-MUTATING work where the effect IS the payload.
    StageThenCommit,

    /// The effect proposes (writes to staging or makes a "draft" call); a
    /// downstream critic Mote validates; only on a valid verdict does the
    /// runtime promote to a committed effect. Full mechanics in
    /// `validate-then-commit.md` (D20).
    ValidateThenCommit,
}
