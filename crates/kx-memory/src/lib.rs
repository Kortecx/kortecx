// SPDX-License-Identifier: Apache-2.0
//! `kx-memory` — the kortecx durable agentic MEMORY subsystem (RC5a).
//!
//! Cross-run, per-namespace memory: what an agent *learned* in one run and can
//! *recall* in a later one. The design mirrors the proven RAG substrate
//! (`kx-dataset` / `kx-dataset-hnsw`), so it inherits the runtime's guarantees.
//!
//! # The two tiers (RC5a)
//!
//! - **Semantic / long-term** — [`MemoryStore::recall`] returns the memories most
//!   similar to a query vector. The similarity index is a rebuildable projection
//!   over the [`kx_dataset::RetrievalIndex`] seam (the exact, deterministic
//!   [`kx_dataset::InMemoryRetrievalIndex`] is the default; a file-backed
//!   `kx-dataset-hnsw` index is a drop-in for scale behind the same trait).
//! - **Episodic store** — [`MemoryStore::list`] is the durable, newest-first log
//!   of what was remembered (and by which run), filterable by run.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** A recall result is the ordered EXACT memory-ref SET only. The
//!   similarity [`MemoryHit::score`] is DISPLAY-ONLY — it never enters a committed
//!   fact, a `MoteId`, or any identity/commit path. Similarity stays *inside* the
//!   store; the runtime matches by exact cryptographic equality.
//! - **Journal-authoritative, off-digest, NO schema bump.** The store
//!   (`memory.db`) is a reconstructible projection/cache — lose it and it rebuilds
//!   from its durable rows (and, ultimately, by re-folding the committed
//!   `remember` action Motes). It is NEVER a second source of truth, NEVER
//!   journaled, and NEVER an input to a `MoteId` (the RC4b `retrieve@1` precedent —
//!   no `JOURNAL_SCHEMA_VERSION` bump).
//! - **Content-addressed, idempotent write.** [`memory_id`] is the content ref of
//!   the payload, so a repeated `remember` (e.g. an exactly-once pre-commit
//!   re-dispatch) is a durable no-op — never a duplicate row.
//! - **Per-namespace isolation.** Every operation is scoped to a `namespace` (the
//!   server-derived caller principal at the gateway). Recall over one namespace can
//!   NEVER surface another's memories — the similarity index is namespaced, so the
//!   isolation is structural, not a post-filter.
//! - **Embedder-agnostic.** The store takes/returns VECTORS; the caller (the
//!   gateway) embeds text. A cross-namespace `embed_fingerprint` guard refuses to
//!   mix incompatible vector spaces after a model upgrade ([`MemoryError::StaleIndex`]).
//!
//! # The dependency wall
//!
//! Guarantee-path crates (the `kx-mote` core, `kx-journal`, `kx-scheduler`,
//! `kx-executor`, `kx-projection`) do NOT depend on this crate — the compiler
//! enforces the direction every build. Moving memory "closer" to the writer for
//! convenience is itself the boundary violation.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod record;
mod sqlite;
mod store;

pub use error::MemoryError;
pub use record::{
    memory_id, now_ms, BundleRequest, DecayCandidate, DecayPolicy, DecayReport, MemoryHit,
    MemoryKind, MemoryRecord, MemoryStats, StoreOutcome, StoreRequest, MAX_CONTENT_LEN,
    MAX_NAMESPACE_LEN,
};
pub use sqlite::SqliteMemoryStore;
pub use store::MemoryStore;

#[cfg(test)]
mod tests;
