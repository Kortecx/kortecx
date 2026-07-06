//! `RC4c` — the LLM listwise rerank (`rerank_hits`) driven by a REAL model on BOTH
//! inference engines (GR15 real-model integrity + GR24 dual-engine parity).
//!
//! Gated `#[ignore]` (needs a served model — never runs in the default `cargo test`).
//! Opt in per engine (free the prior model first — Metal can't hold both 12B models):
//!
//! ```sh
//! # llama.cpp (Gemma-4 GGUF):
//! cargo test -p kx-model-harness --test rerank_live --features with-model \
//!   -- --ignored --nocapture rerank_live_llamacpp
//! # Ollama (gemma3:12b daemon on :11434):
//! cargo test -p kx-model-harness --test rerank_live -- --ignored --nocapture rerank_live_ollama
//! ```
//!
//! The scenario: a clearly on-topic passage is placed LAST among distractors, and the
//! model must rerank it to the TOP — a no-op (or a garbage permutation that fail-closes
//! to the input order) leaves it last and FAILS, so this proves the model actually
//! reordered. The grammar carrier constrains the output to a permutation on both
//! engines (a GBNF unroll on llama.cpp, a strict whole-response `format` on Ollama).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_content::ContentRef;
use kx_dataset::Hit;
use kx_inference::InferenceBackend;
use kx_model_harness::{harness_warrant, rerank_hits};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

/// A query whose answer is UNAMBIGUOUSLY one of the passages, with the on-topic
/// passage placed LAST (index 2) so a non-reordering result fails.
const QUERY: &str = "How do plants make energy from sunlight?";
const PASSAGES: [&str; 3] = [
    "The stock market closed higher today on strong technology earnings.",
    "Bananas are a popular fruit and a good source of dietary potassium.",
    "Photosynthesis is how plants turn sunlight, water, and carbon dioxide into sugar \
     and oxygen inside their leaves.",
];
/// The index (in `PASSAGES`) of the on-topic passage the model must promote to top.
const RELEVANT_IDX: usize = 2;

fn drive(backend: &dyn InferenceBackend, model_id: &ModelId, warrant: &WarrantSpec) {
    // Distinct refs per passage; `hits[i]` corresponds to `PASSAGES[i]`.
    let hits: Vec<Hit> = (0..PASSAGES.len())
        .map(|i| Hit {
            id: ContentRef([u8::try_from(i).unwrap() + 1; 32]),
            score: 0.0,
        })
        .collect();
    let texts: Vec<String> = PASSAGES.iter().map(|s| (*s).to_string()).collect();

    let reranked = rerank_hits(backend, model_id, warrant, QUERY, &hits, &texts);

    assert_eq!(
        reranked.len(),
        hits.len(),
        "rerank preserves the candidate set"
    );
    // The on-topic passage must be reranked to the TOP (it started LAST).
    assert_eq!(
        reranked[0].id,
        hits[RELEVANT_IDX].id,
        "the model must rerank the on-topic passage to position 0 (it began last); \
         got order {:?}",
        reranked
            .iter()
            .map(|h| h.id.as_bytes()[0])
            .collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "real llama.cpp Gemma model; opt in with --features with-model --ignored"]
#[cfg(feature = "with-model")]
fn rerank_live_llamacpp() {
    use kx_inference::LlamaInferenceBackend;
    use kx_model_harness::model_id_for;
    use std::path::PathBuf;

    let gguf = PathBuf::from(
        std::env::var("KX_RERANK_GGUF")
            .unwrap_or_else(|_| "target/models/gemma-4-12b-it-q4_k_m.gguf".to_string()),
    );
    assert!(
        gguf.exists(),
        "Gemma GGUF not found at {gguf:?} (run `just fetch-gemma-model`)"
    );
    let model_id = model_id_for(&gguf).unwrap();
    let warrant = harness_warrant(&model_id, 512, 120_000);
    let backend = LlamaInferenceBackend::with_model(model_id.clone(), gguf);
    drive(&backend, &model_id, &warrant);
}

#[test]
#[ignore = "real Ollama gemma3 daemon on :11434; opt in with --ignored"]
fn rerank_live_ollama() {
    use kx_ollama::{OllamaBackend, OllamaClient};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    let model =
        std::env::var("KX_RERANK_OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:12b".to_string());
    let base =
        std::env::var("KX_OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    let client = Arc::new(OllamaClient::new(&base, false).expect("ollama client"));
    let backend = OllamaBackend::new(client, BTreeSet::from([model.clone()]));
    let model_id = ModelId(model);
    let warrant = harness_warrant(&model_id, 512, 120_000);
    drive(&backend, &model_id, &warrant);
}
