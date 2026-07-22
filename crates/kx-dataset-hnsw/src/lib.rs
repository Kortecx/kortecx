// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx-dataset-hnsw` — the opt-in SCALE retrieval backend (DP3, T2.3).
//!
//! A file-backed, in-process approximate-nearest-neighbour (HNSW) index that
//! implements `kx_dataset::RetrievalIndex` via the pure-Rust `hnsw_rs` crate, for
//! corpora too large for the default exact brute-force scan.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** Used ONLY inside the ReadOnlyNondet retrieval Mote. Similarity
//!   stays inside; only the ordered neighbour-ref SET is committed, matched
//!   downstream by exact hash. The approximate, build-order-sensitive nature of
//!   HNSW never reaches a `MoteId` — so ANN non-determinism is safe here.
//! - **Journal-authoritative.** The on-disk form is a REBUILDABLE CACHE of
//!   `(ContentRef, vector)` records; the HNSW graph is rebuilt from them on
//!   `open`. The `hnsw_rs` internal dump format is intentionally NOT used, so the
//!   cache is decoupled from it: a format break or corrupt file is recovered by
//!   rebuild-from-journal (D40), never a migration.
//! - **Off the default path.** The exact `InMemoryRetrievalIndex` stays the
//!   reproducible default; this crate is consumed only behind an opt-in feature,
//!   so the frozen execution kernel + the default build stay byte-unchanged.
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
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )
)]

mod error;
mod index;
mod open;
mod persist;

pub use error::HnswError;
pub use index::{HnswParams, HnswRetrievalIndex};
pub use open::{dump, open, open_with_params};

#[cfg(test)]
mod tests;
