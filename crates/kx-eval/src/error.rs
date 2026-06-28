//! The closed error vocabulary for loading + comparing eval artifacts.
//!
//! Scoring itself is **total** — a scorer never errors, it returns a score over
//! whatever transcript it is given. Errors arise only at the I/O boundary
//! (loading a corpus, deserializing a baseline) and at the gate decision (a corpus
//! drift the operator must resolve with a deliberate re-baseline).

use thiserror::Error;

/// Why an eval artifact could not be loaded or a comparison could not be made.
#[derive(Debug, Error)]
pub enum EvalError {
    /// A corpus / baseline file was not valid UTF-8 or not well-formed JSON.
    #[error("malformed eval artifact ({what}): {detail}")]
    Malformed {
        /// Which artifact failed (e.g. `"golden suite"`, `"baseline"`, `"format cases"`).
        what: &'static str,
        /// A short structural diagnostic (never the raw payload).
        detail: String,
    },

    /// The current run was scored against a baseline captured on a DIFFERENT corpus
    /// (the `suite_digest` differs). This is fail-closed: the operator must re-capture
    /// the baseline deliberately (a corpus change is a measurement-contract change),
    /// never silently compare across corpora.
    #[error(
        "corpus drift: baseline suite_digest {baseline} != current {current} — re-baseline deliberately"
    )]
    CorpusDrift {
        /// The baseline's recorded suite digest (hex).
        baseline: String,
        /// The current corpus's suite digest (hex).
        current: String,
    },
}
