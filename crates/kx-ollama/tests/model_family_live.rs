//! LIVE (`#[ignore]`) proof that the MODEL-FAMILY REGISTRY reaches a real daemon.
//!
//! The unit tests prove the table renders the right bytes for a family string. They
//! cannot prove the family string is the one the daemon actually reports, and that is
//! the half the incident lived in: `gemma4:12b` reports `details.family = "gemma4"`,
//! which the old hard-coded arm did not claim, so every Gemma-4 turn was dispatched as
//! `ChatML` — a vocabulary Gemma has never been trained on.
//!
//! ⚠ It also proves the CONSTRUCTOR matters. [`OllamaBackend::new`] leaves the family map
//! empty, so it renders no template for any model; only [`OllamaBackend::discover`]
//! populates it from `/api/show`. A test built on `new` would asserts its way to green
//! against a backend that could never have templated anything.
//!
//! Requires a running Ollama daemon serving `gemma4:12b`. Opt in:
//!   KX_SERVE_OLLAMA=on cargo test -p kx-ollama --test model_family_live -- --ignored --nocapture
//! NOT in CI (no live model); skips cleanly when not opted in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::field_reassign_with_default
)]

use std::sync::Arc;

use kx_inference::{InferenceBackend, InferenceInput, InferenceParams};
use kx_mote::ModelId;
use kx_ollama::{OllamaBackend, OllamaClient};
use kx_warrant::WarrantSpec;

const MODEL: &str = "gemma4:12b";

/// Truthy `KX_SERVE_OLLAMA` — the operator opt-in the live serve tests share.
fn opted_in() -> bool {
    std::env::var("KX_SERVE_OLLAMA")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(false)
}

fn backend() -> OllamaBackend {
    let client = Arc::new(OllamaClient::new("http://127.0.0.1:11434", false).unwrap());
    // `discover` — NOT `new`. See the module note: `new` leaves the family map empty.
    OllamaBackend::discover(client, None).expect("live daemon /api/tags")
}

fn warrant() -> WarrantSpec {
    let mut w = WarrantSpec::default();
    w.model_route.model_id = ModelId(MODEL.to_string());
    w.model_route.max_output_tokens = 96;
    w.resource_ceiling.wall_clock_ms = 120_000;
    w
}

fn dispatch(b: &OllamaBackend, prompt: &str) -> String {
    let mut p = InferenceParams::default();
    p.max_output_tokens = 96;
    p.temperature_bps = 0;
    let out = b
        .dispatch(
            &ModelId(MODEL.to_string()),
            &InferenceInput::text(prompt),
            &p,
            &warrant(),
        )
        .expect("live gemma4 dispatch");
    String::from_utf8_lossy(&out.bytes).into_owned()
}

/// The daemon must report `gemma4`, and the registry must render Gemma-4's own
/// vocabulary for it — not `ChatML`, and not Gemma-3's.
#[test]
#[ignore = "live Ollama gemma4:12b; opt in with KX_SERVE_OLLAMA=on --ignored"]
fn the_daemons_declared_family_resolves_to_the_gemma4_template_live() {
    if !opted_in() {
        eprintln!("skipping: set KX_SERVE_OLLAMA=on (needs a running Ollama daemon + gemma4:12b)");
        return;
    }
    let b = backend();

    // PRECONDITION, asserted rather than skipped: the model must be served here, or the
    // rest of this test is true of a backend that has never heard of it.
    assert!(
        b.supports(&ModelId(MODEL.to_string())),
        "{MODEL} is not served by the daemon — pull it before running this proof"
    );

    let rendered = b
        .render_chat(&ModelId(MODEL.to_string()), "SYS", "USR")
        .expect("gemma4 must resolve to a template — `None` here IS the incident (ChatML)");

    assert_eq!(
        rendered,
        "<|turn>system\nSYS<turn|>\n<|turn>user\nUSR<turn|>\n\
         <|turn>model\n<|channel>thought\n<channel|>",
        "the live render must match the registry's measured Gemma-4 template"
    );
    for foreign in [
        "<|im_start|>",
        "<|im_end|>",
        "<start_of_turn>",
        "<end_of_turn>",
    ] {
        assert!(
            !rendered.contains(foreign),
            "a live gemma4 render carried {foreign:?}"
        );
    }
}

/// ★ THE BEHAVIOURAL HALF, with its own negative control.
///
/// Under the correct template the answer is clean. Under the `ChatML` the old code sent,
/// the SAME model on the SAME question emits its thought-channel markers as CONTENT, and
/// they reach the caller as answer text. One variable: the rendered prompt.
///
/// This is what makes the registry a capability rather than a refactor — and it is the
/// mechanism behind the standing "the Gemma `<|channel>thought` preamble reaches scored
/// answers" debt, fixed at the source instead of stripped downstream.
#[test]
#[ignore = "live Ollama gemma4:12b; opt in with KX_SERVE_OLLAMA=on --ignored"]
fn the_correct_template_suppresses_the_thought_preamble_live() {
    if !opted_in() {
        eprintln!("skipping: set KX_SERVE_OLLAMA=on (needs a running Ollama daemon + gemma4:12b)");
        return;
    }
    let b = backend();
    let sys = "You are terse. Answer in one short sentence.";
    let user = "What is the capital of France?";

    let correct = b
        .render_chat(&ModelId(MODEL.to_string()), sys, user)
        .expect("gemma4 resolves");
    // The negative control: what this model received before the registry existed.
    let chatml = format!(
        "<|im_start|>system\n{sys}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n\
         <|im_start|>assistant\n"
    );

    let good = dispatch(&b, &correct);
    let bad = dispatch(&b, &chatml);

    eprintln!("  correct template -> {good:?}");
    eprintln!("  chatml (control) -> {bad:?}");

    assert!(
        !good.contains("<|channel>"),
        "the correct template must not leak the thought channel into the answer: {good:?}"
    );
    // The control must FAIL the same assertion, or the assertion above is vacuous —
    // it would pass on a model that never emits the marker under any prompt.
    assert!(
        bad.contains("<|channel>"),
        "the ChatML control was expected to leak the preamble; if it no longer does, this \
         contrast has stopped measuring anything and the claim must be re-derived: {bad:?}"
    );
    // Both arms must still answer — the defect is contamination, not incapacity.
    for (label, text) in [("correct", &good), ("control", &bad)] {
        assert!(
            text.to_lowercase().contains("paris"),
            "{label} arm did not answer the question: {text:?}"
        );
    }
}
