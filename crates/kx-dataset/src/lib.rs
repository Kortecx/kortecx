// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx-dataset` — the kortecx data-management seam (P4.1c).
//!
//! A pluggable, **journal-authoritative** data layer for the Morphic engine
//! (`kx-workflow`). It types committed content — tensors, vectors, blobs, text,
//! and forward-stubbed multi-modal payloads — behind a small trait surface, and
//! it is always a *reconstructible projection/cache* of committed content, never
//! a second source of truth. Lose the store and it rebuilds by re-folding
//! committed content; correctness lives in the journal (D40) + content
//! addressing (D17), not here.
//!
//! # Shape
//!
//! - [`ContentSchema`] / [`TypedRef`] / [`TensorDType`] — the typed-ref hook that
//!   lets transforms + critics reason over multi-modal content (the multi-modal
//!   layer, P10, extends the enum).
//! - [`DataStore`] + [`InMemoryDataStore`] — store/read typed payloads by content
//!   ref. A Lance backend (vectors + tensors + blobs + Delta versioning) is a
//!   later gated step behind the same trait.
//! - [`Dataset`] / [`DatasetId`] — a content-addressed corpus of typed rows + the
//!   Motes that produced it; its identity is a **pure function** of rows +
//!   lineage (reproducible-by-reference — the recipe-as-product / Delta-sharing
//!   basis, P4.1e).
//! - [`RetrievalIndex`] + [`InMemoryRetrievalIndex`] — the vector / graph-RAG
//!   similarity seam. **Used ONLY inside ReadOnlyNondet retrieval Motes** — the
//!   runtime matches by exact cryptographic equality (SN-8); similarity never
//!   touches the identity / commit / memoization path.
//! - [`AnnotationStore`] + [`Annotation`] — an advisory, mutable, rebuildable
//!   curation projection keyed by content ref (usefulness / yes-no / reviewer /
//!   notes). **Off the truth path**: never journaled, never on the identity path,
//!   never gates execution. It lives here precisely because the guarantee-path
//!   crates do not depend on `kx-dataset` — the dependency graph is the wall
//!   (see [`annotation`]).

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod annotation;
mod error;
pub mod fusion;
mod index;
mod lexical;
mod schema;
mod store;

pub use annotation::{Annotation, AnnotationStore};
pub use error::DataError;
pub use fusion::{
    index_fingerprint, mmr_rerank, rrf_fuse, INDEX_FORMAT_VERSION, MMR_LAMBDA_BP, RRF_C,
};
pub use index::{Hit, InMemoryRetrievalIndex, RetrievalIndex};
pub use lexical::LexicalIndex;
pub use schema::{ContentSchema, TensorDType, TypedRef};
pub use store::{DataStore, Dataset, DatasetId, InMemoryDataStore};

#[cfg(test)]
mod tests;
