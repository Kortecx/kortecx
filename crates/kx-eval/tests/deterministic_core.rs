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
    // The sequence columns are pinned BELOW perfect on purpose: the corpus carries
    // `seq_wrong_order_null` (right set, wrong order — FSA 0, PSA 500 there), so a
    // sequence scorer that went order-blind would read 1000 here and FAIL this exact
    // pin. Six tasks expect tools: FSA (5×1000+0)/6, PSA (5×1000+500)/6.
    assert_eq!(gate("tool_seq_fsa"), Some(833));
    assert_eq!(gate("tool_seq_psa"), Some(916));
    // Grounded tasks include `context_recall_unused_evidence_null` (evidence retrieved,
    // answer ignored it): groundedness (1000+1000+0)/3, context_recall stays 1000 —
    // the aggregate-level witness that the pair separates retrieval from grounding.
    assert_eq!(gate("groundedness"), Some(666));
    assert_eq!(gate("context_recall"), Some(1000));
    // The rejection-recovery task spends one extra turn ⇒ the aggregate is below perfect.
    // 13 tasks: (1000×12 + 750) / 13 = 980 per-mille (integer floor).
    assert_eq!(gate("loop_efficiency"), Some(980));
    // RC4c-2c: the LLM rerank moves the most-relevant passage (placed last) to the top.
    assert_eq!(gate("rerank_quality"), Some(1000));
    // RC5a: the agent recalled the fact it learned earlier AND grounded its answer on it
    // (the fail-closed guard — a recall that returned nothing would score 0).
    assert_eq!(gate("memory_quality"), Some(1000));
    // RC5b: the agent DISTILLED its episodic memories into ONE recalled entry AND grounded
    // its answer on it (the fail-closed guard — nothing consolidated would score 0).
    assert_eq!(gate("consolidation_quality"), Some(1000));
    // The skill-bearing run stayed INSIDE its wish set (search/read/draft;
    // never send) and actually fired tools (the fail-closed guard — a wished run
    // that never touched a tool, or an out-of-wish call, would score 0).
    assert_eq!(gate("skill_quality"), Some(1000));
    // Every model-output format decodes as intended — the 13 RC1 "before" formats
    // PLUS the RC2 grammar-shaped multi-tool envelopes (mcp-calc/calc, mcp-kv/get):
    // the canonical envelope the grammar enforces is the parser's strongest path.
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

/// The two null fixtures, pinned per task (independent of aggregates and the baseline
/// file): each proves its scorer reads DIFFERENTLY from the metric it complements on
/// the exact input where the complement is blind.
#[test]
fn the_null_fixtures_separate_what_their_neighbour_metrics_cannot() {
    let corpus = load_golden_v1().expect("corpus");
    let report = score_corpus(&corpus, ENV_LABEL.into(), "test".into());
    let score_of = |task: &str, metric: &str| {
        report
            .per_task
            .iter()
            .find(|t| t.task_id == task)
            .unwrap_or_else(|| panic!("task {task} present"))
            .scores
            .iter()
            .find(|s| s.metric_id == metric)
            .unwrap_or_else(|| panic!("{metric} scored on {task}"))
            .gate_per_mille()
    };
    // Right set, wrong order: F1 (order-tolerant) is blind, the sequence column is not.
    assert_eq!(score_of("seq_wrong_order_null", "tool_call_f1"), Some(1000));
    assert_eq!(score_of("seq_wrong_order_null", "tool_seq_fsa"), Some(0));
    assert_eq!(score_of("seq_wrong_order_null", "tool_seq_psa"), Some(500));
    // Evidence retrieved but unused: context_recall credits the retriever,
    // groundedness debits the answer.
    assert_eq!(
        score_of("context_recall_unused_evidence_null", "context_recall"),
        Some(1000)
    );
    assert_eq!(
        score_of("context_recall_unused_evidence_null", "groundedness"),
        Some(0)
    );
}

#[test]
fn corpus_digest_is_stable() {
    let a = load_golden_v1().expect("a");
    let b = load_golden_v1().expect("b");
    assert_eq!(a.suite_digest, b.suite_digest);
    assert_eq!(a.suite_digest.len(), 64, "blake3 hex");
}
