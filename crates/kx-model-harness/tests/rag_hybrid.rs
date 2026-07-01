//! RC4c — the hybrid RAG glue (`query_corpus_hybrid`) + the LLM listwise rerank
//! (`rerank_hits`), end-to-end with SYNTHETIC vectors + a stub reranker (no model),
//! so this runs in the default `cargo test` pass. Proves: (1) the BM25 (sparse) leg
//! surfaces an exact-term match a TIED dense embedding would miss, and (2) the rerank
//! is FAIL-CLOSED — a valid permutation reorders, anything else keeps the input order.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use kx_content::ContentRef;
use kx_dataset::{Hit, InMemoryDataStore, InMemoryRetrievalIndex, LexicalIndex, RetrievalIndex};
use kx_dataset_bm25::Bm25Index;
use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams,
};
use kx_model_harness::{ingest_corpus_hybrid, query_corpus_hybrid, rerank_hits, Embedder};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

fn model() -> ModelId {
    ModelId("local".into())
}

fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model(),
            max_input_tokens: 4096,
            max_output_tokens: 4096,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// A deliberately WEAK embedder: every text maps to the SAME 1-dim vector `[1.0]`, so
/// the dense leg TIES all documents (cosine == 1 for every pair). Retrieval order is
/// then decided entirely by the BM25 (sparse) leg — exactly the "a weak decoder-LLM
/// embedding ties; the keyword leg saves it" case hybrid retrieval exists for.
struct TiedEmbed;

impl InferenceBackend for TiedEmbed {
    fn dispatch(
        &self,
        _model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        Err(InferenceError::Unsupported {
            reason: "stub: embeddings only",
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "tied-embed"
    }
}

impl EmbeddingBackend for TiedEmbed {
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        _text: &str,
        _pooling: EmbeddingPooling,
        _warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        Ok(EmbeddingOutput {
            vector: vec![1.0],
            dim: 1,
            backend_name: "tied-embed",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }
}

const FOX: &str = "the quick brown fox jumps";
const DOG: &str = "a lazy dog sleeps all day";
const PHYSICS: &str = "quantum entanglement in modern physics";

#[test]
fn hybrid_surfaces_a_keyword_match_a_dense_tie_misses() {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let mut lexical = Bm25Index::new();
    let backend = TiedEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);

    let docs = [FOX, DOG, PHYSICS];
    ingest_corpus_hybrid(&store, &mut index, &mut lexical, &embedder, &docs, &[]).unwrap();
    assert_eq!(index.len(), 3);
    assert_eq!(lexical.len(), 3);

    // Dense ties all three; only BM25 distinguishes "quantum physics" → the PHYSICS doc.
    let physics_ref = ContentRef::of(PHYSICS.as_bytes());
    let (_fact, hits) =
        query_corpus_hybrid(&index, &lexical, &embedder, "quantum physics", 1, true).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].id, physics_ref,
        "BM25 surfaces the exact-term doc the tied dense leg cannot rank"
    );
}

/// A stub generation backend that echoes a fixed `reply` as the completion — stands
/// in for a model emitting a (valid or invalid) rerank permutation.
struct StubReranker {
    reply: String,
}

impl InferenceBackend for StubReranker {
    fn dispatch(
        &self,
        _model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        Ok(InferenceOutput {
            bytes: self.reply.clone().into_bytes(),
            output_tokens: 0,
            backend_name: "stub-rerank",
            model_id: model(),
            elapsed: Duration::from_millis(0),
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "stub-rerank"
    }
}

fn hit(tag: u8) -> Hit {
    Hit {
        id: ContentRef([tag; 32]),
        score: f32::from(tag),
    }
}

#[test]
fn rerank_reorders_on_a_valid_permutation() {
    let hits = [hit(1), hit(2), hit(3)];
    let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let backend = StubReranker {
        reply: "[2,0,1]".to_string(),
    };
    let out = rerank_hits(&backend, &model(), &warrant(), "q", &hits, &texts);
    assert_eq!(
        out.iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![hits[2].id, hits[0].id, hits[1].id],
        "a valid permutation reorders the hits exactly"
    );
}

#[test]
fn rerank_fails_closed_to_input_order_on_garbage() {
    let hits = [hit(1), hit(2), hit(3)];
    let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let input_order: Vec<ContentRef> = hits.iter().map(|h| h.id).collect();
    for reply in [
        "not a permutation",
        "[0,0,1]",
        "[0,1,2,3]",
        "[0,1]",
        "[5,0,1]",
    ] {
        let backend = StubReranker {
            reply: reply.to_string(),
        };
        let out = rerank_hits(&backend, &model(), &warrant(), "q", &hits, &texts);
        assert_eq!(
            out.iter().map(|h| h.id).collect::<Vec<_>>(),
            input_order,
            "a non-permutation ({reply:?}) must keep the upstream order (fail-closed)"
        );
    }
}

#[test]
fn rerank_is_identity_for_trivial_or_mismatched_inputs() {
    let hits = [hit(1)];
    let texts = vec!["only".to_string()];
    let backend = StubReranker {
        reply: "[0]".to_string(),
    };
    // n < 2 ⇒ identity (never even dispatches).
    let out = rerank_hits(&backend, &model(), &warrant(), "q", &hits, &texts);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, hits[0].id);

    // mismatched texts/hits ⇒ identity.
    let two = [hit(1), hit(2)];
    let one_text = vec!["x".to_string()];
    let out = rerank_hits(&backend, &model(), &warrant(), "q", &two, &one_text);
    assert_eq!(
        out.iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![two[0].id, two[1].id]
    );
}
