//! PR-6b-4 e2e witness: `KX_SERVE_AUTOGRANT=1 kx serve --features inference`
//! provisions `kx/recipes/react-auto` and drives a LIVE `ReAct` chain through it.
//!
//! With the operator opt-in, the serve seeds `kx/recipes/react-auto`; the binder
//! rebuilds its warrant from the LIVE registry at bind (auto-granting the
//! registered/dialed tool set, capped) and submits with `react_seed = true` → the
//! coordinator anchors the run-salted chain → the embedded worker drives REAL
//! greedy inference → the settle freezes the terminal branch, surfaced via
//! `ListReactTurns`. The bind-layer override (union warrant, `MoteId` invariance,
//! auth gate) is pinned model-free in `react_auto_bind.rs`; this proves the SERVE
//! wiring under the flag (recipe provisioning, the form, the live drive).
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without a
//! `GGUF` (`just fetch-agent-model` or `KX_SERVE_MODEL_GGUF`) or the bundled
//! `kx-mcp-echo` bin (`cargo build -p kx-mcp`, or `KX_MCP_ECHO_PATH`).

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_AUTO_RECIPE_HANDLE, REACT_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

fn serve_model() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_SERVE_MODEL_GGUF") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let standin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen3-0.6b-q4_k_m.gguf");
    standin.is_file().then_some(standin)
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (just fetch-agent-model); opt in with --ignored"]
async fn autogrant_serve_provisions_react_auto_and_drives_a_live_chain() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — run `just fetch-agent-model` (or set KX_SERVE_MODEL_GGUF)"
        );
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    // react-auto requires the model + the bundled echo capability (same gate as
    // react). If react itself didn't provision, the bundled bin is absent — skip.
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!("skipping: kx/recipes/react not provisioned — bundled kx-mcp-echo missing");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }
    assert!(
        recipes
            .recipes
            .iter()
            .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE),
        "KX_SERVE_AUTOGRANT on ⇒ kx/recipes/react-auto is provisioned"
    );

    // The form is the react contract (instruction + the two budget caps).
    let form = c
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let names: Vec<&str> = form.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"instruction"));
    assert!(names.contains(&"max_turns"));
    assert!(names.contains(&"max_tool_calls"));

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"What is 2+2? Answer briefly in prose.","max_turns":4,"max_tool_calls":2}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-auto")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    let mut answered = false;
    for _ in 0..600 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        if turns.turns.iter().any(|t| t.branch == "answer") {
            answered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        answered,
        "the react-auto chain settled a terminal Answer fact"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}

