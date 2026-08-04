//! **P2** — what a model's tool-call markers actually detokenize to.
//!
//! The GBNF constrains CHARACTERS, but a model emits TOKENS. When llama.cpp arms a lazy
//! grammar it replays the trigger's capture group into the grammar and walks it
//! character by character; if the grammar cannot accept the walk it THROWS, and nothing
//! catches that before the C boundary — the process aborts mid-decode.
//!
//! So before trusting any dialect against a real model, read what its markers tokenize to
//! and what those tokens detokenize back into. A marker that renders as a different string
//! (or as nothing) makes its grammar branch unreachable, and the failure is fatal rather
//! than degraded.
//!
//! `KX_DIALECT_PROBE_GGUF=<path> cargo test -p kx-llamacpp --test dialect_vocab_probe -- --ignored --nocapture`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_llamacpp::{LlamaBackend, Model, ModelParams};

#[test]
#[ignore = "needs a real GGUF; set KX_DIALECT_PROBE_GGUF"]
fn report_marker_tokenization() {
    let path = std::env::var("KX_DIALECT_PROBE_GGUF")
        .expect("PRECONDITION: set KX_DIALECT_PROBE_GGUF to the model under test");
    let backend = LlamaBackend::new().expect("backend");
    let params = ModelParams::new().with_n_gpu_layers(0);
    let model = Model::load_with_params(&backend, &path, &params).expect("load");
    let vocab = model.vocab();

    let derived = model
        .chat_template(None)
        .as_deref()
        .and_then(kx_toolcall::derive_dialect_from_template);
    println!("derived dialect: {derived:?}");

    let mut markers: Vec<String> = vec!["{\"tool_call\"".to_string()];
    if let Some(d) = &derived {
        markers.push(d.open.to_string());
        if let Some(m) = &d.call_marker {
            markers.push(format!("{}{}", d.open, m));
        }
        if let Some(c) = &d.close {
            markers.push(c.to_string());
        }
    }

    for m in markers {
        for parse_special in [true, false] {
            match vocab.tokenize(&m, false, parse_special) {
                Ok(toks) => {
                    let pieces: Vec<String> = toks
                        .iter()
                        .map(|t| {
                            let p = vocab.token_to_piece(*t, 0, true).unwrap_or_default();
                            format!("{:?}(id={} eog={})", p, t.0, vocab.is_eog(*t))
                        })
                        .collect();
                    println!(
                        "PROBE {m:?} parse_special={parse_special} -> {} token(s): {}",
                        toks.len(),
                        pieces.join(" ")
                    );
                }
                Err(e) => println!("PROBE {m:?} parse_special={parse_special} -> ERROR {e:?}"),
            }
        }
    }

    // The grammar the runtime would actually arm for this model.
    let mut dialects = vec![kx_toolcall::CANONICAL_ENVELOPE];
    if let Some(d) = derived.clone() {
        dialects.push(d);
    }
    let spec = kx_grammar::ToolEnvelopeSpec::new(vec![
        kx_grammar::ToolSpec::new("mcp-calc/calc", "1"),
        kx_grammar::ToolSpec::new("mcp-kv/get", "1"),
    ]);
    println!("=== GRAMMAR ===");
    println!("{}", spec.to_gbnf_for(&dialects));
    println!("=== TRIGGERS ===");
    for t in kx_grammar::tool_call_trigger_patterns(&dialects) {
        println!("{t}");
    }
}

