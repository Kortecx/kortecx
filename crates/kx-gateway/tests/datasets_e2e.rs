//! End-to-end witnesses for the T3.7 Datasets data-plane over a REAL bound port,
//! exercising the FFI-FREE client-vector path (no model / no Metal — Linux/CI safe):
//!
//! - `IngestDocuments` with explicit client vectors creates a dataset + indexes it;
//! - `ListDatasets` enumerates it with the right doc-count + dimension;
//! - `QueryDataset` with a query vector returns the nearest document's bytes + ref;
//! - re-ingesting identical content is a no-op (content-addressed dedup);
//! - a text-only query with NO embedder is `FAILED_PRECONDITION` (honest degrade);
//! - an unknown dataset is `NOT_FOUND`;
//! - the dataset is durable across a restart (the `SQLite` store + rebuilt `HNSW` index).
//!
//! This is the deterministic happy path the SDK/UI contract tests build on.

#![cfg(all(feature = "embedded-worker", feature = "hnsw"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use kx_gateway::start;
use kx_proto::proto;

/// A 4-dim vector pointing mostly along one axis — clearly separated so the
/// (approximate) HNSW order is unambiguous for these tiny corpora.
fn vec4(a: f32, b: f32, c: f32, d: f32) -> Vec<f32> {
    vec![a, b, c, d]
}

fn doc(content: &[u8], embedding: Vec<f32>) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding,
        ..Default::default()
    }
}

#[tokio::test]
async fn ingest_list_query_client_vectors_end_to_end() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    let ingest = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: "corpus".into(),
            documents: vec![
                doc(b"alpha", vec4(1.0, 0.0, 0.0, 0.1)),
                doc(b"bravo", vec4(0.0, 1.0, 0.0, 0.1)),
                doc(b"charlie", vec4(0.0, 0.0, 1.0, 0.1)),
            ],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ingest.dataset_id, "corpus");
    assert_eq!(ingest.inserted, 3);
    assert_eq!(ingest.doc_count, 3);
    assert_eq!(ingest.dim, 4);

    let list = c
        .list_datasets(proto::ListDatasetsRequest {})
        .await
        .unwrap()
        .into_inner()
        .datasets;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].dataset_id, "corpus");
    assert_eq!(list[0].doc_count, 3);
    assert_eq!(list[0].dim, 4);

    // Closest to axis 1 ⇒ "bravo" is the top hit; bytes attached, ref is 32B.
    let hits = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "corpus".into(),
            query_text: String::new(),
            query_embedding: vec4(0.0, 1.0, 0.0, 0.1),
            k: 3,
        })
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert!(!hits.is_empty(), "a non-empty corpus returns hits");
    assert_eq!(hits[0].content, b"bravo");
    assert_eq!(hits[0].content_ref.len(), 32);

    // Re-ingesting identical content is a no-op (content-addressed dedup).
    let again = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: "corpus".into(),
            documents: vec![doc(b"alpha", vec4(1.0, 0.0, 0.0, 0.1))],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.inserted, 0);
    assert_eq!(again.doc_count, 3);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn text_query_without_an_embedder_is_failed_precondition() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    c.ingest_documents(proto::IngestDocumentsRequest {
        dataset: "corpus".into(),
        documents: vec![doc(b"alpha", vec4(1.0, 0.0, 0.0, 0.1))],
    })
    .await
    .unwrap();

    let err = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "corpus".into(),
            query_text: "find alpha".into(),
            query_embedding: Vec::new(),
            k: 1,
        })
        .await
        .unwrap_err();
    // The FFI-free (no `inference`) build has no embedder → an honest degrade. With
    // `--features inference` + a resolved model the same request would embed + succeed,
    // so only assert the code on the FFI-free build the SDK/UI contract jobs run.
    #[cfg(not(feature = "inference"))]
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    #[cfg(feature = "inference")]
    let _ = err;

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_dataset_query_is_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    let err = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "nope".into(),
            query_text: String::new(),
            query_embedding: vec4(1.0, 0.0, 0.0, 0.1),
            k: 1,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn datasets_are_durable_across_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    {
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = common::connect_client(running.local_addr()).await;
        c.ingest_documents(proto::IngestDocumentsRequest {
            dataset: "corpus".into(),
            documents: vec![
                doc(b"alpha", vec4(1.0, 0.0, 0.0, 0.1)),
                doc(b"bravo", vec4(0.0, 1.0, 0.0, 0.1)),
            ],
        })
        .await
        .unwrap();
        running.shutdown().await.unwrap();
    }

    // Restart on the SAME dir: the durable SQLite store + the rebuilt-on-open HNSW
    // index recover the dataset, and a query still serves the right neighbour.
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;
    let list = c
        .list_datasets(proto::ListDatasetsRequest {})
        .await
        .unwrap()
        .into_inner()
        .datasets;
    assert_eq!(list.len(), 1, "the dataset survives a restart");
    assert_eq!(list[0].doc_count, 2);
    let hits = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "corpus".into(),
            query_text: String::new(),
            query_embedding: vec4(1.0, 0.0, 0.0, 0.1),
            k: 1,
        })
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert_eq!(hits[0].content, b"alpha");

    running.shutdown().await.unwrap();
}
