//! Graph-RAG as a `ReadOnlyNondet` retrieval Mote, and the SN-8 boundary: the
//! committed retrieval fact is the neighbour SET (exact content refs), with
//! similarity scores excluded — so similarity never reaches a `MoteId`.
#![allow(clippy::unwrap_used)]

use kx_content::ContentRef;
use kx_dataset::{Hit, InMemoryRetrievalIndex, RetrievalIndex};
use kx_mote::{LogicRef, ModelId, NdClass, ToolName};
use kx_workflow::{
    compile, encode_retrieval_fact, permissive_warrant, retrieval, retrieval_result_ref,
    WorkflowDef,
};

fn model() -> ModelId {
    ModelId("local".into())
}

#[test]
fn retrieval_step_compiles_to_read_only_nondet_mote() {
    let warrant = permissive_warrant(model());
    let mut wf = WorkflowDef::new(0);
    wf.add_step(retrieval(
        LogicRef::from_bytes([7; 32]),
        model(),
        warrant,
        ToolName("rag".into()),
    ));
    let compiled = compile(&wf).unwrap();
    let mote = &compiled.motes[0].mote;
    assert_eq!(
        mote.nd_class(),
        NdClass::ReadOnlyNondet,
        "retrieval is a nondeterministic read — similarity stays inside the Mote body"
    );
    assert!(!mote.def.is_topology_shaper);
}

#[test]
fn end_to_end_retrieval_produces_a_content_addressed_fact() {
    // A real similarity query (inside the would-be Mote body) ...
    let mut index = InMemoryRetrievalIndex::new();
    let a = ContentRef::of(b"doc-a");
    let b = ContentRef::of(b"doc-b");
    index.insert(a, vec![1.0, 0.0]);
    index.insert(b, vec![0.0, 1.0]);
    let hits = index.query(&[1.0, 0.0], 2);

    // ... yields an EXACT content-addressed fact downstream consumes by hash.
    let fact_ref = retrieval_result_ref(&hits);
    assert_eq!(fact_ref, ContentRef::of(&encode_retrieval_fact(&hits)));
    // Deterministic: same hits → same fact.
    assert_eq!(retrieval_result_ref(&hits), fact_ref);
}

#[test]
fn similarity_scores_do_not_leak_into_the_committed_fact() {
    // Two retrieval results with the SAME neighbour ids but DIFFERENT scores
    // must produce the SAME committed fact — proof that similarity (the float
    // score) never reaches the content-addressed identity (SN-8).
    let id1 = ContentRef::of(b"n1");
    let id2 = ContentRef::of(b"n2");
    let high = [
        Hit {
            id: id1,
            score: 0.99,
        },
        Hit {
            id: id2,
            score: 0.50,
        },
    ];
    let low = [
        Hit {
            id: id1,
            score: 0.11,
        },
        Hit {
            id: id2,
            score: 0.02,
        },
    ];
    assert_eq!(
        encode_retrieval_fact(&high),
        encode_retrieval_fact(&low),
        "scores must be excluded from the committed fact"
    );
    assert_eq!(retrieval_result_ref(&high), retrieval_result_ref(&low));

    // A different neighbour SET, however, IS a different fact.
    let different = [Hit {
        id: ContentRef::of(b"n3"),
        score: 0.99,
    }];
    assert_ne!(
        retrieval_result_ref(&high),
        retrieval_result_ref(&different)
    );
}
