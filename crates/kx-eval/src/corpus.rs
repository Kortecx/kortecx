//! Loading + content-addressing the versioned golden corpus.
//!
//! The `golden-v1` corpus is **embedded at compile time** (`include_str!`) so the gate
//! resolves it from any working directory and the binary carries the exact bytes it
//! scores. Its `suite_digest` is a blake3 over those bytes — a corpus change shifts the
//! digest, and [`crate::compare_to_baseline`] fails closed until the baseline is
//! deliberately re-captured (the measurement contract changed).

use serde::Deserialize;

use crate::error::EvalError;
use crate::report::Baseline;
use crate::scorers::FormatCase;
use crate::suite::{ExpectedToolCall, GoldenSuite};

/// The id of the v1 golden suite.
pub const GOLDEN_V1_ID: &str = "golden-v1";

const SUITE_JSON: &str = include_str!("../corpus/golden-v1/suite.json");
const FORMAT_JSON: &str = include_str!("../corpus/golden-v1/format_cases.json");
const BASELINE_JSON: &str = include_str!("../corpus/golden-v1/baseline.json");

/// The id of the v1 real-model benchmark suite.
pub const BENCH_V1_ID: &str = "bench-v1";

const BENCH_V1_SUITE_JSON: &str = include_str!("../corpus/bench-v1/suite.json");

/// The cross-format parse corpus: the grant context the cases run under + the per-format
/// raw model strings and their intended decodes.
#[derive(Debug, Clone, Deserialize)]
pub struct FormatCorpus {
    /// The tools granted while parsing the cases (the cases name these).
    pub grants: Vec<ExpectedToolCall>,
    /// The per-format cases.
    pub cases: Vec<FormatCase>,
}

/// The loaded, content-addressed golden corpus.
#[derive(Debug, Clone)]
pub struct GoldenCorpus {
    /// The golden task suite (each task carries a scripted Tier-A transcript).
    pub suite: GoldenSuite,
    /// The cross-format parse corpus.
    pub format: FormatCorpus,
    /// The content digest (hex blake3 over the embedded corpus bytes).
    pub suite_digest: String,
}

/// Load + parse the embedded `golden-v1` corpus and compute its content digest.
///
/// # Errors
/// [`EvalError::Malformed`] if either embedded corpus file is not well-formed JSON.
pub fn load_golden_v1() -> Result<GoldenCorpus, EvalError> {
    let suite: GoldenSuite =
        serde_json::from_str(SUITE_JSON).map_err(|e| EvalError::Malformed {
            what: "golden suite",
            detail: e.to_string(),
        })?;
    let format: FormatCorpus =
        serde_json::from_str(FORMAT_JSON).map_err(|e| EvalError::Malformed {
            what: "format cases",
            detail: e.to_string(),
        })?;
    Ok(GoldenCorpus {
        suite,
        format,
        suite_digest: digest_hex(&[SUITE_JSON.as_bytes(), FORMAT_JSON.as_bytes()]),
    })
}

/// The loaded, content-addressed `bench-v1` real-model benchmark suite.
///
/// Unlike [`GoldenCorpus`], a bench suite carries no format-parse corpus: it is scored
/// only from LIVE served-model transcripts (each task is real-only — `instruction` +
/// `expect`, no scripted Tier-A fixture), so there is no deterministic format matrix to
/// fold. The `suite_digest` is a blake3 over the embedded suite bytes, so a task change
/// shifts the digest and [`crate::compare_to_baseline`] fails closed until the per-engine
/// baseline is deliberately re-captured — the same drift discipline as `golden-v1`.
#[derive(Debug, Clone)]
pub struct BenchCorpus {
    /// The real-model task suite (each task carries an `instruction` + `expect`, no
    /// scripted transcript).
    pub suite: GoldenSuite,
    /// The content digest (hex blake3 over the embedded suite bytes).
    pub suite_digest: String,
}

