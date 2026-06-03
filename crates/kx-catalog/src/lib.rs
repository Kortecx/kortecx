// SPDX-License-Identifier: Apache-2.0
//! # kx-catalog — the snapshot registry + sharable catalog (M7, D82/D83)
//!
//! The kortecx **catalog** is the Unity-Catalog-class registry of reusable
//! assets. This crate lands its foundation in two layers:
//!
//! - **M7.0 — [`TaskSignature`]** (`signature`): the verdict-reuse identity
//!   foundation. A `TaskSignature` pins the deterministic critic terminating a
//!   chain plus the closed set of [`SignatureAxis`] a reuse must additionally
//!   match. It is built ONLY via [`TaskSignature::model_invariant`] /
//!   [`TaskSignature::scoped`] and content-addressed by [`TaskSignatureHash`]
//!   (`blake3` over a domain-tagged canonical bincode encoding, exactly the
//!   `kx_workflow::ManifestId` / `kx_mote::MoteDef::hash` discipline). The
//!   [`VerdictScope`] expresses "this critic's `Valid` verdict is reusable for
//!   runs matching this signature".
//! - **M7.1 — the registry** (`registry` / `in_memory`): a content-addressed,
//!   **idempotent + immutable** store of [`SignatureEntry`] keyed by
//!   `TaskSignatureHash`. [`CatalogRegistry`] is backend-agnostic (in-memory now;
//!   a persistent / cloud backend is a later impl behind the same trait, exactly
//!   as `kx_content::ContentStore` and `kx_dataset::RetrievalIndex`).
//!
//! ## A separate truth (R4 — NOT a journal-as-truth violation)
//!
//! The catalog is authoritative for **what recipes exist** (immutable,
//! content-addressed, idempotent registration); the journal stays authoritative
//! for **what runs did**. This crate therefore **never** writes the journal and
//! **does not depend on `kx-journal`** — the dependency direction is the wall.
//! Registering a signature creates no run and mutates no committed fact; a
//! snapshot invocation (later milestones) is always a FRESH registered run
//! (`WorldMutating` work re-runs by default — D83), never a replayed result.
//!
//! ## The SN-8 wall (load-bearing)
//!
//! Like `kx_dataset::AnnotationStore`, the catalog is **off the trust path**: it
//! never gates selection, eviction, or promotion. The promotion decision stays
//! `kx_projection::promotion`'s fail-closed exact `CriticVerdict::is_valid`
//! check. A [`VerdictScope`] is *recorded* here for reuse bookkeeping; it never
//! short-circuits that gate. The wall is enforced by the dependency graph: the
//! guarantee-path crates (`kx-scheduler` / `kx-executor` / `kx-projection`) do
//! NOT depend on `kx-catalog`, so the compiler rejects wiring catalog data onto
//! the identity / commit / selection path. **No floats** touch any type here, so
//! even a future mistake could carry none onto a canonical hash.
//!
//! ## Verdict-reuse lives HERE, not on `MoteDef`
//!
//! A [`VerdictScope`] is stored in [`SignatureEntry`], **not** as a
//! `MoteDef.verdict_for` field. This keeps `kx-mote` byte-unchanged and the
//! canonical projection digest invariant (a `MoteDef` schema bump would move it).
//! It mirrors the established pattern of keeping reuse / advisory metadata off
//! the identity path (`AnnotationStore`, `kx-memoizer`).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
// `.expect()` on canonical-bincode encode of a type WITHOUT floats and WITHOUT
// non-encodable variants IS infallible (the single site is
// `TaskSignature::task_signature_hash`), and `.expect("poisoned lock")` on the
// `InMemoryCatalog` RwLock is the correct propagate-on-catastrophe behavior.
// Both sites carry an inline justification; this crate-level allow suppresses
// the workspace `clippy::expect_used = "deny"` policy for those documented uses
// (mirrors kx-mote / kx-critic-types / kx-content).
#![allow(clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod entry;
mod in_memory;
mod registry;
mod signature;

#[cfg(test)]
mod tests;

pub use entry::{FreeParamContract, FreeParamSlot, RecipeSnapshot, SignatureEntry, SlotBinding};
pub use in_memory::InMemoryCatalog;
pub use registry::{CatalogError, CatalogRegistry, RegistrationOutcome};
pub use signature::{
    canonical_config, SignatureAxis, TaskSignature, TaskSignatureHash, VerdictScope,
    TASK_SIGNATURE_SCHEMA_VERSION,
};
