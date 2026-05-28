#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-tiering — tag-driven storage tiering (P1.12)
//!
//! Reuses the per-Mote [`NdClass`](kx_mote::NdClass) tag for memory management.
//! The tag already does double duty in the runtime — a recovery gate *and* a
//! tiering signal (`mote.md` §6):
//!
//! - **PURE** — output is a deterministic function of inputs, so the payload is
//!   **droppable + recomputable**. Under memory pressure it may be evicted; a
//!   later read returns [`NotFound`](kx_content::NotFound) and the consumer
//!   recomputes the Mote by re-running its deterministic logic (re-running re-puts
//!   bit-identical bytes → the *same* content ref, by content-addressing).
//! - **READ-ONLY-NONDET** and **WORLD-MUTATING** — not recomputable, so the
//!   payload is **always persisted** and is *never* evicted by tiering.
//!
//! ## The content store stays tag-blind
//!
//! Per `content-store.md` §7 the [`ContentStore`](kx_content::ContentStore) only
//! stores / retrieves / deletes bytes by ref and must not depend on the journal.
//! This crate is the **tiering pass**: it reads a projection
//! [`Snapshot`](kx_projection::Snapshot) to learn which committed `result_ref`s
//! belong to PURE Motes, then directs the store to [`delete`](kx_content::ContentStore::delete)
//! them. Tiering and the orphan-GC walker share that one idempotent `delete(ref)`
//! primitive; the store does not distinguish eviction-by-tiering from
//! deletion-by-orphan-GC.
//!
//! ## Shared-ref protection (content-addressed dedup)
//!
//! Because refs are content-addressed BLAKE3, two distinct committed Motes with
//! identical payload bytes resolve to the **same** ref. A ref is evictable **iff
//! every committed, non-repudiated Mote that resolves to it is PURE** — any
//! WORLD-MUTATING / READ-ONLY-NONDET contributor protects the ref. See
//! [`select_candidates`].
//!
//! ## Usage
//!
//! ```no_run
//! use kx_tiering::{run_pass, TieringBudget};
//! # fn demo<S: kx_content::ContentStore>(snapshot: &kx_projection::Snapshot, store: &S)
//! #   -> Result<(), kx_tiering::TieringError> {
//! // Keep at most 64 resident PURE payload objects; evict oldest-commit-first.
//! let report = run_pass(snapshot, store, TieringBudget::MaxObjects(64))?;
//! println!("evicted {} refs, reclaimed {} bytes", report.evicted.len(), report.bytes_reclaimed);
//! # Ok(())
//! # }
//! ```

mod candidate;
mod error;
mod pass;
mod policy;

pub use candidate::{select_candidates, EvictionCandidate};
pub use error::TieringError;
pub use pass::{run_pass, EvictionReport};
pub use policy::{ResidentUsage, TieringBudget};
