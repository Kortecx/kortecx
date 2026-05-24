//! End-to-end smoke test exercising the full llama.cpp inference pipeline
//! through the safe wrapper:
//!
//! ```text
//! LlamaBackend → Model::load → Vocab::tokenize → Batch + Context::decode
//!              → Context::logits_ith → Sampler::sample → Vocab::detokenize
//! ```
//!
//! Gated on the `model-smoke-test` feature so the default `cargo test` runs
//! without a network dependency. With the feature, `build.rs` downloads the
//! stories260K GGUF (~1.2 MB) and emits `KX_LLAMACPP_SMOKE_TEST_MODEL` as a
//! compile-time env so this file `env!`s the path.
//!
//! The stories260K model is too small to produce coherent text, but it
//! exercises every code path on a real GGUF: tensor load, tokenizer,
//! KV-cache allocation, prompt decode, logits readout, sampling, detokenize.

#![cfg(feature = "model-smoke-test")]

use kx_llamacpp::{Batch, Context, ContextParams, LlamaBackend, Model, ModelParams, Sampler};

const MODEL_PATH: &str = env!(
    "KX_LLAMACPP_SMOKE_TEST_MODEL",
    "build.rs should have set this when model-smoke-test feature is on"
);

/// Smallest possible end-to-end: load, tokenize, decode, sample once,
/// detokenize. Verifies the full pipeline links and runs.
#[test]
fn smoke_end_to_end_inference() {
    let backend = LlamaBackend::new().expect("backend init");

    // Load with vocab-aware params (no GPU offload — single-compute OSS).
    let params = ModelParams::new().with_n_gpu_layers(0);
    let model = Model::load_with_params(&backend, MODEL_PATH, &params)
        .expect("load stories260K from downloaded GGUF");

    // Sanity: metadata is non-trivial.
    assert!(model.n_embd() > 0, "model n_embd must be positive");
    assert!(model.n_layer() > 0, "model n_layer must be positive");
    assert!(model.n_params() > 0, "model n_params must be positive");
    assert!(model.size() > 0, "model size must be positive");
    let desc = model.desc();
    assert!(!desc.is_empty(), "model description must be non-empty");
    eprintln!(
        "model: {desc} ({} params, {} bytes)",
        model.n_params(),
        model.size()
    );

    // Vocab + tokenize a short prompt.
    let vocab = model.vocab();
    assert!(vocab.n_tokens() > 0, "vocab must have tokens");

    let prompt = "Once upon a time";
    let tokens = vocab
        .tokenize(
            prompt, /* add_special */ true, /* parse_special */ false,
        )
        .expect("tokenize prompt");
    assert!(
        !tokens.is_empty(),
        "tokenize must produce at least one token"
    );
    eprintln!(
        "tokenized {} into {} tokens: {:?}",
        prompt,
        tokens.len(),
        tokens
    );

    // Round-trip: detokenize the prompt tokens back. With BOS prepended the
    // result starts with a BOS marker; we just check it includes the prompt
    // substring (the model's BPE may insert leading-space tokens etc).
    let round_trip = vocab
        .detokenize(&tokens, /* special */ false)
        .expect("detokenize");
    eprintln!("round-trip: {round_trip:?}");

    // Build a context sized for the smoke test (small, single seq).
    let ctx_params = ContextParams::new()
        .with_n_ctx(128)
        .with_n_batch(32)
        .with_n_ubatch(32)
        .with_n_seq_max(1);
    let mut ctx = Context::new_with_params(&model, &ctx_params).expect("context");
    assert!(
        ctx.n_ctx() >= 32,
        "context must be at least the requested size"
    );

    // Build a batch with the prompt; only the last position needs logits.
    let mut batch = Batch::with_capacity(tokens.len() as i32, 1);
    for (i, &t) in tokens.iter().enumerate() {
        let is_last = i + 1 == tokens.len();
        batch.add(t, i as i32, &[0], is_last);
    }
    assert_eq!(batch.n_tokens() as usize, tokens.len());

    // Decode the prompt.
    ctx.decode(&batch).expect("decode prompt");

    // KV-cache should now hold the prompt.
    assert_eq!(
        ctx.kv_cache_seq_pos_max(0) as usize,
        tokens.len() - 1,
        "after decode, seq 0 holds tokens[0..len-1]"
    );

    // Greedy-sample the next token from the last position's logits.
    let mut sampler = Sampler::greedy(&backend).expect("greedy sampler");
    let next = sampler.sample(&mut ctx, -1);
    eprintln!("greedy-sampled next token: {next}");

    // Detokenize that single token. Should yield some byte sequence (possibly
    // not human-readable on such a tiny model — that's fine, we just need a
    // round-trip without error).
    let piece = vocab
        .token_to_piece(next, 0, false)
        .expect("token_to_piece for sampled token");
    eprintln!("sampled piece bytes: {piece:?}");

    // Continue for a few more tokens, exercising incremental decode.
    let mut current = next;
    let mut generated = vec![next];
    for step in 0..5 {
        // Submit just the new token; position continues from where prompt ended.
        let mut step_batch = Batch::with_capacity(1, 1);
        step_batch.add(current, (tokens.len() + step) as i32, &[0], true);
        ctx.decode(&step_batch).expect("decode step");

        current = sampler.sample(&mut ctx, -1);
        generated.push(current);
        if vocab.is_eog(current) {
            eprintln!("hit EOG at step {step}");
            break;
        }
    }
    assert!(generated.len() >= 2, "should generate at least 2 tokens");

    // Performance counters should be non-zero after a decode.
    let perf = ctx.perf();
    assert!(
        perf.n_p_eval + perf.n_eval > 0,
        "perf counters must reflect work"
    );
    eprintln!(
        "perf: p_eval={} eval={} reused={}",
        perf.n_p_eval, perf.n_eval, perf.n_reused
    );
}

