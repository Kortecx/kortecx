//! Deterministic (Tier-A) eval gate — the REQUIRED, flake-proof CI test.
//!
//! Scores the embedded `golden-v1` corpus with NO model / gateway / clock and asserts:
//! (1) no regression vs the committed `baseline.json`, (2) exact pinned scorer values
//! (independent of the baseline file, so a corrupted baseline can't mask a scorer
//! regression), (3) the corpus digest is stable. A scorer-logic change flips an
//! assertion here — there is no source of non-determinism, so it cannot flake.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_eval::{
    compare_to_baseline, embedded_baseline, load_golden_v1, score_corpus, Baseline, ScoreValue,
};

/// A fixed environment label (Gate values are env-independent integer ratios; the label
/// only annotates the trend record, which this test does not compare).
const ENV_LABEL: &str = "ci";

fn committed_baseline() -> Baseline {
    embedded_baseline().expect("embedded baseline parses")
}

#[test]
fn no_regression_against_committed_baseline() {
    let corpus = load_golden_v1().expect("golden-v1 corpus loads");
    let report = score_corpus(&corpus, ENV_LABEL.into(), "test".into());
    let baseline = committed_baseline();
    assert_eq!(
        report.suite_digest, baseline.suite_digest,
        "corpus digest matches the committed baseline (no silent drift)"
    );
    let cmp = compare_to_baseline(&report, &baseline, 0).expect("no corpus drift");
    assert!(
        cmp.ok,
        "eval regressed vs committed baseline: {:?}",
        cmp.regressions
    );
}

#[test]
fn aggregate_gate_values_are_pinned() {
    let corpus = load_golden_v1().expect("corpus");
    let report = score_corpus(&corpus, ENV_LABEL.into(), "test".into());
    let gate = |id: &str| {
        report
            .gates
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.per_mille)
    };
    assert_eq!(gate("task_success"), Some(1000));
    assert_eq!(gate("tool_call_f1"), Some(1000));
    assert_eq!(gate("groundedness"), Some(1000));
    // The rejection-recovery task spends one extra turn ⇒ the aggregate is below perfect:
    // (1000×6 + 750) / 7 = 964 per-mille (integer floor).
    assert_eq!(gate("loop_efficiency"), Some(964));
    // All 13 model-output formats decode as intended (the committed "before").
    assert_eq!(gate("format_coverage"), Some(1000));
}

#[test]
fn rejection_recovery_loop_efficiency_is_750_per_task() {
    // Pin the per-task scorer behaviour directly (independent of the aggregate + the
    // baseline file) — a loop_efficiency regression flips this regardless of corpus size.
    let corpus = load_golden_v1().expect("corpus");
    let report = score_corpus(&corpus, ENV_LABEL.into(), "test".into());
    let task = report
        .per_task
        .iter()
        .find(|t| t.task_id == "tool_rejection_recovery")
        .expect("rejection-recovery task present");
    let le = task
        .scores
        .iter()
        .find(|s| s.metric_id == "loop_efficiency")
        .expect("loop_efficiency scored");
    assert!(
        matches!(le.value, ScoreValue::Gate { per_mille: 750 }),
        "rejection-recovery loop_efficiency should be 750, got {:?}",
        le.value
    );
}

#[test]
fn corpus_digest_is_stable() {
    let a = load_golden_v1().expect("a");
    let b = load_golden_v1().expect("b");
    assert_eq!(a.suite_digest, b.suite_digest);
    assert_eq!(a.suite_digest.len(), 64, "blake3 hex");
}
