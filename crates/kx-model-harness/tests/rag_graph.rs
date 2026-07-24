//! Graph-RAG PR-1 — the graph-fusion glue (`ingest_corpus_graph` /
//! `query_corpus_graph`) end-to-end with SYNTHETIC vectors + a stub extractor (no
//! live model), so this runs in the default `cargo test` pass. Proves:
//! (1) a MULTI-HOP query surfaces a bridge chunk that dense+sparse alone exclude
//!     (the answer chunk names neither the query entity nor its direct neighbour), and
//! (2) with an EMPTY graph the fused result is byte-identical to `query_corpus_hybrid`
//!     — the graph-RAG-OFF invariant.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use kx_content::ContentRef;
use kx_dataset::{InMemoryDataStore, InMemoryRetrievalIndex, RetrievalIndex};
use kx_dataset_bm25::Bm25Index;
use kx_dataset_graph::InMemoryGraph;
use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams,
};
use kx_model_harness::{
    ingest_corpus_graph, ingest_corpus_hybrid, query_corpus_graph, query_corpus_hybrid, Embedder,
    Extractor,
};
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

// A 3-link chain Acme→Beta→Gamma→Orion plus one unrelated distractor. The answer
// (doc-C) names neither the query entity (Acme) nor its direct neighbour (Beta), so
// only a 2-hop graph walk connects it.
const DOC_A: &str = "Acme partnered with Beta on a venture.";
const DOC_B: &str = "Beta owns Gamma outright.";
const DOC_C: &str = "Gamma built the Orion reactor last year.";
const DOC_D: &str = "Delta makes widgets for retail.";
const QUERY: &str = "What does Acme control?";

/// A combined stub: a controlled embedder (so dense retrieval ranks A > B > D > C,
/// leaving the answer C last) AND a fixed-triple/entity extractor keyed off the
/// chunk text in the prompt. No live model — deterministic.
struct GraphStub;

fn embed_vec(text: &str) -> Vec<f32> {
    let t = text.to_lowercase();
    // Query + Acme-doc share the query direction (cosine 1); the rest are graded
    // strictly below, with the answer chunk (gamma/orion) the FARTHEST from the query.
    if t.contains("acme") {
        vec![1.0, 0.0]
    } else if t.contains("beta") {
        vec![0.8, 0.6]
    } else if t.contains("delta") {
        vec![0.6, 0.8]
    } else if t.contains("gamma") || t.contains("orion") {
        vec![0.1, 1.0]
    } else {
        vec![0.0, 1.0]
    }
}

impl EmbeddingBackend for GraphStub {
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        _pooling: EmbeddingPooling,
        _warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        let vector = embed_vec(text);
        Ok(EmbeddingOutput {
            dim: vector.len() as u32,
            vector,
            backend_name: "graph-stub",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }
}

impl InferenceBackend for GraphStub {
    fn dispatch(
        &self,
        _model_id: &ModelId,
        input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        let prompt = match input {
            InferenceInput::Text(s) => s.as_str(),
            _ => "",
        };
        // The query-entity prompt vs the triple-extraction prompt.
        let reply = if prompt.contains("named entities") {
            "[\"Acme\"]".to_string()
        } else if prompt.contains("partnered") {
            "[{\"subject\":\"Acme\",\"predicate\":\"partnered\",\"object\":\"Beta\"}]".to_string()
        } else if prompt.contains("owns") {
            "[{\"subject\":\"Beta\",\"predicate\":\"owns\",\"object\":\"Gamma\"}]".to_string()
        } else if prompt.contains("Orion") {
            "[{\"subject\":\"Gamma\",\"predicate\":\"built\",\"object\":\"Orion\"}]".to_string()
        } else if prompt.contains("widgets") {
            "[{\"subject\":\"Delta\",\"predicate\":\"makes\",\"object\":\"widgets\"}]".to_string()
        } else {
            "[]".to_string()
        };
        Ok(InferenceOutput {
            bytes: reply.into_bytes(),
            output_tokens: 0,
            backend_name: "graph-stub",
            model_id: model(),
            elapsed: Duration::from_millis(0),
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "graph-stub"
    }
}

#[test]
fn graph_leg_surfaces_a_two_hop_bridge_chunk_hybrid_excludes() {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let mut lexical = Bm25Index::new();
    let mut graph = InMemoryGraph::new();
    let backend = GraphStub;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);
    let extractor = Extractor::new(&backend, &m, &w);

    let docs = [DOC_A, DOC_B, DOC_C, DOC_D];
    ingest_corpus_graph(
        &store,
        &mut index,
        &mut lexical,
        &mut graph,
        &embedder,
        &extractor,
        &docs,
        &[],
    )
    .unwrap();
    assert_eq!(index.len(), 4);
    // Four docs → four triples in the graph (one per doc).
    use kx_dataset_graph::KnowledgeGraph;
    assert_eq!(graph.len(), 4);

    let answer = ContentRef::of(DOC_C.as_bytes());

    // Dense (A>B>D>C) + sparse (only "Acme" matches doc-A) rank the answer LAST, so at
    // k=3 hybrid returns {A, B, D} — the answer chunk is excluded.
    let (_f, hybrid) = query_corpus_hybrid(&index, &lexical, &embedder, QUERY, 3, false).unwrap();
    let hybrid_ids: Vec<ContentRef> = hybrid.iter().map(|h| h.id).collect();
    assert!(
        !hybrid_ids.contains(&answer),
        "dense+sparse alone must miss the 2-hop answer chunk (got {hybrid_ids:?})"
    );

    // The graph leg (seed Acme → Beta → Gamma) surfaces doc-C into the top-3.
    let (_f, with_graph) = query_corpus_graph(
        &index, &lexical, &graph, &embedder, &extractor, QUERY, 3, false,
    )
    .unwrap();
    let graph_ids: Vec<ContentRef> = with_graph.iter().map(|h| h.id).collect();
    assert!(
        graph_ids.contains(&answer),
        "the multi-hop graph leg must surface the bridge chunk (got {graph_ids:?})"
    );
}

#[test]
fn an_empty_graph_leaves_the_result_byte_identical_to_hybrid() {
    // The graph-RAG-OFF invariant at the glue level: with an EMPTY graph, the graph
    // leg is empty and the fused result byte-equals `query_corpus_hybrid`.
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let mut lexical = Bm25Index::new();
    let empty_graph = InMemoryGraph::new();
    let backend = GraphStub;
    let (m, w) = (model(), warrant());
    let embedder = Embedder::new(&backend, &m, &w, EmbeddingPooling::Mean);
    let extractor = Extractor::new(&backend, &m, &w);

    let docs = [DOC_A, DOC_B, DOC_C, DOC_D];
    ingest_corpus_hybrid(&store, &mut index, &mut lexical, &embedder, &docs, &[]).unwrap();

    for k in [1usize, 3, 4] {
        let (hf, hybrid) =
            query_corpus_hybrid(&index, &lexical, &embedder, QUERY, k, false).unwrap();
        let (gf, graphed) = query_corpus_graph(
            &index,
            &lexical,
            &empty_graph,
            &embedder,
            &extractor,
            QUERY,
            k,
            false,
        )
        .unwrap();
        assert_eq!(hf, gf, "committed fact ref must match at k={k}");
        assert_eq!(hybrid.len(), graphed.len());
        for (a, b) in hybrid.iter().zip(graphed.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.score.to_bits(), b.score.to_bits());
        }
    }
}
