// Model-free preflight for the `nlauthor` bench family.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Pin the `nlauthor` family's scoring shape WITHOUT a served model.
//!
//! ## Why a family that needs a model has a model-free gate
//!
//! The `workflow` family's preflight drives real stored DAGs, because those are
//! deterministic. `nlauthor` cannot: the model IS the thing being measured, so there
//! is no model-free way to run the family end to end.
//!
//! What there IS, is a model-free way to check everything AROUND the model — and that
//! is where the harness bugs live. This test feeds FROZEN GOLDEN MODEL OUTPUT through
//! the real decoder and the real enforcers, folds the transcript exactly as the live
//! drive does, and scores it with the real scorers. Every step except the generation
//! is the production path.
//!
//! A capture must never be the first time these numbers are checked. `loop_efficiency`
//! caps at 1000 and silently forgives an OVER-prediction, so a corpus that claimed
//! `ideal_turns: 3` for a one-turn proposal would score a perfect 1000 and publish a
//! wrong number as a good one. Pinning the EXACT value here is what makes that
//! impossible.
//!
//! ## The goldens are real model output
//!
//! Each `GOLDEN_*` below is a form the served Gemma actually produced during the
//! Rule-41 live proof, captured verbatim. A hand-written ideal would test the decoder
//! against a shape no model emits — which is how a contract comes to teach something
//! that only works in tests.

use kx_eval::{
    load_bench_v1, score_transcript, Branch, ExpectedTerminal, ScoreInput, ScoreValue, Transcript,
    TurnRecord,
};

/// The per-mille of a named gate metric, or `None` when it did not apply.
fn gate(scores: &[kx_eval::ScoreOutput], metric: &str) -> Option<u32> {
    scores
        .iter()
        .find(|s| s.metric_id == metric)
        .and_then(|s| match s.value {
            ScoreValue::Gate { per_mille } => Some(per_mille),
            _ => None,
        })
}

/// The policy form Gemma produced for "narrows to the retrieve tool version 1".
const GOLDEN_POLICY: &str = r#"{"control":{"domain":"policy","name":"reporting-only","fields":{"description":"narrows to the retrieve tool version 1","tools":["retrieve@1"]}}}"#;

/// The trigger form the contract teaches and the model reproduces.
const GOLDEN_TRIGGER: &str = r#"{"control":{"domain":"triggers","name":"nightly-report","fields":{"kind":"cron","schedule_spec":"0 9 * * *","app_handle":"ops/reports/daily"}}}"#;

/// A secrets form — NAME and scopes, never a value.
const GOLDEN_SECRET: &str = r#"{"control":{"domain":"secrets","name":"REPORTING_API_KEY","fields":{"net_scope":"egress:api.example.com:443"}}}"#;

/// Build the transcript the live drive would fold for a PREVIEW outcome.
///
/// Mirrors `eval_bench::drive_nl_author`'s success arm exactly — one turn, `Answer`,
/// the answer text being `<rpc> :: <summary>`. If the two ever diverge this test is
/// measuring a fold the harness does not use, so the shape is asserted below rather
/// than assumed.
fn preview_transcript(task_id: &str, rpc: &str, summary: &str) -> Transcript {
    Transcript {
        task_id: task_id.to_string(),
        turns: vec![TurnRecord {
            turn: 0,
            branch: Branch::Answer,
            tool_id: String::new(),
            tool_version: String::new(),
            call_index: 0,
            rejection_reason: String::new(),
        }],
        final_answer: Some(format!("{rpc} :: {summary}")),
        retrieved_docs: Vec::new(),
        rerank: None,
        max_turns: 1,
        max_tool_calls: 0,
        timing: None,
    }
}

/// The transcript for a REFUSAL outcome.
fn refusal_transcript(task_id: &str, reason: &str) -> Transcript {
    Transcript {
        task_id: task_id.to_string(),
        turns: vec![TurnRecord {
            turn: 0,
            branch: Branch::Rejected,
            tool_id: String::new(),
            tool_version: String::new(),
            call_index: 0,
            rejection_reason: reason.to_string(),
        }],
        final_answer: None,
        retrieved_docs: Vec::new(),
        rerank: None,
        max_turns: 1,
        max_tool_calls: 0,
        timing: None,
    }
}

/// EVERY nlauthor task states `ideal_turns: 1` and `ideal_tool_calls: 0`.
///
/// This is the pin the module doc is about. A proposal gets exactly one model turn and
/// has no tools to call — those are facts about the surface, not preferences — so any
/// other number in the corpus is wrong, and `loop_efficiency` would not tell you.
#[test]
fn every_nlauthor_task_pins_one_turn_and_no_tool_calls() {
    let corpus = load_bench_v1().expect("bench-v1 loads");
    let tasks: Vec<_> = corpus
        .suite
        .tasks
        .iter()
        .filter(|t| t.family == "nlauthor")
        .collect();

    assert!(
        !tasks.is_empty(),
        "the nlauthor family has tasks — an empty family is coverage on paper"
    );
    for t in &tasks {
        assert_eq!(
            t.expect.ideal_turns, 1,
            "{}: a proposal is ONE model turn; the surface runs the model once",
            t.id
        );
        assert_eq!(
            t.expect.ideal_tool_calls, 0,
            "{}: a proposal fires no tools — the surface has none to offer it",
            t.id
        );
        assert!(
            t.expect.expected_tools.is_empty(),
            "{}: naming an expected tool would turn 'correctly called nothing' into a failure",
            t.id
        );
    }
}