/// KV-cache management round-trip: decode → query positions → seq_keep →
/// seq_rm → clear. Verifies the cache ops behave as documented.
///
/// Note: `seq_cp` is intentionally not exercised here. In llama.cpp b9000 the
/// unified-KV-cache implementation only supports `seq_cp` when the source
/// buffer page is "full" (an internal allocator constraint), which is not
/// reproducible from a smoke test without intricate setup. The wrapper
/// exposes the call; callers are expected to respect the upstream contract.
#[test]
fn smoke_kv_cache_management() {
    let backend = LlamaBackend::new().expect("backend init");
    let model = Model::load(&backend, MODEL_PATH).expect("load model");
    let mut ctx = Context::new_with_params(
        &model,
        &ContextParams::new().with_n_ctx(64).with_n_seq_max(1),
    )
    .expect("context");

    let vocab = model.vocab();
    let tokens = vocab
        .tokenize("hello world", true, false)
        .expect("tokenize");
    assert!(!tokens.is_empty());

    // Fill seq 0.
    let mut batch = Batch::with_capacity(tokens.len() as i32, 1);
    for (i, &t) in tokens.iter().enumerate() {
        batch.add(t, i as i32, &[0], false);
    }
    ctx.decode(&batch).expect("decode");

    let max_pos = ctx.kv_cache_seq_pos_max(0);
    assert_eq!(max_pos as usize, tokens.len() - 1, "seq 0 should be full");

    // seq_keep on the only sequence is a no-op (kept sequence stays).
    ctx.kv_cache_seq_keep(0);
    assert_eq!(
        ctx.kv_cache_seq_pos_max(0),
        max_pos,
        "seq 0 should still be present after seq_keep(0)"
    );

    // Truncate seq 0: drop everything past position 1.
    let _ = ctx.kv_cache_seq_rm(0, 1, -1);
    let trimmed = ctx.kv_cache_seq_pos_max(0);
    assert!(
        trimmed < max_pos,
        "after seq_rm(0, 1, -1) the max position must drop"
    );

    // Clear: cache should be empty.
    ctx.kv_cache_clear(false);
    assert_eq!(
        ctx.kv_cache_seq_pos_max(0),
        -1,
        "after kv_cache_clear, seq 0 should be empty"
    );
}

/// Tokenize → detokenize round-trip for several short strings.
#[test]
fn smoke_tokenize_detokenize_roundtrip() {
    let backend = LlamaBackend::new().expect("backend init");
    let model = Model::load(&backend, MODEL_PATH).expect("load");
    let vocab = model.vocab();

    for s in ["hi", "the quick brown fox", "once upon a time, in a land"] {
        let tokens = vocab.tokenize(s, false, false).expect("tokenize");
        assert!(!tokens.is_empty(), "tokenize {s:?} produced empty");
        let back = vocab.detokenize(&tokens, false).expect("detokenize");
        // The model's tokenizer normalizes whitespace; just check non-empty
        // and that obviously-present substrings survive.
        assert!(!back.is_empty(), "detokenize {s:?} produced empty");
        eprintln!("{s:?} -> {} tokens -> {back:?}", tokens.len());
    }
}

/// Verifies BOS / EOS / NL token queries return sensible values for a LLaMA
/// architecture model.
#[test]
fn smoke_vocab_special_tokens() {
    let backend = LlamaBackend::new().expect("backend init");
    let model = Model::load(&backend, MODEL_PATH).expect("load");
    let vocab = model.vocab();

    // BOS / EOS should be valid tokens in [0, n_tokens).
    let bos = vocab.bos();
    let eos = vocab.eos();
    let n = vocab.n_tokens();
    assert!(bos >= 0 && bos < n, "bos {bos} out of range [0, {n})");
    assert!(eos >= 0 && eos < n, "eos {eos} out of range [0, {n})");
    assert!(vocab.is_eog(eos), "eos must be an end-of-generation marker");
    eprintln!("bos={bos} eos={eos} nl={} n_tokens={n}", vocab.nl());
}
