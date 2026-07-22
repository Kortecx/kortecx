// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
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
//! - **M7.2 — namespacing + grants/RBAC + revocation** (`path` / `party` /
//!   `action` / `grant` / `ledger` / `in_memory_ledger`, D86): an asset lives at
//!   an [`AssetPath`] (`namespace/collection/name`) bound to an owner; a
//!   content-addressed [`Grant`] issues a grantee [`CatalogActionSet`] catalog
//!   actions plus a runtime scope, **narrowing-only** through the FROZEN
//!   `kx_warrant::intersect` (the model can never authorize a widen). A
//!   [`Revocation`] is a NEW fact (D-LOCK-4) honored only for an authorized
//!   revoker. [`GrantLedger`] is backend-agnostic (in-memory now; durable / cloud
//!   later, D94); "journaled" is realized as the append-only, content-addressed,
//!   immutable, idempotent discipline — this crate still **never depends on
//!   `kx-journal`**. Authorization is the fail-closed [`GrantLedger::effective_grants`]
//!   fold, never trusted from a fact; the runtime warrant a `Use` runs under is
//!   action-aligned ([`GrantLedger::resolve_effective_warrant_for`]).
//! - **M7.2 — content-versioning + provenance/lineage** (`version` /
//!   `version_ledger` / `in_memory_version_ledger` / `governed`, D82/D88 +
//!   D-LOCK-4): an [`AssetPath`] is a **mutable handle** that resolves to an
//!   immutable, content-addressed [`VersionedContent`]; "update" is never a
//!   mutation — you **publish a new [`AssetVersion`]** and the [`VersionLedger`]
//!   moves the handle (prior versions retained forever; rollback is a new fact).
//!   Lineage is a **computed-not-stored** fold over the `prior` edges (the
//!   `kx_projection::transitive_consumers` pattern, without the dependency) and is
//!   ADVISORY (D84) — it never gates. [`GovernedCatalog`] is the production
//!   surface: publishing requires a [`CatalogAction::Register`] grant, viewing
//!   requires `Read` — composing the [`GrantLedger`] with the [`VersionLedger`]
//!   without coupling either impl.
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

mod action;
mod advertise;
mod body;
mod discovery;
mod discovery_index;
mod entry;
mod governed;
mod grant;
mod in_memory;
mod in_memory_ledger;
mod in_memory_version_ledger;
mod ledger;
mod metadata;
mod party;
mod path;
mod registry;
mod signature;
mod sqlite_body_ledger;
mod sqlite_catalog;
mod sqlite_grant_ledger;
mod sqlite_util;
mod sqlite_version_ledger;
mod version;
mod version_ledger;

#[cfg(test)]
mod tests;

pub use entry::{FreeParamContract, FreeParamSlot, RecipeSnapshot, SignatureEntry, SlotBinding};
pub use in_memory::InMemoryCatalog;
pub use registry::{CatalogError, CatalogRegistry, RegistrationOutcome};
pub use signature::{
    canonical_config, SignatureAxis, TaskSignature, TaskSignatureHash, VerdictScope,
    TASK_SIGNATURE_SCHEMA_VERSION,
};

// M7.2 (D86) — namespacing + grants/RBAC + revocation.
pub use action::{CatalogAction, CatalogActionSet};
pub use grant::{
    effective_runtime_warrant, revocation_idempotency_key, Grant, GrantId, Revocation,
    RevocationId, GRANT_SCHEMA_VERSION,
};
pub use in_memory_ledger::InMemoryGrantLedger;
pub use ledger::{
    AppendOutcome, AssetBinding, EffectiveGrants, FactId, GrantLedger, GrantWarrant, LedgerError,
    LedgerFact, MAX_DELEGATION_DEPTH,
};
pub use party::PartyId;
pub use path::{AssetPath, AssetPathError, AssetRef, MAX_SEGMENT_LEN};

// M7.2 (D82/D88 + D-LOCK-4) — content-versioning handles + provenance/lineage.
pub use governed::{GovernedCatalog, GovernedError};
pub use in_memory_version_ledger::InMemoryVersionLedger;
pub use version::{
    AssetVersion, Provenance, VersionError, VersionId, VersionedContent,
    CATALOG_VERSION_SCHEMA_VERSION, MAX_PROVENANCE_LINEAGE,
};
pub use version_ledger::{
    PublishOutcome, VersionLedger, VersionLedgerError, MAX_VERSION_CHAIN_DEPTH,
    MAX_VERSION_DESCENDANTS,
};

// M7.3 (D85) — Mote-as-MCP advertisement (descriptor only; execution is M8/D121).
pub use advertise::{
    advertise_snapshot, encode_param_schema, free_params_to_input_schema, AdvertiseError,
    SchemaResolver, SnapshotAdvertisement,
};

// M8 (D121) — content-addressed recipe-BODY storage: turns "advertised" into
// "servable" by resolving the executable WorkflowDef a published recipe runs.
pub use body::{body_manifest_id, BodyLedger, BodyLedgerError, BodyOutcome, InMemoryBodyLedger};

// G1 (D94) — durable SQLite backends behind the EXISTING ledger traits: catalog
// (recipes/grants/versions/bodies) survives a process restart. Each is a second
// impl of its trait (the in-memory ones stay); the durable+in-memory pair is the
// "backend-agnostic" discipline (as SqliteJournal/InMemoryJournal). OFF the
// guarantee path — adding rusqlite here is no spine→catalog edge (the wall holds).
pub use sqlite_body_ledger::{SqliteBodyLedger, BODY_LEDGER_SCHEMA_VERSION};
pub use sqlite_catalog::{SqliteCatalog, CATALOG_SCHEMA_VERSION};
pub use sqlite_grant_ledger::{SqliteGrantLedger, GRANT_LEDGER_SCHEMA_VERSION};
pub use sqlite_version_ledger::{SqliteVersionLedger, VERSION_LEDGER_SCHEMA_VERSION};
// The M5.3 closed, no-float typed-arg schema, re-exported so an advertisement's
// `input_schema` is one import surface (the SAME schema M8's `validate_args` uses).
pub use kx_tool_registry::{validate_args, InputSchema, ParamSpec, ParamType, SchemaError};

// M7.3 (D87/D84) — discovery (fuzzy-in, exact-out) + advisory metadata.
pub use discovery::{commit_selection, CatalogDiscovery, FuzzyDiscovery, SelectionFact};
// One import surface for catalog callers building discovery: the content-address
// type (in `SelectionFact`/`commit_selection`) and the similarity seam the fuzzy
// surface wraps (`kx_dataset`, already a dependency; the SN-8-confined ANN seam).
pub use discovery_index::{DiscoveryIndex, InMemoryDiscoveryIndex, MAX_DISCOVERY_RESULT};
pub use kx_content::ContentRef;
pub use kx_dataset::{Hit, InMemoryRetrievalIndex, RetrievalIndex};
pub use metadata::{
    AdvisoryMetadata, AdvisoryMetadataStore, Tag, TagError, MAX_TAGS_PER_ASSET, MAX_TAG_LEN,
};

// REUSE (never modify) the frozen monotonic-narrowing seam — one import surface
// for catalog callers building grant runtime scopes (M7.2 consumes `intersect`;
// it adds no warrant axis — `secret_scope`/`cost_ceiling`/`tls_required` landed
// in M5.3a).
pub use kx_warrant::{intersect, NarrowingError, Role, WarrantSpec};
