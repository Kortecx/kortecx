// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx-dataset-bm25` — the opt-in SPARSE (keyword / BM25) retrieval backend (RC4a).
//!
//! A hand-rolled, pure-Rust, FFI-free inverted index that implements
//! `kx_dataset::LexicalIndex`. It ranks documents by lexical term overlap (Okapi
//! **BM25+**, with a non-negative IDF), the keyword leg of hybrid retrieval — it
//! catches exact-term matches (names, codes, rare words) that a weak decoder-LLM
//! sentence embedding mis-ranks (`T-RAG-EMBED-QUALITY`).
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** Used ONLY inside the ReadOnlyNondet retrieval Mote. The BM25 score
//!   is a display/ranking aid that is rank-fused (`kx_dataset::fusion::rrf_fuse`)
//!   and then discarded; only the ordered content-ref SET is committed, matched
//!   downstream by exact hash. A score never reaches a `MoteId`.
//! - **Journal-authoritative.** The on-disk form is a REBUILDABLE CACHE of
//!   `(ContentRef, text)` records; the inverted index is re-tokenized on `open`,
//!   so a tokenizer break or corrupt file is recovered by rebuild (D40), never a
//!   migration.
//! - **Deterministic.** A fixed tokenizer (lowercase + alphanumeric runs), per-doc
//!   score accumulation in a fixed (sorted-query-term) order, and a `score desc,
//!   ref asc` tiebreak byte-identical to the dense backends ⇒ the ordered-ref
//!   result is reproducible across machines.
#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    // BM25 is inherently floating-point over integer corpus stats (tf / df / doc
    // length / avgdl). u32→f64 is lossless; the one u64 (total token count) → f64
    // cast is precision-safe for any single-node corpus. Compact u32 doc ids and
    // the display-only f32 `Hit.score` narrow intentionally.
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod index;
mod open;
mod persist;
mod tokenize;

pub use error::Bm25Error;
pub use index::{Bm25Index, Bm25Params};
pub use open::{dump, open};
pub use tokenize::TOKENIZER_VERSION;

#[cfg(test)]
mod tests;