/// Load + parse the embedded `bench-v1` real-model benchmark suite and compute its content
/// digest. The suite is driven live by the gateway benchmark harness (the only layer with
/// a served model + a client); this crate stays proto-free and only owns the corpus + the
/// scorers it is fed.
///
/// # Errors
/// [`EvalError::Malformed`] if the embedded suite file is not well-formed JSON.
pub fn load_bench_v1() -> Result<BenchCorpus, EvalError> {
    let suite: GoldenSuite =
        serde_json::from_str(BENCH_V1_SUITE_JSON).map_err(|e| EvalError::Malformed {
            what: "bench-v1 suite",
            detail: e.to_string(),
        })?;
    Ok(BenchCorpus {
        suite,
        suite_digest: digest_hex(&[BENCH_V1_SUITE_JSON.as_bytes()]),
    })
}

/// The committed `golden-v1` baseline (embedded), the gate's default yardstick. Embedded
/// so the gate runs from an INSTALLED binary, not just the source tree.
///
/// # Errors
/// [`EvalError::Malformed`] if the embedded baseline is not well-formed JSON.
pub fn embedded_baseline() -> Result<Baseline, EvalError> {
    serde_json::from_str(BASELINE_JSON).map_err(|e| EvalError::Malformed {
        what: "baseline",
        detail: e.to_string(),
    })
}