/// Walk a full, well-formed native call through the armed grammar one token at a time,
/// printing after each. If the grammar cannot accept a token, llama.cpp throws through the
/// C boundary and the PROCESS ABORTS — so the last line printed names the exact token that
/// killed it. That is the only way to locate this failure: it cannot be caught.
#[test]
#[ignore = "needs a real GGUF; set KX_DIALECT_PROBE_GGUF"]
fn walk_a_native_call_through_the_armed_grammar() {
    use kx_llamacpp::Sampler;
    let path = std::env::var("KX_DIALECT_PROBE_GGUF").expect("set KX_DIALECT_PROBE_GGUF");
    let backend = LlamaBackend::new().expect("backend");
    let params = ModelParams::new().with_n_gpu_layers(0);
    let model = Model::load_with_params(&backend, &path, &params).expect("load");
    let vocab = model.vocab();

    let mut dialects = vec![kx_toolcall::CANONICAL_ENVELOPE];
    if let Some(d) = model
        .chat_template(None)
        .as_deref()
        .and_then(kx_toolcall::derive_dialect_from_template)
    {
        dialects.push(d);
    }
    let spec = kx_grammar::ToolEnvelopeSpec::new(vec![
        kx_grammar::ToolSpec::new("mcp-calc/calc", "1"),
        kx_grammar::ToolSpec::new("mcp-kv/get", "1"),
    ]);
    let gbnf = spec.to_gbnf_for(&dialects);
    let triggers = kx_grammar::tool_call_trigger_patterns(&dialects);
    let refs: Vec<&str> = triggers.iter().map(String::as_str).collect();

    let emission = std::env::var("KX_WALK_TEXT").unwrap_or_else(|_| {
        "<|tool_call>call:mcp-calc/calc{\"a\": 6, \"b\": 7, \"op\": \"mul\"}<tool_call|>"
            .to_string()
    });
    println!("WALK emission = {emission:?}");

    let mut sampler = Sampler::chain(&backend)
        .add_grammar_lazy(&vocab, &gbnf, "root", &refs)
        .and_then(kx_llamacpp::SamplerChainBuilder::add_greedy)
        .and_then(kx_llamacpp::SamplerChainBuilder::build)
        .expect("build lazy sampler");

    let tokens = vocab.tokenize(&emission, false, true).expect("tokenize");
    for (i, t) in tokens.iter().enumerate() {
        let piece = String::from_utf8_lossy(&vocab.token_to_piece(*t, 0, true).unwrap_or_default())
            .to_string();
        println!("WALK step {i:>3} accepting id={} piece={piece:?}", t.0);
        sampler.accept(*t);
    }
    println!("WALK COMPLETE — the grammar accepted the whole native call");
}

/// Generate FOR REAL with the dialect grammar armed, printing every token as it is
/// produced. Goes through the production `Generator` — mask, sample, accept — so it shows
/// what the model is actually STEERED to emit, not what it would have emitted free.
#[test]
#[ignore = "needs a real GGUF; set KX_DIALECT_PROBE_GGUF"]
fn generate_under_the_armed_grammar() {
    use kx_llamacpp::{Context, Generator, Sampler};
    let path = std::env::var("KX_DIALECT_PROBE_GGUF").expect("set KX_DIALECT_PROBE_GGUF");
    let backend = LlamaBackend::new().expect("backend");
    let params = ModelParams::new().with_n_gpu_layers(999);
    let model = Model::load_with_params(&backend, &path, &params).expect("load");
    let vocab = model.vocab();

    let mut dialects = vec![kx_toolcall::CANONICAL_ENVELOPE];
    if let Some(d) = model
        .chat_template(None)
        .as_deref()
        .and_then(kx_toolcall::derive_dialect_from_template)
    {
        dialects.push(d);
    }
    let spec = kx_grammar::ToolEnvelopeSpec::new(vec![
        kx_grammar::ToolSpec::new("mcp-calc/calc", "1"),
        kx_grammar::ToolSpec::new("mcp-kv/get", "1"),
    ]);
    let gbnf = spec.to_gbnf_for(&dialects);
    let triggers = kx_grammar::tool_call_trigger_patterns(&dialects);
    let refs: Vec<&str> = triggers.iter().map(String::as_str).collect();

    let prompt = std::env::var("KX_GEN_PROMPT").unwrap_or_else(|_| {
        "You can call tools. Available: mcp-calc/calc (args: a, b, op where op is one of \
         add, div, mul, sub). What is 6 multiplied by 7? Use the calculator tool. Reply \
         with exactly one tool call and nothing else."
            .to_string()
    });

    let mut sampler = Sampler::chain(&backend)
        .add_grammar_lazy(&vocab, &gbnf, "root", &refs)
        .and_then(kx_llamacpp::SamplerChainBuilder::add_greedy)
        .and_then(kx_llamacpp::SamplerChainBuilder::build)
        .expect("build lazy sampler");

    let mut ctx = Context::new(&model).expect("ctx");
    let toks = vocab.tokenize(&prompt, true, true).expect("tokenize");
    let generator = Generator::new(&mut ctx, &mut sampler, &vocab, toks).expect("generator");

    let mut out = String::new();
    for (step, t) in generator.enumerate().take(96) {
        let tok = t.expect("token");
        let piece =
            String::from_utf8_lossy(&vocab.token_to_piece(tok, 0, true).unwrap_or_default())
                .to_string();
        let e = kx_llamacpp::grammar_engagement();
        println!(
            "GEN {step:>3} id={} piece={piece:?} engaged={} awaiting={}",
            tok.0, e.engaged, e.awaiting
        );
        out.push_str(&piece);
    }
    println!("GEN OUTPUT = {out:?}");
}