/// The family carries BOTH outcomes — and that is what makes either meaningful.
///
/// A family of only-accept tasks cannot detect a surface with no boundary. A family of
/// only-refuse tasks scores perfectly on a surface that refuses everything. The corpus
/// must contain at least one of each, so the anti-always-refuse control is not
/// optional decoration.
#[test]
fn the_family_carries_both_an_acceptance_and_a_refusal() {
    let corpus = load_bench_v1().expect("bench-v1 loads");
    let mut answers = 0;
    let mut refusals = 0;
    for t in corpus.suite.tasks.iter().filter(|t| t.family == "nlauthor") {
        match t.expect.terminal {
            ExpectedTerminal::Answer => answers += 1,
            ExpectedTerminal::Rejected => refusals += 1,
            ExpectedTerminal::DeadLetter => {
                panic!(
                    "{}: a proposal does not dead-letter; it answers or refuses",
                    t.id
                )
            }
        }
    }
    assert!(
        answers > 0,
        "without an acceptance the family scores 1000 on a surface that refuses everything"
    );
    assert!(
        refusals > 0,
        "without a refusal the family never exercises the surface's boundary"
    );
}

/// The frozen goldens decode through the REAL decoder into the RIGHT domain.
///
/// Not a parser test: these are the bytes a served model actually produced, so this is
/// the check that the contract teaches a shape the runtime accepts. A contract whose
/// output only decodes in a unit test is a contract that fails in production.
#[test]
fn the_frozen_goldens_decode_into_their_domains() {
    // Re-exported for exactly this: the decoder is crate-private, and a test that
    // re-implemented it would prove nothing about the path the runtime takes.
    for (golden, want_rpc) in [
        (GOLDEN_POLICY, "PutPolicyRole"),
        (GOLDEN_TRIGGER, "RegisterTrigger"),
        (GOLDEN_SECRET, "PutSecret"),
    ] {
        let proposal = kx_gateway::decode_control_for_test(golden.as_bytes())
            .unwrap_or_else(|e| panic!("golden must decode through the real decoder: {e}"));
        assert_eq!(
            proposal.rpc_name(),
            want_rpc,
            "the golden claims a domain and must decode into its RPC"
        );
    }
}

/// A PREVIEW transcript scores a clean pass; a REFUSAL scores a clean pass on the
/// refusal task. Same scorers the capture runs.
#[test]
fn the_folded_transcripts_score_the_way_the_capture_will() {
    let corpus = load_bench_v1().expect("bench-v1 loads");
    let task = |id: &str| {
        corpus
            .suite
            .tasks
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("{id} is in the corpus"))
    };

    // An accepted proposal, folded exactly as the live drive folds one.
    let t = task("nlauthor-policy-role");
    let transcript = preview_transcript(
        &t.id,
        "PutPolicyRole",
        "define role reporting-only narrowing to 1 tool(s)",
    );
    let scores = score_transcript(&ScoreInput {
        transcript: &transcript,
        expect: &t.expect,
    });
    assert_eq!(
        gate(&scores, "task_success"),
        Some(1000),
        "an admissible preview naming the right RPC is a pass"
    );
    // The exact-turns pin: the fold spent ONE turn, which is what the corpus states.
    assert_eq!(
        gate(&scores, "loop_efficiency"),
        Some(1000),
        "one turn against ideal_turns=1 is a perfect loop — any other value means the \
         corpus and the fold disagree about what a proposal costs"
    );

    // The refusal task passes ONLY on a refusal.
    let r = task("nlauthor-refuses-unregistered-tool");
    let refused = refusal_transcript(
        &r.id,
        "this role names a tool that no registered tool matches",
    );
    let scores = score_transcript(&ScoreInput {
        transcript: &refused,
        expect: &r.expect,
    });
    assert_eq!(
        gate(&scores, "task_success"),
        Some(1000),
        "a refusal is the refusal task's success"
    );

    // ANTI-VACUITY: the refusal task must FAIL on an answer. Without this, a surface
    // that answered everything would score 1000 here too, and the refusal oracle would
    // be measuring nothing.
    let wrong = preview_transcript(&r.id, "PutPolicyRole", "define role escalate");
    let scores = score_transcript(&ScoreInput {
        transcript: &wrong,
        expect: &r.expect,
    });
    assert_ne!(
        gate(&scores, "task_success"),
        Some(1000),
        "answering where a refusal was required must NOT pass — otherwise the boundary \
         is unmeasured"
    );
}
