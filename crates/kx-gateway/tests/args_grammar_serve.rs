//! Does the typed tool-args grammar actually CONSTRAIN a served turn?
//!
//! Everything below the model was already proven: the schema renders (kx-grammar),
//! it reaches both engine legs, and `validate_args` refuses a bad call fail-closed
//! before any effect fires (`kx-model-harness::broker`). What no test answered is
//! the only question that needs a model to answer — **does a real decode obey it?**
//!
//! That is a behavioural claim, so it needs a control that can FAIL. The two arms
//! here differ in exactly one variable: `KX_SERVE_REACT_GRAMMAR`. It does not touch
//! the prompt (the tool menu is gated independently on `KX_SERVE_REACT_TOOL_MENU`),
//! so both arms see byte-identical instructions and differ only in whether the
//! decoder is masked.
//!
//! The oracle reads the **emitted args**, not whether the call succeeded. A
//! grammar defect preserves availability — the turn still answers, the broker
//! still refuses a bad call — so any gate asking "did it work?" is blind to it.
//!
//! ⚠ FEATURE GATE. This file is `serve-engine`, deliberately NOT `inference`.
//! `inference = ["serve-engine", ...]` is one-directional, so an `inference`-gated
//! test compiles to an empty harness under
//! `console,serve-engine,hnsw,hosted-apps,observability` — the exact set the live
//! proofs build, and the reason a sibling live file has never run there. Gated
//! this way the file runs on BOTH builds and picks its engine at runtime.
//!
//! ```text
//! # Ollama leg (the RC-shaped build):
//! KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma4:12b \
//!   cargo test -p kx-gateway --features serve-engine,hnsw \
//!   --test args_grammar_serve -- --ignored --nocapture --test-threads=1
//!
//! # llama.cpp leg:
//! KX_SERVE_MODEL_GGUF=~/.kx-models/gemma-4-12b-it-q4_k_m.gguf \
//!   cargo test -p kx-gateway --features inference,hnsw \
//!   --test args_grammar_serve -- --ignored --nocapture --test-threads=1
//! ```
#![cfg(feature = "serve-engine")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// `mcp-calc/calc@1`'s declared `op` vocabulary — the exact set
/// `crates/kx-gateway/src/mcp_tool.rs` registers. Read from the tool's own
/// registration rather than pinned here would be better still; it is pinned
/// because the test must fail if the registration silently loses its enum.
const ALLOWED_OPS: [&str; 4] = ["add", "div", "mul", "sub"];

/// The tool the model is asked to call.
const CALC_TOOL: &str = "mcp-calc/calc";

/// The task, phrased so the natural word for the operation is NOT in the
/// declared vocabulary: a model says "multiply", the schema says `mul`. The
/// gap is what the unconstrained arm has to fall into.
const TASK: &str = "What is 6 multiplied by 7? You must use the calculator tool to work it out.";

fn serve_gguf() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var("KX_SERVE_MODEL_GGUF").ok()?);
    p.is_file().then_some(p)
}

fn ollama_opted_in() -> bool {
    std::env::var("KX_SERVE_OLLAMA").is_ok_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        matches!(v.as_str(), "1" | "on" | "true" | "yes")
    })
}

/// Resolve the engine for this run, ONE per run. Ollama opt-in wins; otherwise a
/// GGUF. `None` ⇒ the caller must skip, loudly.
fn resolve_engine() -> Option<&'static str> {
    if ollama_opted_in() {
        return Some("ollama");
    }
    let gguf = serve_gguf()?;
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    Some("llamacpp")
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// Pull the `args` object out of a turn's RAW emitted text.
///
/// The text is whatever the model produced, which on the llama.cpp leg may carry
/// prose around the envelope (the GBNF is lazy — it triggers on the `{"tool_call"`
/// opener). So: find the key, walk back to the enclosing `{`, brace-match forward
/// with string/escape awareness, parse.
fn tool_call_args(raw: &str) -> Option<serde_json::Value> {
    let key = raw.find("\"tool_call\"")?;
    let start = raw[..key].rfind('{')?;
    let bytes = raw.as_bytes();
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let v: serde_json::Value = serde_json::from_str(raw.get(start..=i)?).ok()?;
                    return v.get("tool_call")?.get("args").cloned();
                }
            }
            _ => {}
        }
    }
    None
}

/// One turn in which the runtime resolved a TOOL, and what the model actually
/// emitted for it.
///
/// ⚠ The two variants exist because the first version of this instrument could not
/// see its own subject. It looked only for the quoted `"tool_call"` JSON key, so
/// when the unconstrained arm emitted Gemma-4's NATIVE syntax —
/// `<|tool_call>call:mcp-calc/calc{op: "mul",a:6,b:7}<tool_call|>`, not JSON at all
/// — it parsed nothing and reported "no violations", which is the same reading it
/// gives for a perfectly obedient arm. Being off-envelope entirely is the LARGEST
/// violation available, and the instrument was blind to precisely that.
enum Emission {
    /// The model emitted the constrained envelope and it parsed.
    Envelope(serde_json::Value),
    /// A tool was proposed, but not as the constrained JSON envelope.
    OffEnvelope(String),
}

