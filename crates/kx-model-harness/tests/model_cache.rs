//! Loaded-model cache validation against the real GGUF (M4, PR-1).
//!
//! Needs the pinned model; run with `--features with-model` (the
//! `smoke-test-with-model` gate). These are the assertions that prove the
//! 16GB-reload-per-dispatch bug is gone and that the cache is *transparent*:
//!
//! - A cache HIT is byte-identical to the cold load (greedy decode).
//! - The second dispatch performs NO second load (`loads_performed` stays 1) —
//!   the deterministic, non-flaky proof (vs. timing) that the model is reused.
//! - Four concurrent dispatches against one backend share one cached model and
//!   serialize through the owner thread (exactly one cold load total).

#![cfg(feature = "with-model")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::thread;

use kx_inference::{InferenceBackend, InferenceInput, InferenceParams, LlamaInferenceBackend};
use kx_model_harness::{default_gguf_path, harness_warrant, model_id_for, ROW_PROMPT};

/// Greedy, short — deterministic and fast.
fn greedy() -> InferenceParams {
    InferenceParams {
        max_output_tokens: 16,
        temperature_bps: 0,
        ..Default::default()
    }
}

#[test]
fn cache_hit_is_byte_identical_and_does_not_reload() {
    let gguf = default_gguf_path();
    let model_id = model_id_for(&gguf).unwrap();
    let warrant = harness_warrant(&model_id, 16, 60_000);
    let backend = LlamaInferenceBackend::with_model(model_id.clone(), gguf);
    let input = InferenceInput::Text(ROW_PROMPT.to_string());
    let params = greedy();

    let out1 = backend
        .dispatch(&model_id, &input, &params, &warrant)
        .unwrap();
    assert_eq!(
        backend.loads_performed(),
        1,
        "first dispatch performs exactly one cold load"
    );

    let out2 = backend
        .dispatch(&model_id, &input, &params, &warrant)
        .unwrap();
    assert_eq!(
        backend.loads_performed(),
        1,
        "second dispatch is a cache HIT — the per-dispatch reload is gone"
    );

    assert!(!out1.bytes.is_empty(), "model produced output");
    assert_eq!(
        out1.bytes, out2.bytes,
        "greedy ⇒ a cache hit yields byte-identical output to the cold load"
    );
    assert_eq!(out1.output_tokens, out2.output_tokens);
}

#[test]
fn concurrent_dispatch_shares_one_cached_model() {
    let gguf = default_gguf_path();
    let model_id = model_id_for(&gguf).unwrap();
    let backend: Arc<LlamaInferenceBackend> =
        Arc::new(LlamaInferenceBackend::with_model(model_id.clone(), gguf));

    let mut handles = Vec::with_capacity(4);
    for _ in 0..4 {
        let b = Arc::clone(&backend);
        let id = model_id.clone();
        handles.push(thread::spawn(move || {
            let warrant = harness_warrant(&id, 16, 60_000);
            let out = b
                .dispatch(
                    &id,
                    &InferenceInput::Text(ROW_PROMPT.to_string()),
                    &greedy(),
                    &warrant,
                )
                .unwrap();
            assert!(!out.bytes.is_empty());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // All four threads serialized through the owner thread and shared one
    // cached model: exactly one cold load total.
    assert_eq!(
        backend.loads_performed(),
        1,
        "four concurrent dispatches load the model exactly once"
    );
}
