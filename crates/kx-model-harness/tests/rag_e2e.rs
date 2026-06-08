//! DP2 — the RAG ingest/query glue, end-to-end with SYNTHETIC vectors (no model).
//!
//! A deterministic keyword embedder makes retrieval predictable + assertable
//! without a GGUF, so this runs in the default `cargo test` pass. It proves the
//! whole `ingest_corpus` → `query_corpus` path: relevance ranking, the SN-8
//! scores-excluded fact, `DatasetId` reproducibility, content-addressed dedup,
//! and the empty/oversize-k edges. Real-model embedding numerics are covered by
//! the `kx-llamacpp` smoke (`embed_with`); a model-gated end-to-end belongs to a
//! `with-model` test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use kx_content::ContentRef;
use kx_dataset::{InMemoryDataStore, InMemoryRetrievalIndex, RetrievalIndex};
use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams,
};
use kx_model_harness::{encode_vector_le, ingest_corpus, query_corpus, Embedder};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use kx_workflow::retrieval_result_ref;

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
            max_input_tokens: 2048,
            max_output_tokens: 2048,
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

/// A deterministic keyword embedder: a 3-dim one-hot over {apple, banana, cherry}
/// by substring presence, so a query about apples retrieves the apple document.
struct KeywordEmbed;

impl InferenceBackend for KeywordEmbed {
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
        "keyword-embed"
    }
}

impl EmbeddingBackend for KeywordEmbed {
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        _pooling: EmbeddingPooling,
        _warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        let l = text.to_lowercase();
        let vector = vec![
            f32::from(u8::from(l.contains("apple"))),
            f32::from(u8::from(l.contains("banana"))),
            f32::from(u8::from(l.contains("cherry"))),
        ];
        Ok(EmbeddingOutput {
            vector,
            dim: 3,
            backend_name: "keyword-embed",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }
}

const APPLE: &str = "I baked an apple pie";
const BANANA: &str = "a banana split for dessert";
const CHERRY: &str = "fresh cherry tart";

#[test]
fn ingest_then_query_retrieves_the_relevant_doc() {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let backend = KeywordEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);

    let docs = [APPLE, BANANA, CHERRY];
    let ds = ingest_corpus(&store, &mut index, &embedder, &docs, &[]).unwrap();
    // 3 docs × (text row + vector row) = 6 rows; index holds one vector per doc.
    assert_eq!(ds.len(), 6, "two rows per document");
    assert_eq!(index.len(), 3);

    let (_fact, hits) =
        query_corpus(&index, &embedder, "what apple recipe should I try", 1).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].id,
        ContentRef::of(APPLE.as_bytes()),
        "the apple document is the nearest neighbour"
    );
    assert!(hits[0].score > 0.99, "cosine ~1 for the matching one-hot");
}

#[test]
fn result_fact_excludes_scores_and_is_stable() {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let backend = KeywordEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);
    ingest_corpus(&store, &mut index, &embedder, &[APPLE, BANANA, CHERRY], &[]).unwrap();

    let (fact_a, hits_a) = query_corpus(&index, &embedder, "apple", 2).unwrap();
    let (fact_b, _hits_b) = query_corpus(&index, &embedder, "apple", 2).unwrap();
    assert_eq!(fact_a, fact_b, "the same query yields the same exact fact");
    // The fact is purely the ordered refs — scores excluded (SN-8).
    assert_eq!(
        fact_a,
        retrieval_result_ref(&hits_a),
        "fact = retrieval_result_ref(hits) = ordered refs, no scores"
    );
}

#[test]
fn dataset_id_is_reproducible_across_two_ingests() {
    let docs = [APPLE, BANANA, CHERRY];
    let backend = KeywordEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);

    let id1 = {
        let store = InMemoryDataStore::new();
        let mut index = InMemoryRetrievalIndex::new();
        ingest_corpus(&store, &mut index, &embedder, &docs, &[])
            .unwrap()
            .id()
    };
    let id2 = {
        let store = InMemoryDataStore::new();
        let mut index = InMemoryRetrievalIndex::new();
        ingest_corpus(&store, &mut index, &embedder, &docs, &[])
            .unwrap()
            .id()
    };
    assert_eq!(id1, id2, "same corpus + model + pooling → same DatasetId");
}

#[test]
fn duplicate_docs_dedup_in_the_index() {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let backend = KeywordEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);
    let ds = ingest_corpus(&store, &mut index, &embedder, &[APPLE, APPLE, BANANA], &[]).unwrap();
    // The Dataset records every ingest row (6), but the content-addressed index
    // holds ONE entry per DISTINCT document.
    assert_eq!(ds.len(), 6);
    assert_eq!(index.len(), 2, "content-addressed dedup → 2 distinct docs");
}

#[test]
fn k_larger_than_corpus_truncates_to_available() {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let backend = KeywordEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);
    ingest_corpus(&store, &mut index, &embedder, &[APPLE, BANANA], &[]).unwrap();
    let (_fact, hits) = query_corpus(&index, &embedder, "apple", 10).unwrap();
    assert_eq!(hits.len(), 2, "k is clamped to the corpus size");
}

#[test]
fn empty_corpus_query_returns_no_hits() {
    let index = InMemoryRetrievalIndex::new();
    let backend = KeywordEmbed;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);
    let (fact, hits) = query_corpus(&index, &embedder, "apple", 5).unwrap();
    assert!(hits.is_empty());
    assert_eq!(
        fact,
        retrieval_result_ref(&[]),
        "empty fact is well-defined"
    );
}

#[test]
fn vector_row_uses_canonical_le_encoding() {
    // The stored Vector row's bytes are exactly the little-endian f32 sequence —
    // the reproducible content-addressed form the DatasetId depends on.
    let v = [1.0f32, -0.5, 2.25];
    let bytes = encode_vector_le(&v);
    assert_eq!(bytes.len(), 12);
    assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
    assert_eq!(&bytes[4..8], &(-0.5f32).to_le_bytes());
    assert_eq!(&bytes[8..12], &2.25f32.to_le_bytes());
}