/// What one arm produced. Raw texts are kept so a failure prints what was actually
/// seen rather than only what was expected.
struct ArmOutcome {
    emissions: Vec<Emission>,
    raws: Vec<String>,
}

impl ArmOutcome {
    /// Every args object that parsed as the constrained envelope.
    fn args(&self) -> Vec<&serde_json::Value> {
        self.emissions
            .iter()
            .filter_map(|e| match e {
                Emission::Envelope(a) => Some(a),
                Emission::OffEnvelope(_) => None,
            })
            .collect()
    }

    /// How many tool proposals this arm made, in any shape.
    fn proposals(&self) -> usize {
        self.emissions.len()
    }

    /// The `op` values emitted across the arm's parsed envelopes.
    fn ops(&self) -> Vec<String> {
        self.args()
            .iter()
            .filter_map(|a| a.get("op").and_then(|v| v.as_str()).map(str::to_owned))
            .collect()
    }

    /// Emitted keys the schema does not declare (`deny_unknown` forbids them).
    fn undeclared_keys(&self) -> BTreeSet<String> {
        let declared: BTreeSet<&str> = ["op", "a", "b"].into_iter().collect();
        self.args()
            .iter()
            .filter_map(|a| a.as_object())
            .flat_map(|o| o.keys())
            .filter(|k| !declared.contains(k.as_str()))
            .cloned()
            .collect()
    }

    /// Values for `a`/`b` that are not JSON integers (a float or a string).
    fn non_integer_operands(&self) -> Vec<serde_json::Value> {
        self.args()
            .iter()
            .flat_map(|a| ["a", "b"].into_iter().filter_map(move |k| a.get(k)))
            .filter(|v| !v.is_i64() && !v.is_u64())
            .cloned()
            .collect()
    }

    /// Every way this arm's output breaks the declared schema, worst first.
    fn violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for e in &self.emissions {
            if let Emission::OffEnvelope(raw) = e {
                out.push(format!(
                    "proposed a tool OUTSIDE the constrained envelope: {raw:?}"
                ));
            }
        }
        for op in self.ops() {
            if !ALLOWED_OPS.contains(&op.as_str()) {
                out.push(format!("op={op:?} is outside the declared enum"));
            }
        }
        for k in self.undeclared_keys() {
            out.push(format!("undeclared arg key {k:?}"));
        }
        for v in self.non_integer_operands() {
            out.push(format!("non-integer operand {v}"));
        }
        out
    }
}

