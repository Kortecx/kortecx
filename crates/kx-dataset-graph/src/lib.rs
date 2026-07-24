// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx-dataset-graph` — the opt-in GRAPH-RAG retrieval leg.
//!
//! An in-memory knowledge graph of `(subject, predicate, object)` triples, each
//! tagged with the source chunk's [`kx_content::ContentRef`], plus a deterministic
//! multi-hop neighbour walk that returns the source refs nearest a set of seed
//! entities. It is the THIRD fusion leg alongside the dense (HNSW) + sparse (BM25)
//! legs — RRF-fused by rank, so a source that a pure similarity search never
//! surfaces (because no single chunk mentions both query endpoints) is bridged
//! through the graph.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** Used ONLY inside the ReadOnlyNondet retrieval Mote. The graph is a
//!   nondeterministic, extraction-derived read of the world; only the ordered
//!   neighbour-ref SET is committed, matched downstream by exact hash. Traversal
//!   never reaches a `MoteId` — so extraction non-determinism is safe here.
//! - **Rebuildable projection.** The graph is a cache built from committed chunk
//!   content at ingest; lose it and it rebuilds by re-extracting (D40). PR-1 holds
//!   it in memory for the serve's lifetime; a durable triple store is a follow-up.
//! - **Off the default path.** Consumed only behind an opt-in flag
//!   (`KX_FLAG_SERVE_GRAPH_RAG`); an empty graph leg fuses to a byte-identical
//!   result, so the default build + the frozen execution kernel stay byte-unchanged.
#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation
    )
)]

mod graph;
mod triple;

pub use graph::{InMemoryGraph, KnowledgeGraph};
pub use triple::{normalize_entity, Triple};

#[cfg(test)]
mod tests;