/// blake3 over the concatenated blobs, as lowercase hex.
fn digest_hex(blobs: &[&[u8]]) -> String {
    let mut hasher = blake3::Hasher::new();
    for b in blobs {
        hasher.update(b);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_v1_loads_and_is_addressed() {
        let c = load_golden_v1().expect("golden-v1 corpus parses");
        assert_eq!(c.suite.id, GOLDEN_V1_ID);
        assert!(!c.suite.tasks.is_empty(), "suite has tasks");
        assert!(!c.format.cases.is_empty(), "format matrix has cases");
        assert_eq!(c.suite_digest.len(), 64, "blake3 hex is 64 chars");
        // The digest is a pure function of the embedded bytes ⇒ stable across calls.
        let again = load_golden_v1().expect("reload");
        assert_eq!(c.suite_digest, again.suite_digest);
    }

    #[test]
    fn bench_v1_loads_addressed_and_is_real_only() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        assert_eq!(c.suite.id, BENCH_V1_ID);
        assert!(!c.suite.tasks.is_empty(), "bench suite has tasks");
        assert_eq!(c.suite_digest.len(), 64, "blake3 hex is 64 chars");
        // Every bench task is REAL-ONLY: no scripted Tier-A fixture (scored live only),
        // and every task carries a non-empty instruction to send a served model.
        for t in &c.suite.tasks {
            assert!(
                t.scripted_transcript.is_none(),
                "bench task {} must be real-only (no scripted transcript)",
                t.id
            );
            assert!(
                !t.instruction.is_empty(),
                "bench task {} has an instruction",
                t.id
            );
        }
        // Content-addressed ⇒ stable, and distinct from golden-v1's corpus digest.
        let again = load_bench_v1().expect("reload");
        assert_eq!(c.suite_digest, again.suite_digest);
        assert_ne!(
            c.suite_digest,
            load_golden_v1().expect("golden-v1").suite_digest,
            "bench-v1 is a distinct corpus"
        );
    }

    #[test]
    fn bench_v1_covers_the_rc_substrate_families() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        // The coverage contract: every task belongs to one of the substrate families the
        // live runner knows how to drive, and all of them are present. A task tagged with
        // an unknown family would be silently driven down the default react path and score
        // a meaningless number.
        let families: std::collections::BTreeSet<&str> =
            c.suite.tasks.iter().map(|t| t.family.as_str()).collect();
        assert_eq!(
            families,
            [
                "adversarial",
                "failure",
                "http",
                "irrelevance",
                "long",
                "memory",
                "menu",
                "nlauthor",
                "react",
                "reach",
                "scaffold",
                "script",
                "swarm",
                "tool",
                "workflow"
            ]
            .into_iter()
            .collect(),
            "bench-v1 covers exactly the substrate families the runner drives"
        );
        // Each family carries at least one task — an empty family is coverage on paper.
        for f in [
            "tool",
            "react",
            "reach",
            "swarm",
            "script",
            "http",
            "failure",
            "menu",
            "long",
            "adversarial",
            "irrelevance",
            "memory",
            "scaffold",
            "workflow",
            // The NL authoring surface. Deliberately NOT in the tool-required list
            // below: a proposal fires no tools at all — the surface has none to
            // offer it — so demanding an expected-tool would turn "correctly called
            // nothing" into a corpus error.
            "nlauthor",
        ] {
            assert!(
                c.suite.tasks.iter().any(|t| t.family == f),
                "family {f} has at least one task"
            );
        }
        // A tool-required task must name the tools it expects; a contract/negative task
        // must name NONE (that emptiness IS its assertion — see `tool-contract-refusal`).
        // `failure`, `adversarial`, `irrelevance` and `memory` are deliberately absent
        // from this list: their point is often that a call must NOT happen, and
        // requiring an expectation would turn "fired nothing, correctly" into a corpus
        // error.
        for t in &c.suite.tasks {
            if ["tool", "script", "http", "menu", "long"].contains(&t.family.as_str()) {
                assert!(
                    !t.expect.expected_tools.is_empty(),
                    "{}-family task {} expects at least one tool call",
                    t.family,
                    t.id
                );
            }
        }
    }

    /// The `@`-scope namespace stays unambiguous: a per-family gate is
    /// `metric@[a-z]+`, a per-task pass^k gate is `pass_k4@<task-id-with-hyphens>`,
    /// and the machinery sentinel is `pass_k4@trials`. That only works if the three
    /// scope shapes can never collide — which this pins at the corpus.
    #[test]
    fn bench_v1_gate_scopes_cannot_collide() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        for t in &c.suite.tasks {
            assert!(
                t.family.chars().all(|ch| ch.is_ascii_lowercase()),
                "family {:?} must be lowercase letters only — a hyphenated family \
                 would collide with a per-task gate scope",
                t.family
            );
            assert!(
                t.id.contains('-'),
                "task id {:?} must contain a hyphen — an unhyphenated id would \
                 collide with a family gate scope",
                t.id
            );
            assert_ne!(t.family, "trials", "the sentinel scope is reserved");
            assert_ne!(
                t.family, "attempts",
                "the scaffold-completion sentinel scope is reserved"
            );
            assert_ne!(
                t.family, "timers",
                "the durable-wait sentinel scope is reserved"
            );
            assert_ne!(
                t.family, "retries",
                "the retry-attempts sentinel scope is reserved"
            );
        }
    }

    /// The `scaffold` family's canary discipline: the fact the answer must carry
    /// rides ONLY the task instruction (which becomes the SCAFFOLD goal — the run
    /// itself is driven with empty args off the stored, canary-free envelope). A
    /// canary that also appeared anywhere else in the corpus would let the run
    /// answer without the generated project ever reaching it.
    #[test]
    fn scaffold_canaries_ride_only_their_own_scaffold_goal() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        for t in c.suite.tasks.iter().filter(|t| t.family == "scaffold") {
            assert!(
                !t.expect.answer_must_contain.is_empty(),
                "{}: a scaffold task is canary-scored",
                t.id
            );
            for canary in &t.expect.answer_must_contain {
                assert!(
                    t.instruction.contains(canary),
                    "{}: the canary {canary:?} must ride the scaffold goal",
                    t.id
                );
                for other in &c.suite.tasks {
                    if other.id != t.id {
                        assert!(
                            !other.instruction.contains(canary),
                            "{}: canary {canary:?} leaks into task {}",
                            t.id,
                            other.id
                        );
                    }
                }
            }
            // The run consumes the GENERATED project, not tools — the tool scorers
            // must stay honestly N/A, and the family must never join the
            // tools-required list above.
            assert!(
                t.expect.expected_tools.is_empty(),
                "{}: no expected tools on a scaffold task",
                t.id
            );
        }
    }

    /// The pass^k flagship population is corpus data, covered by the suite digest —
    /// changing it is corpus drift, never a silent redefinition. Exactly three tasks,
    /// one per axis the reliability gate watches (a marginal react loop, the http
    /// family, the failure family).
    #[test]
    fn bench_v1_flagship_set_is_exactly_three() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        let flagship: Vec<&str> = c
            .suite
            .tasks
            .iter()
            .filter(|t| t.flagship)
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(
            flagship.len(),
            3,
            "the pass^k flagship set is three tasks, got {flagship:?}"
        );
    }

    /// A2, the de-hollowing proof: the chaining oracle FAILS when the chain is broken.
    ///
    /// This is the property `kv-then-calc` did not have. Its answer was 42+8=50, which a
    /// model reached having called one of its two tools — so it scored 1000 while proving
    /// nothing, and `task_success@tool` = 1000 was partly hollow with it.
    ///
    /// The test scores the SHIPPED expectation (not a hand-rolled copy) against two
    /// synthetic runs, so weakening the task in `suite.json` breaks the test rather than
    /// quietly restoring the hollowness. Deterministic and model-free: it cannot pass by
    /// luck the way a live run can.
    #[test]
    fn the_two_hop_oracle_fails_when_the_chain_is_broken() {
        use crate::scorers::{score_transcript, ScoreValue};
        use crate::transcript::{Branch, Transcript, TurnRecord};

        let c = load_bench_v1().expect("bench-v1 suite parses");
        let task = c
            .suite
            .tasks
            .iter()
            .find(|t| t.id == "kv-two-hop")
            .expect("kv-two-hop is the structurally-underivable chaining task");

        let kv_turn = |turn: u32| TurnRecord {
            turn,
            branch: Branch::Tool,
            tool_id: "mcp-kv/get".to_string(),
            tool_version: "1".to_string(),
            call_index: 0,
            rejection_reason: String::new(),
            raw: Vec::new(),
        };
        let run = |turns: Vec<TurnRecord>, answer: &str| Transcript {
            task_id: task.id.clone(),
            turns,
            final_answer: Some(answer.to_string()),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
            timing: None,
        };
        let answer_turn = |turn: u32| TurnRecord {
            turn,
            branch: Branch::Answer,
            tool_id: String::new(),
            tool_version: String::new(),
            call_index: 0,
            rejection_reason: String::new(),
            raw: Vec::new(),
        };
        let success_of = |t: &Transcript| {
            score_transcript(&crate::scorers::ScoreInput {
                transcript: t,
                expect: &task.expect,
            })
            .into_iter()
            .find(|s| s.metric_id == "task_success")
            .and_then(|s| match s.value {
                ScoreValue::Gate { per_mille } => Some(per_mille),
                ScoreValue::Spike { .. } => None,
            })
            .expect("task_success is always applicable and always a gate")
        };

        // BOTH hops fired and the terminal token was carried out ⇒ full marks.
        let chained = run(
            vec![kv_turn(0), kv_turn(1), answer_turn(2)],
            "The value is QUILL-MERIDIAN-58.",
        );
        assert_eq!(
            success_of(&chained),
            1000,
            "an actually-chained run scores full marks"
        );

        // The chain BROKEN: hop 1 fired, hop 2 never did, and the model answered with the
        // best thing it had — the intermediate key. There is no route from there to the
        // terminal token, so the oracle must read 0. If this ever reads 1000, the task has
        // become derivable and measures the model instead of the runtime.
        let broken = run(
            vec![kv_turn(0), answer_turn(1)],
            "The value is relay_node_7.",
        );
        assert_eq!(
            success_of(&broken),
            0,
            "a run that never made the second hop CANNOT satisfy the oracle"
        );
    }

    /// The `workflow` family's oracle discipline: every task runs a STORED definition
    /// whose steps are all deterministic, so the model never touches the answer and
    /// the tool scorers must stay honestly N/A. Its oracle tokens are FIXTURE-BORNE —
    /// they live only in the harness fixture's route bodies, never in any task
    /// instruction (a token in an instruction would be derivable without the run) —
    /// and the retry task's stateful fixture bans the family from the pass^k
    /// flagship set (a re-driven trial would find the depot already poisoned).
    #[test]
    fn workflow_oracles_are_fixture_borne_and_never_flagship() {
        let c = load_bench_v1().expect("bench-v1 suite parses");
        let tasks: Vec<_> = c
            .suite
            .tasks
            .iter()
            .filter(|t| t.family == "workflow")
            .collect();
        assert_eq!(tasks.len(), 7, "the workflow family is seven tasks");
        for t in &tasks {
            assert!(
                t.expect.expected_tools.is_empty(),
                "{}: a deterministic DAG has no expected tool calls",
                t.id
            );
            assert_eq!(
                t.expect.ideal_tool_calls, 0,
                "{}: tool-call efficiency is N/A for a stored DAG",
                t.id
            );
            assert!(
                !t.expect.answer_must_contain.is_empty(),
                "{}: a workflow task is oracle-scored",
                t.id
            );
            assert!(
                !t.flagship,
                "{}: the workflow family is never flagship",
                t.id
            );
            for canary in &t.expect.answer_must_contain {
                for other in &c.suite.tasks {
                    assert!(
                        !other.instruction.contains(canary.as_str()),
                        "{}: oracle {canary:?} must be fixture-borne, but task {} \
                         carries it in an instruction",
                        t.id,
                        other.id
                    );
                }
            }
        }
    }

    /// The conditional pair's de-hollowing proof: the sluice tasks fail when BOTH arms
    /// ran. Scored against the SHIPPED expectations (never a copy). Two layers catch a
    /// both-arms runtime live: the first-non-skip join fails closed on two survivors
    /// (the run dead-letters — no answer at all), and even a hypothetical aggregate
    /// that carried both gate orders scores 0 on the `answer_must_not_contain` gate
    /// (`injection_resistance` — the absence half of the answer oracle) on BOTH tasks.
    /// Each task's own single-order answer scores full marks on both gates. Weakening
    /// either expectation in `suite.json` breaks this test rather than quietly
    /// restoring the hollowness.
    #[test]
    fn the_conditional_pair_fails_when_both_arms_run() {
        use crate::scorers::{score_transcript, ScoreValue};
        use crate::transcript::{Branch, Transcript, TurnRecord};

        let c = load_bench_v1().expect("bench-v1 suite parses");
        let task_of = |id: &str| {
            c.suite
                .tasks
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("{id} is in the corpus"))
        };
        let high = task_of("workflow-conditional-high-reading");
        let low = task_of("workflow-conditional-low-reading");

        let answer_turn = TurnRecord {
            turn: 0,
            branch: Branch::Answer,
            tool_id: String::new(),
            tool_version: String::new(),
            call_index: 0,
            rejection_reason: String::new(),
            raw: Vec::new(),
        };
        let run = |task: &crate::suite::GoldenTask, answer: &str| Transcript {
            task_id: task.id.clone(),
            turns: vec![answer_turn.clone()],
            final_answer: Some(answer.to_string()),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 1,
            max_tool_calls: 0,
            timing: None,
        };
        let gate_of = |task: &crate::suite::GoldenTask, answer: &str, metric: &str| {
            score_transcript(&crate::scorers::ScoreInput {
                transcript: &run(task, answer),
                expect: &task.expect,
            })
            .into_iter()
            .find(|s| s.metric_id == metric)
            .and_then(|s| match s.value {
                ScoreValue::Gate { per_mille } => Some(per_mille),
                ScoreValue::Spike { .. } => None,
            })
            .unwrap_or_else(|| panic!("{metric} is applicable and a gate here"))
        };

        let both_arms = r#"{"order":"WEIR-OPEN-31"} {"order":"WEIR-SHUT-58"}"#;
        for (task, own) in [
            (high, r#"{"order":"WEIR-OPEN-31"}"#),
            (low, r#"{"order":"WEIR-SHUT-58"}"#),
        ] {
            assert_eq!(
                gate_of(task, both_arms, "injection_resistance"),
                0,
                "{}: an answer carrying BOTH gate orders fails the absence gate",
                task.id
            );
            assert_eq!(
                gate_of(task, own, "task_success"),
                1000,
                "{}: the taken-arm-only answer satisfies the presence gate",
                task.id
            );
            assert_eq!(
                gate_of(task, own, "injection_resistance"),
                1000,
                "{}: the taken-arm-only answer satisfies the absence gate",
                task.id
            );
        }
    }
}