/// Drive one arm to a terminal branch and collect what the model emitted.
async fn run_arm(engine: &str, label: &str) -> ArmOutcome {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // PRECONDITION, asserted rather than skipped: without the calc tool the arm
    // would emit nothing and BOTH arms would agree — a false green in which the
    // constrained arm "never violates" because it never called anything.
    let tools = c
        .discover_tools(proto::DiscoverToolsRequest::default())
        .await
        .expect("DiscoverTools")
        .into_inner();
    assert!(
        tools.tools.iter().any(|t| t.tool_name == CALC_TOOL),
        "[{label}] {CALC_TOOL} must be registered for this oracle to mean anything \
         (build it with `cargo build -p kx-mcp --bins`, or set KX_MCP_CALC_PATH); saw {:?}",
        tools.tools.iter().map(|t| &t.tool_name).collect::<Vec<_>>()
    );

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: kx_gateway::REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: serde_json::to_vec(&serde_json::json!({
                "instruction": TASK,
                "max_turns": 4,
                "max_tool_calls": 3,
            }))
            .unwrap(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke react-auto")
        .into_inner();

    let mut settled = None;
    for _ in 0..1800 {
        let t = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        if t.turns
            .iter()
            .any(|x| x.branch == "answer" || x.branch == "dead_lettered")
        {
            settled = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let turns = settled
        .unwrap_or_else(|| panic!("[{label}] the chain never settled a terminal branch"))
        .turns;

    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: resp.instance_id.clone(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();

    let (mut emissions, mut raws) = (Vec::new(), Vec::new());
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for t in &turns {
        let Some(rref) = view
            .motes
            .iter()
            .find(|m| m.mote_id == t.turn_mote_id)
            .and_then(|m| m.result_ref.clone())
        else {
            continue;
        };
        let Ok(content) = c
            .get_content(proto::GetContentRequest {
                content_ref: rref,
                instance_id: resp.instance_id.clone(),
            })
            .await
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&content.into_inner().payload).into_owned();
        eprintln!(
            "  [{label}/{engine}] turn={} branch={} tool={} rejected={:?} raw={:?}",
            t.turn, t.branch, t.tool_id, t.rejection_reason, text
        );
        raws.push(text.clone());

        // `ListReactTurns` reports the same emission on both the `pending` and the
        // resolved row for a turn, so dedupe on the text — otherwise one tool call
        // counts twice and "2 calls" means one.
        if !seen.insert(text.clone()) {
            continue;
        }
        // A tool was PROPOSED iff the runtime resolved one for this turn. That is
        // the runtime's own judgement, not the instrument's — so an emission in a
        // dialect the instrument does not parse still counts, instead of vanishing.
        let proposed_a_tool = t.branch == "tool" || !t.tool_id.is_empty();
        if !proposed_a_tool {
            continue;
        }
        match tool_call_args(&text) {
            Some(a) => emissions.push(Emission::Envelope(a)),
            None => emissions.push(Emission::OffEnvelope(text)),
        }
    }
    running.shutdown().await.unwrap();
    ArmOutcome { emissions, raws }
}

/// THE ORACLE. One variable, two arms, an assertion over what the model EMITTED.
///
/// Constrained: every emitted `op` is inside the declared enum, every operand is
/// an integer, no undeclared key appears. Unconstrained: at least one of those
/// breaks — and if it does not, this test FAILS, because a control that cannot
/// fail makes the constrained arm's green worth nothing. An inconclusive oracle
/// must never read as a pass.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs Ollama (KX_SERVE_OLLAMA=on) or a GGUF; opt in with --ignored"]
async fn typed_args_constrain_a_served_decode_and_the_unconstrained_arm_does_not() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_OLLAMA=on or KX_SERVE_MODEL_GGUF");
        return;
    };
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    // ---- arm A: the grammar armed (the default posture) ----
    std::env::remove_var("KX_SERVE_REACT_GRAMMAR");
    eprintln!("ARGS-GRAMMAR [{engine}] arm A — constrained (KX_SERVE_REACT_GRAMMAR unset ⇒ on)");
    let constrained = run_arm(engine, "constrained").await;

    // ---- arm B: the SAME prompt, decoder unmasked ----
    std::env::set_var("KX_SERVE_REACT_GRAMMAR", "0");
    eprintln!("ARGS-GRAMMAR [{engine}] arm B — control (KX_SERVE_REACT_GRAMMAR=0)");
    let control = run_arm(engine, "control").await;
    std::env::remove_var("KX_SERVE_REACT_GRAMMAR");
    std::env::remove_var("KX_SERVE_AUTOGRANT");

    let (a_viol, b_viol) = (constrained.violations(), control.violations());
    eprintln!(
        "ARGS-GRAMMAR [{engine}] constrained: {} proposal(s), {} parsed as the envelope, \
         ops={:?}, violations={a_viol:?}",
        constrained.proposals(),
        constrained.args().len(),
        constrained.ops()
    );
    eprintln!(
        "ARGS-GRAMMAR [{engine}] control:     {} proposal(s), {} parsed as the envelope, \
         ops={:?}, violations={b_viol:?}",
        control.proposals(),
        control.args().len(),
        control.ops()
    );

    // The constrained arm must have actually called something — otherwise "no
    // violation" is true of a turn that never happened.
    assert!(
        constrained.proposals() > 0,
        "[{engine}] the constrained arm proposed no tool at all, so it constrained nothing. \
         Raw turns: {:?}",
        constrained.raws
    );
    // Both arms must reach the tool, or the comparison is between a turn that
    // happened and one that did not.
    assert!(
        control.proposals() > 0,
        "[{engine}] the control arm proposed no tool, so there is nothing to compare against. \
         Raw turns: {:?}",
        control.raws
    );

    // THE NO-EFFECT SIGNATURE, checked before the arm assertions so the failure says
    // what is actually wrong. Identical violations on both sides means the variable
    // did nothing — which is a different defect from "the constraint was violated",
    // and reporting it as the latter sends the reader to the wrong place.
    assert_ne!(
        a_viol, b_viol,
        "[{engine}] the grammar had NO OBSERVABLE EFFECT: both arms emitted the same thing, \
         so `KX_SERVE_REACT_GRAMMAR` changed nothing.\n\
         On the llama.cpp leg the GBNF is LAZY — `kx-inference` arms it with the trigger \
         `[\\s\\S]*?(\\{{[ \\t\\n]*\"tool_call\")`, so it only starts masking once the model \
         has already opened a JSON object with a quoted `tool_call` key. A model that emits \
         its OWN tool syntax (Gemma-4 opens `<|tool_call>call:…`) never matches the trigger, \
         the sampler stage never engages, and the typed args constrain nothing.\n\
         Constrained: {a_viol:?}\nControl: {b_viol:?}"
    );

    assert!(
        a_viol.is_empty(),
        "[{engine}] the CONSTRAINED arm emitted args the declared schema forbids: {a_viol:?}. \
         Raw turns: {:?}",
        constrained.raws
    );

    assert!(
        !b_viol.is_empty(),
        "[{engine}] INCONCLUSIVE, not a pass: the unconstrained control emitted nothing the \
         schema forbids ({} proposal(s), ops={:?}), so the constrained arm's clean result is not \
         evidence the grammar did anything — the model may simply comply unprompted on this \
         task. Record this rather than re-running until it violates. Raw turns: {:?}",
        control.proposals(),
        control.ops(),
        control.raws
    );
}
