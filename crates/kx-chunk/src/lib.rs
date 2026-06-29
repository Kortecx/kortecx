// SPDX-License-Identifier: Apache-2.0
//! `kx-chunk` — the kortecx deterministic document chunker (RC4a).
//!
//! A pure, `std`-only, recursive **character** text splitter. It segments a
//! document into bounded, overlapping passages along a fixed separator hierarchy
//! (paragraph → line → sentence → word → hard char). Chunking turns a RAG hit
//! from "a whole document" into "the relevant passage", which both sharpens
//! retrieval and shrinks the grounded context an agent must reason over.
//!
//! # Determinism (load-bearing)
//!
//! Sizes are measured in **Unicode scalar values (chars)**, never bytes (so a
//! chunk never splits a codepoint) and never tokens (no model). The separator
//! hierarchy is a compile-time constant, the merge is greedy left-to-right, and
//! every offset is an integer char index — no locale, no RNG, no float. Identical
//! input bytes therefore yield identical `(char_start, char_end, index)` on any
//! machine. Each chunk maps to a **contiguous** `[char_start, char_end)` range of
//! the parent text, so `chunk.text` is an exact substring — content-addressable
//! and provenance-exact.
//!
//! # Fingerprint axis
//!
//! [`CHUNKER_VERSION`] plus the [`ChunkParams`] (`max_chars` / `overlap_chars`)
//! are inputs to the retrieval-index fingerprint: changing the chunking changes
//! what was indexed, so a stale index is detected rather than silently mis-queried
//! (see `kx-dataset`'s `index_fingerprint`).
#![forbid(unsafe_code)]
#![allow(
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
        clippy::cast_possible_truncation
    )
)]

mod chunk;

pub use chunk::{
    chunk, Chunk, ChunkParams, CHUNKER_VERSION, DEFAULT_MAX_CHARS, DEFAULT_OVERLAP_CHARS,
};

#[cfg(test)]
mod tests;