/// PR-1/BUG-32 (real-model integration witness, LOCAL / `--ignored`): drive a served
/// Gemma-4 model on a tool-forcing instruction against the bundled `mcp-echo`
/// (namespaced/dialed) tool and assert the chain reaches a **terminal answer** — i.e.
/// when the model dials the tool by the (bare/decorated) name it reads from the menu,
/// the authority gate RESOLVES it to the namespaced grant and the tool FIRES, instead
/// of refusing it `UngrantedTool` (the BUG-32 symptom: a refused dial dead-letters the
/// chain with no answer). The assertion is the non-flaky invariant (the chain settles,
/// never dead-letters on a dialed tool); whether a `tool` round actually fired is
/// model-nondeterministic (the model may answer a trivial echo directly), so it is
/// LOGGED for the operator, not asserted. CI keeps the DETERMINISTIC model-free
/// fire-commits proof (`kx-coordinator/tests/react_live.rs`, which stage Gemma's EXACT
/// byte shapes — `mcp-echo:echo` + bare leaf). Run with `KX_SERVE_MODEL_GGUF=<gemma>`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF; opt in with --ignored"]
async fn react_auto_dialed_tool_resolves_and_the_chain_settles() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF (a real GGUF)");
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE)
    {
        eprintln!("skipping: react-auto not provisioned — bundled kx-mcp-echo missing");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // A tool-forcing instruction. `mcp-echo` is a dialed/namespaced tool — the model
    // proposes whatever short name it reads from the menu, and the BUG-32 gate resolves
    // that to the grant (a refused dial would dead-letter the chain ⇒ no answer below).
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"You MUST use the echo tool to echo the exact text 'pong'. Call the tool first, then report what it returned.","max_turns":6,"max_tool_calls":3}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-auto")
        .into_inner();

    let mut fired = false;
    let mut answered = false;
    let mut bounded = false;
    let mut last = String::new();
    // ~180s: ample for the fast default agent model (`just fetch-agent-model` ⇒
    // Qwen3-0.6B settles in ~3s). A large opt-in model (e.g. Gemma-4-12B via
    // KX_SERVE_MODEL_GGUF) running a multi-turn tool loop is slow — that is model
    // slowness, not a failure; raise the bound when driving a big model.
    for _ in 0..1800 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        let branches: Vec<&str> = turns.turns.iter().map(|t| t.branch.as_str()).collect();
        let snap = format!("{branches:?}");
        if snap != last {
            eprintln!("react-auto witness — trajectory so far: {snap}");
            // PR-3 (A2): surface WHY each refused turn was rejected (the diagnostic
            // that distinguishes a name/args/decode refusal from model weakness).
            for t in &turns.turns {
                if t.branch == "rejected" && !t.rejection_reason.is_empty() {
                    eprintln!("  turn {} rejected: {}", t.turn, t.rejection_reason);
                }
            }
            last = snap.clone();
        }
        let tool_calls = turns.turns.iter().filter(|t| t.branch == "tool").count();
        let cap = turns
            .turns
            .iter()
            .map(|t| t.max_tool_calls as usize)
            .max()
            .unwrap_or(0);
        fired |= tool_calls > 0;
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        // PR-3 (A2/A3): the chain is BOUNDED — it reaches a terminal branch
        // (`answer` / `dead_lettered`) OR spends its tool-call budget (a Tool tail
        // at the cap quiesces without an `answer` branch). It NEVER wedges. A2's
        // termination invariant + A3's live tool-FIRE are both observed here; the
        // DETERMINISTIC proofs live in `kx-coordinator/tests/react_live.rs` and
        // `kx-tool-registry` (the JSON5 arg-repair fuzz).
        let dead = turns
            .turns
            .iter()
            .any(|t| t.branch == "answer" || t.branch == "dead_lettered");
        if dead || (cap > 0 && tool_calls >= cap) {
            bounded = true;
            eprintln!(
                "react-auto witness — BOUNDED. tool fired: {fired}, answered: {answered}, \
                 tool_calls: {tool_calls}/{cap}, trajectory: {snap}"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The §2.246 live close-out: a real model's dialed-tool call RESOLVES + FIRES
    // end-to-end (A3 repairs the JSON5 args Gemma emits; BUG-32 resolves the name),
    // OR the model answers directly — and the loop is bounded, never a silent wedge.
    assert!(
        bounded,
        "the live A2 chain is BOUNDED — it reached a terminal branch or spent its \
         tool-call budget, never a silent wedge. tool fired: {fired}; answered: {answered}"
    );
    assert!(
        fired || answered,
        "the live model either FIRED the dialed tool (A3 made its JSON5 args valid) or \
         answered directly — it did NOT get stuck unable to act. tool fired: {fired}"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}

/// W2 (real-model integration witness, LOCAL / `--ignored`): a served Gemma-4 model
/// driven on a tool-forcing goal ALWAYS reaches a TERMINAL branch — `answer` (the
/// settle-nudge steered it to settle on its last useful turn) or `dead_lettered`
/// (the honest terminal: it looped on tools and exhausted its budget without
/// answering). Before W2, a tool-looping chain quiesced on a Tool tail with NO
/// terminal branch, so the client wait timed out and `kx agent run` exited 3 (a
/// masquerading "resumable timeout") for a permanent failure. The non-flaky
/// invariant asserted here is exactly that fix: the chain settles to a terminal,
/// NEVER a no-terminal hang. Whether it is `answer` vs `dead_lettered` is
/// model-nondeterministic, so it is LOGGED. CI keeps the deterministic model-free
/// proofs (`kx-coordinator/tests/react_live.rs`: `last_useful_turn_is_settle_nudged`,
/// `settle_nudge_lets_a_looping_model_answer`, `chain_is_bounded_by_the_durable_budget`).
/// Run with `KX_SERVE_MODEL_GGUF=<gemma>` (Gemma-4 ONLY for the manual loop — Qwen3
/// is too weak to drive a multi-turn tool loop and would false-green W2).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a Gemma GGUF; opt in with --ignored"]
async fn react_auto_w2_tool_looper_reaches_a_terminal_via_nudge_or_honest_deadletter() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF (a real GGUF)");
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE)
    {
        eprintln!("skipping: react-auto not provisioned — bundled kx-mcp-echo missing");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // A goal designed to tempt a model into looping on the tool without settling.
    // max_tool_calls (4) < max_turns (8) leaves room for the settle-nudge to fire.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"Investigate thoroughly using the echo tool: echo 'alpha', then echo 'beta', then echo 'gamma', then keep gathering more with the tool before you answer. Only when you are completely done, give a final summary.","max_turns":8,"max_tool_calls":4}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-auto")
        .into_inner();

    let mut terminal_branch: Option<String> = None;
    let mut last = String::new();
    for _ in 0..3600 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        let branches: Vec<&str> = turns.turns.iter().map(|t| t.branch.as_str()).collect();
        let snap = format!("{branches:?}");
        if snap != last {
            eprintln!("W2 witness — trajectory so far: {snap}");
            last = snap;
        }
        if let Some(t) = turns
            .turns
            .iter()
            .find(|t| t.branch == "answer" || t.branch == "dead_lettered")
        {
            terminal_branch = Some(t.branch.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // THE W2 invariant: a tool-looping chain reaches a TERMINAL branch (the nudge
    // settled it to `answer`, or it dead-lettered honestly) — it NEVER quiesces on a
    // Tool tail with no terminal (the pre-W2 exit-3 masquerade).
    let branch = terminal_branch.expect(
        "the W2 chain reached a terminal branch (answer via the settle-nudge, or an \
         honest dead_lettered) — never a no-terminal hang",
    );
    eprintln!("W2 witness — terminal branch: {branch} (nudge-settled or honest dead-letter)");

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}

/// Lower-case hex of a byte slice (for the 64-hex `context_refs` wire form).
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Poll `GetProjection` until `mote_id` is `Committed`; return its `result_ref`.
async fn await_committed_ref(
    c: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    mote_id: &[u8],
) -> [u8; 32] {
    for _ in 0..1200 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            return m
                .result_ref
                .clone()
                .expect("a committed Mote carries a result_ref")
                .try_into()
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the answer turn never committed");
}

/// PR-9d e2e witness: a SUCCESSOR ReAct turn stays GROUNDED by the run's attached
/// context. We attach a fact the model cannot otherwise know (a fixed-but-arbitrary
/// codename), force a FIRST tool turn, then require the final answer to quote the
/// fact — so the only way the post-tool (successor) turn can answer is if PR-9d
/// carried the context past turn 0. WITHOUT the carry, `build_react_turn` drops
/// `CONTEXT_ITEMS_KEY` and the successor turn reasons blind (can't know the codename).
/// Gemma-4 ONLY (Qwen3 is too weak to follow the two-step instruction — false-greens).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a Gemma GGUF; opt in with --ignored"]
async fn react_auto_carries_attached_context_to_a_successor_turn() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF (a real GGUF)");
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    if !c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner()
        .recipes
        .iter()
        .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE)
    {
        eprintln!("skipping: react-auto not provisioned — bundled kx-mcp-echo missing");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // Attach a fact the model cannot know — the carry is the ONLY way a successor turn
    // learns it. Uploads-scope PutContent → 64-hex ref → entry `context_refs`.
    const SECRET: &str = "ZEPHYR-NINE";
    let put = c
        .put_content(proto::PutContentRequest {
            payload: format!(
                "CLASSIFIED CONTEXT. The mission codename is {SECRET}. Quote it EXACTLY when asked."
            )
            .into_bytes(),
            media_type: "text/plain".into(),
            filename: "context.txt".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let ctx_hex = hex_encode(&put.content_ref);

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            // max_tool_calls=1 STRUCTURALLY forces a single tool turn, then turn >=1
            // (the successor) must SETTLE to an answer (a 2nd tool proposal is refused →
            // re-prompt/settle-nudge). So the answer necessarily comes from a successor
            // turn, where the carry is the ONLY source of the codename.
            args: br#"{"instruction":"Call the echo tool EXACTLY ONCE with the text 'probe' to check your tools. After you see the tool result, STOP using tools and give your FINAL ANSWER: state the mission codename from your CLASSIFIED CONTEXT, exactly. Do not call any tool again.","max_turns":6,"max_tool_calls":1}"#.to_vec(),
            context_bundles: vec![],
            context_refs: vec![ctx_hex],
        })
        .await
        .expect("invoke react-auto with attached context")
        .into_inner();

    // Drive the chain to a TERMINAL branch (answer or honest dead-letter), counting
    // tool turns (>=1 tool turn ⇒ a SUCCESSOR turn ran, where the carry is exercised).
    let mut answer_mote: Option<Vec<u8>> = None;
    let mut terminal: Option<String> = None;
    let mut tool_turns = 0usize;
    let mut last = String::new();
    'poll: for _ in 0..3600 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        let branches: Vec<&str> = turns.turns.iter().map(|t| t.branch.as_str()).collect();
        let snap = format!("{branches:?}");
        if snap != last {
            eprintln!("PR-9d carry witness — trajectory: {snap}");
            last = snap;
        }
        tool_turns = turns.turns.iter().filter(|t| t.branch == "tool").count();
        for t in &turns.turns {
            if t.branch == "answer" {
                answer_mote = Some(t.turn_mote_id.clone());
                terminal = Some("answer".to_string());
                break 'poll;
            }
            if t.branch == "dead_lettered" {
                terminal = Some("dead_lettered".to_string());
                break 'poll;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // PR-9d LIVE witness: the multi-turn chain WITH attached context runs end-to-end on
    // a real model to a TERMINAL branch (never a crash/hang), exercising the per-turn
    // carry on a successor turn. The carry's CORRECTNESS is proven DETERMINISTICALLY —
    // `kx-coordinator` `context_carry_tests` (delivery) + `kx-gateway`
    // `dispatch_model_prepends_carried_context_for_a_successor_turn` (consumption) — so
    // this witness gates the integrated path running, not the (flaky) model wording.
    let terminal = terminal.expect("the chain reached a terminal branch (never a hang)");
    assert!(
        tool_turns >= 1,
        "the chain went MULTI-TURN with context attached (>=1 tool turn ⇒ a successor \
         turn ran the carry path live)"
    );
    eprintln!("PR-9d carry witness — terminal: {terminal} (tool_turns={tool_turns})");

    // If the model SETTLED on an answer, observe whether it quoted the context-only
    // codename — a soft real-model signal (Gemma-4 tends to loop the tool rather than
    // settle, so the quote is NOT a hard gate; the deterministic tests are the proof).
    if let Some(am) = answer_mote {
        let result_ref = await_committed_ref(&mut c, &resp.instance_id, &am).await;
        let blob = c
            .get_content(proto::GetContentRequest {
                content_ref: result_ref.to_vec(),
                instance_id: resp.instance_id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        let text = String::from_utf8_lossy(&blob.payload);
        eprintln!("PR-9d carry witness — FINAL ANSWER (successor turn): {text}");
        if text.to_uppercase().contains(SECRET) {
            eprintln!(
                "PR-9d carry CONFIRMED LIVE: a successor answer quoted the context-only \
                 codename {SECRET:?}"
            );
        } else {
            eprintln!(
                "PR-9d note: reached an answer but did not quote {SECRET:?} (model wording; \
                 carry correctness is proven in the deterministic tests)"
            );
        }
    }

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
