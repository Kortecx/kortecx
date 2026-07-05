//! LIVE (`#[ignore]`) proof of `T-GEMMA3-TOOL-LOOP-ANSWER-FORCE` on Ollama gemma3:12b.
//!
//! The deterministic tests prove the ARMING (the gateway sets `answer_only` on a
//! duplicate-rejection re-prompt / near-budget settle-nudge — `kx-gateway` +
//! `kx-coordinator`). THIS test proves the LIVE half on the real model: with the
//! answer-only `format` applied, gemma3 is FORCED to emit a parseable `{"answer":…}`
//! envelope (it settles) even for a prompt that would otherwise elicit a tool call —
//! whereas the sibling UNION `format` on the SAME prompt fires the tool. That contrast
//! is the answer-force capability's GR28 live witness on the Ollama engine. (llama.cpp
//! needs no witness: its GBNF renderer ignores `answer_only` and already completes the
//! loop — proven by `kx-grammar`'s `gbnf_ignores_answer_only`.)
//!
//! Requires a running Ollama daemon serving `gemma3:12b`. Opt in:
//!   KX_SERVE_OLLAMA=on cargo test -p kx-ollama --test answer_force_live -- --ignored --nocapture
//! NOT in CI (no live model); skips cleanly when not opted in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::field_reassign_with_default
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use kx_grammar::{ToolEnvelopeSpec, ToolSpec};
use kx_inference::{InferenceBackend, InferenceInput, InferenceParams};
use kx_mote::{Grammar, ModelId};
use kx_ollama::{OllamaBackend, OllamaClient};
use kx_warrant::WarrantSpec;

const MODEL: &str = "gemma3:12b";

/// Truthy `KX_SERVE_OLLAMA` — the operator opt-in the live serve tests share.
fn opted_in() -> bool {
    std::env::var("KX_SERVE_OLLAMA")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(false)
}

fn warrant(max_out: u32) -> WarrantSpec {
    let mut w = WarrantSpec::default();
    w.model_route.model_id = ModelId(MODEL.to_string());
    w.model_route.max_output_tokens = max_out;
    w.resource_ceiling.wall_clock_ms = 60_000;
    w
}

fn params(max_out: u32, grammar: Option<Grammar>) -> InferenceParams {
    let mut p = InferenceParams::default();
    p.max_output_tokens = max_out;
    p.grammar = grammar;
    p
}

fn grammar_of(spec: ToolEnvelopeSpec) -> Grammar {
    Grammar::new(spec.to_raw().unwrap())
}

fn dispatch(backend: &OllamaBackend, prompt: &str, grammar: Option<Grammar>) -> Vec<u8> {
    backend
        .dispatch(
            &ModelId(MODEL.to_string()),
            &InferenceInput::text(prompt),
            &params(256, grammar),
            &warrant(256),
        )
        .expect("live gemma3 dispatch")
        .bytes
}

#[test]
#[ignore = "live Ollama gemma3:12b; opt in with KX_SERVE_OLLAMA=on --ignored"]
fn answer_only_format_forces_gemma3_to_settle_live() {
    if !opted_in() {
        eprintln!("skipping: set KX_SERVE_OLLAMA=on (needs a running Ollama daemon + gemma3:12b)");
        return;
    }
    let client = Arc::new(OllamaClient::new("http://127.0.0.1:11434", false).unwrap());
    let mut models = BTreeSet::new();
    models.insert(MODEL.to_string());
    let backend = OllamaBackend::new(client, models);

    // A prompt that NECESSITATES the tool (so the contrast is meaningful).
    let prompt = "Read the recent messages in channel C0123ABCD using the slack/read_channel \
                  tool, then summarize them.";
    let tools = vec![ToolSpec::new("slack/read_channel", "1")];

    // (1) The UNION (answerable) format: the tool-eliciting prompt FIRES — the output
    //     carries a `tool_call` (this is the #293 firing guarantee, re-confirmed here).
    let union_out = dispatch(
        &backend,
        prompt,
        Some(grammar_of(
            ToolEnvelopeSpec::new(tools.clone()).with_answerable(true),
        )),
    );
    let union_v: serde_json::Value =
        serde_json::from_slice(&union_out).expect("union ⇒ parseable JSON");
    eprintln!(
        "[answer-force live] UNION out = {}",
        String::from_utf8_lossy(&union_out)
    );

    // (2) The ANSWER-ONLY format on the SAME prompt: gemma3 is FORCED to settle — the
    //     output is a parseable `{"answer":…}` envelope with NO `tool_call` arm. This is
    //     the answer-force: the model cannot loop on a tool call; it must answer.
    let answer_out = dispatch(
        &backend,
        prompt,
        Some(grammar_of(
            ToolEnvelopeSpec::new(tools).with_answer_only(true),
        )),
    );
    eprintln!(
        "[answer-force live] ANSWER-ONLY out = {}",
        String::from_utf8_lossy(&answer_out)
    );
    let answer_v: serde_json::Value =
        serde_json::from_slice(&answer_out).expect("answer-only ⇒ parseable JSON");
    assert!(
        answer_v.get("answer").is_some(),
        "answer-only forces an {{\"answer\"}} envelope: {answer_v}"
    );
    assert!(
        answer_v.get("tool_call").is_none(),
        "answer-only drops the tool_call arm (the model cannot fire): {answer_v}"
    );
    // The CONTRAST is the proof: the union armed a tool_call for this prompt; the
    // answer-only arm forced a settle on the identical prompt.
    assert!(
        union_v.get("tool_call").is_some(),
        "the union arm fires the tool on a tool-eliciting prompt (the contrast): {union_v}"
    );
}
