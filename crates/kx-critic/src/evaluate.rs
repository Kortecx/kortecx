//! [`evaluate`] — the single entry point dispatching a [`CheckSpec`] to its check.

use kx_critic_types::{CheckSpec, CriticReason, CriticVerdict};

use crate::checks;

/// Reason code for an [`CheckSpec::LlmJudge`] reaching the in-process evaluator:
/// the model-graded gate is dispatched by the live serve executor, NEVER
/// in-process, so this path is unreachable in the live loop and fails closed.
const JUDGE_UNPARSEABLE_CODE: u16 = 1;

/// Evaluate one deterministic check against a producer's committed output bytes.
///
/// **Pure, total, deterministic** (see the crate-level contract): returns a
/// [`CriticVerdict`] for every input, never panics, and yields byte-identical
/// verdicts for identical `(spec, input)` across runs / processes / machines.
///
/// [`CheckSpec::LlmJudge`] (T-AGENT2) is **not** a deterministic in-process
/// check: the live serve executor dispatches the served model for it (see
/// `kx_gateway`'s `run_judge`). Reaching the in-process evaluator with a judge
/// spec is a misroute — it **fails closed to `Invalid`** (withhold), never
/// `Valid`, so a misconfiguration can never silently promote unverified output.
#[must_use]
pub fn evaluate(spec: &CheckSpec, input: &[u8]) -> CriticVerdict {
    match spec {
        CheckSpec::Schema(s) => checks::schema::eval(s, input),
        CheckSpec::Dedup(s) => checks::dedup::eval(s, input),
        CheckSpec::StatBounds(s) => checks::stat_bounds::eval(s, input),
        CheckSpec::PiiLeak(s) => checks::pii::eval(s, input),
        CheckSpec::LlmJudge(_) => CriticVerdict::Invalid {
            reason: CriticReason::JudgeRejected {
                reason_code: JUDGE_UNPARSEABLE_CODE,
            },
        },
    }
}
