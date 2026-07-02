//! The deterministic (Tier-A) suite runner — scores the embedded golden corpus with no
//! model, no gateway, no clock.
//!
//! Each task's scripted transcript flows through every per-transcript scorer; the
//! cross-format corpus flows through the parse scorer; the results aggregate into an
//! [`EvalReport`]. The caller supplies the environment label + git sha for the trend
//! record (capture is fallible and lives at the binary boundary, keeping this fn pure
//! aside from the values handed in).

use crate::corpus::{load_golden_v1, GoldenCorpus};
use crate::error::EvalError;
use crate::report::{aggregate, EvalReport, TaskScore};
use crate::scorers::{score_format_coverage, score_transcript, ScoreInput};

/// Load + score the embedded `golden-v1` corpus deterministically (Tier A).
///
/// # Errors
/// [`EvalError::Malformed`] if the corpus fails to load.
pub fn score_golden_v1(env_label: String, git_sha: String) -> Result<EvalReport, EvalError> {
    let corpus = load_golden_v1()?;
    Ok(score_corpus(&corpus, env_label, git_sha))
}

/// Load + score ONE capability family of the embedded corpus (`--suite`).
/// Report-only iteration aid: the suite id is labeled `golden-v1:<family>` so a
/// family report can never be mistaken for (or compared against) the aggregate
/// baseline by accident.
///
/// # Errors
/// [`EvalError::Malformed`] if the corpus fails to load or the family is unknown.
pub fn score_golden_v1_family(
    family: &str,
    env_label: String,
    git_sha: String,
) -> Result<EvalReport, EvalError> {
    let mut corpus = load_golden_v1()?;
    let known: std::collections::BTreeSet<String> = corpus
        .suite
        .tasks
        .iter()
        .map(|t| t.family.clone())
        .collect();
    if !known.contains(family) {
        return Err(EvalError::Malformed {
            what: "suite family",
            detail: format!(
                "unknown family {family:?} (known: {})",
                known.into_iter().collect::<Vec<_>>().join(", ")
            ),
        });
    }
    corpus.suite.tasks.retain(|t| t.family == family);
    corpus.suite.id = format!("{}:{family}", corpus.suite.id);
    Ok(score_corpus(&corpus, env_label, git_sha))
}

/// Score an already-loaded corpus (Tier A — scripted transcripts).
#[must_use]
pub fn score_corpus(corpus: &GoldenCorpus, env_label: String, git_sha: String) -> EvalReport {
    let per_task: Vec<TaskScore> = corpus
        .suite
        .tasks
        .iter()
        .map(|task| TaskScore {
            task_id: task.id.clone(),
            scores: score_transcript(&ScoreInput {
                transcript: &task.scripted_transcript,
                expect: &task.expect,
            }),
        })
        .collect();
    let format_coverage = score_format_coverage(&corpus.format.grants, &corpus.format.cases);
    aggregate(
        corpus.suite.id.clone(),
        corpus.suite_digest.clone(),
        per_task,
        &format_coverage,
        &[],
        env_label,
        git_sha,
    )
}
