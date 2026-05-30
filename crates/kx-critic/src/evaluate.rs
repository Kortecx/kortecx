//! [`evaluate`] — the single entry point dispatching a [`CheckSpec`] to its check.

use kx_critic_types::{CheckSpec, CriticVerdict};

use crate::checks;

/// Evaluate one deterministic check against a producer's committed output bytes.
///
/// **Pure, total, deterministic** (see the crate-level contract): returns a
/// [`CriticVerdict`] for every input, never panics, and yields byte-identical
/// verdicts for identical `(spec, input)` across runs / processes / machines.
#[must_use]
pub fn evaluate(spec: &CheckSpec, input: &[u8]) -> CriticVerdict {
    match spec {
        CheckSpec::Schema(s) => checks::schema::eval(s, input),
        CheckSpec::Dedup(s) => checks::dedup::eval(s, input),
        CheckSpec::StatBounds(s) => checks::stat_bounds::eval(s, input),
        CheckSpec::PiiLeak(s) => checks::pii::eval(s, input),
    }
}
