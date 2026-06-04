// Integration test: see the note in `tests/smoke.rs` on the lint allowances.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! End-to-end smoke test for the IMAGE multi-modal pipeline through the safe
//! wrapper (PR-2):
//!
//! ```text
//! Model::load → Mtmd::from_file (projector) → Bitmap::from_image_buf (decode)
//!   → Mtmd::tokenize (text + image chunks) → Mtmd::eval_chunks (prefill)
//!   → Generator::from_prefilled → sample/detokenize loop
//! ```
//!
//! Gated on `model-smoke-test-multimodal` so the default `cargo test` runs
//! without a network dependency. With the feature, `build.rs` downloads
//! Qwen2-VL-2B-Instruct (GGUF + mmproj, ~1.6 GB) and emits
//! `KX_LLAMACPP_SMOKE_VLM_GGUF` / `KX_LLAMACPP_SMOKE_VLM_MMPROJ`. The test image
//! is a committed 96×96 PNG (a red square on white) — small, deterministic,
//! describable.

#![cfg(feature = "model-smoke-test-multimodal")]

use kx_llamacpp::{
    Bitmap, Context, ContextParams, Generator, LlamaBackend, LlamaError, Model, Mtmd, Sampler,
};

const VLM_GGUF: &str = env!(
    "KX_LLAMACPP_SMOKE_VLM_GGUF",
    "build.rs sets this under model-smoke-test-multimodal"
);
const VLM_MMPROJ: &str = env!(
    "KX_LLAMACPP_SMOKE_VLM_MMPROJ",
    "build.rs sets this under model-smoke-test-multimodal"
);

/// A committed 96×96 PNG: a red square on a white background.
const RED_SQUARE_PNG: &[u8] = include_bytes!("fixtures/red_square.png");

const N_CTX: u32 = 4096;
const N_BATCH: u32 = 2048;
const N_UBATCH: u32 = 512;

/// Build a Qwen2-VL ChatML user turn with one media marker (the projector
/// replaces it with the encoded image tokens during `tokenize`).
fn prompt_with_image(question: &str) -> String {
    let marker = Mtmd::default_marker();
    format!(
        "<|im_start|>system\nYou are a helpful assistant.<|im_end|>\n\
         <|im_start|>user\n{marker}{question}<|im_end|>\n\
         <|im_start|>assistant\n"
    )
}

/// Run the full image→text pipeline once and return the generated text.
fn describe_image(question: &str, image: &[u8], max_tokens: usize) -> String {
    let backend = LlamaBackend::new().expect("backend init");
    let model = Model::load(&backend, VLM_GGUF).expect("load VLM gguf");

    let mtmd = Mtmd::from_file(&model, VLM_MMPROJ, 0, true).expect("load projector");
    assert!(
        mtmd.supports_vision(),
        "the Qwen2-VL projector must support vision"
    );

    let bitmap = Bitmap::from_image_buf(&mtmd, image).expect("decode the test PNG");
    let chunks = mtmd
        .tokenize(&prompt_with_image(question), &[&bitmap])
        .expect("tokenize text + image into chunks");
    assert!(!chunks.is_empty(), "tokenize produced no chunks");

    let mut ctx = Context::new_with_params(
        &model,
        &ContextParams::new()
            .with_n_ctx(N_CTX)
            .with_n_batch(N_BATCH)
            .with_n_ubatch(N_UBATCH)
            .with_n_seq_max(1),
    )
    .expect("context");

    let n_batch = ctx.n_batch() as i32;
    let n_past = mtmd
        .eval_chunks(&mut ctx, &chunks, 0, 0, n_batch, true)
        .expect("multi-modal prefill");
    assert!(n_past > 0, "prefill must advance n_past past zero");

    let vocab = model.vocab();
    let mut sampler = Sampler::greedy(&backend).expect("greedy");
    let gen = Generator::from_prefilled(&mut ctx, &mut sampler, &vocab, n_past);

    let mut bytes: Vec<u8> = Vec::new();
    for tok in gen.take(max_tokens) {
        let tok = tok.expect("token");
        if tok.is_eog(&vocab) {
            break;
        }
        vocab
            .token_to_piece_into(tok, 0, false, &mut bytes)
            .expect("detokenize");
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The headline: a real VLM describes the test image, image→text, non-empty.
#[test]
fn smoke_image_to_text_describe() {
    let answer = describe_image("What color is the shape in this image?", RED_SQUARE_PNG, 24);
    eprintln!("VLM answer: {answer:?}");
    assert!(
        !answer.trim().is_empty(),
        "the image→text pipeline must produce non-empty output"
    );
    // Soft signal (not a hard assert — model quality is not what this gates):
    // a correct Qwen2-VL answer mentions "red".
    if answer.to_lowercase().contains("red") {
        eprintln!("(model correctly identified the red square)");
    } else {
        eprintln!("(note: model did not say 'red'; smoke only requires non-empty)");
    }
}

/// Greedy image→text must be deterministic across independent runs — the
/// wrapper-level guarantee the runtime's capture/replay relies on.
#[test]
fn smoke_image_to_text_deterministic() {
    let q = "Describe this image in one short sentence.";
    let a = describe_image(q, RED_SQUARE_PNG, 16);
    let b = describe_image(q, RED_SQUARE_PNG, 16);
    eprintln!("run A: {a:?}\nrun B: {b:?}");
    assert_eq!(
        a, b,
        "greedy image→text must be byte-identical across runs (same image + model)"
    );
}

/// Fail-closed on undecodable image bytes: a truncated PNG yields a typed
/// `BitmapDecodeFailed`, never a panic / UB. The boundary for untrusted media.
#[test]
fn smoke_corrupt_image_fails_closed() {
    let backend = LlamaBackend::new().expect("backend init");
    let model = Model::load(&backend, VLM_GGUF).expect("load");
    let mtmd = Mtmd::from_file(&model, VLM_MMPROJ, 0, true).expect("load projector");

    // PNG magic followed by garbage — stb cannot decode this.
    let corrupt = [
        0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0x00, 0x13, 0x37,
    ];
    match Bitmap::from_image_buf(&mtmd, &corrupt) {
        Err(LlamaError::BitmapDecodeFailed { .. }) => {}
        Err(other) => panic!("expected BitmapDecodeFailed, got {other}"),
        Ok(_) => panic!("a corrupt image must not decode"),
    }
}

/// The image projector reports vision support and (for Qwen2-VL) not audio —
/// the capability introspection the dispatch gate relies on.
#[test]
fn smoke_projector_capabilities() {
    let backend = LlamaBackend::new().expect("backend init");
    let model = Model::load(&backend, VLM_GGUF).expect("load");
    let mtmd = Mtmd::from_file(&model, VLM_MMPROJ, 0, true).expect("load projector");
    assert!(mtmd.supports_vision(), "Qwen2-VL projector supports vision");
    assert!(
        !mtmd.supports_audio(),
        "an image projector must not claim audio support"
    );
}
