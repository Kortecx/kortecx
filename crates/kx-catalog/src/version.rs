// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Content-versioning facts (M7.2, D82/D88 + D-LOCK-4) — an [`AssetVersion`] is
//! the immutable, content-addressed "publish a new version" fact that the
//! [`crate::VersionLedger`] folds into a *mutable-handle → immutable-content*
//! mapping plus a *computed-not-stored* provenance/lineage chain.
//!
//! ## "Update" is never a mutation (D-LOCK-4)
//!
//! Versioning is by content: a new recipe → a new [`crate::TaskSignatureHash`], a
//! new workflow → a new `kx_workflow::ManifestId`. There is no edit-in-place. To
//! "update" the asset at a catalog path you **publish a new [`AssetVersion`]** (a
//! new immutable fact) and the ledger **moves the handle** ([`AssetPath`] →
//! latest). Prior versions are retained forever — a rollback is itself a new
//! `AssetVersion` whose `content` pins an OLDER [`VersionedContent`]. The
//! `prior` edge forms the append-only version chain; lineage is a fold over it.
//!
//! ## Provenance is advisory (D84), but tamper-evident
//!
//! [`Provenance`] (which recipe/run/corpus produced this content) is **advisory**
//! — it NEVER gates a runtime decision (only a [`crate::CatalogAction::Register`]
//! grant gates a publish, in [`crate::GovernedCatalog`]). But it folds into the
//! [`VersionId`], so a version's claimed provenance is content-addressed: a forged
//! provenance produces a different id, and a forged `prior` edge is caught by the
//! ledger's lineage-integrity fold. No float, no wall-clock touches any field —
//! ordering is the chain-derived `revision`, not a timestamp — so the canonical
//! bytes stay byte-deterministic (I1.c).

use kx_dataset::DatasetId;
use kx_mote::MoteId;
use kx_workflow::ManifestId;
use serde::{Deserialize, Serialize};

use crate::path::AssetPath;
use crate::signature::{canonical_config, TaskSignatureHash};

/// A 32-byte BLAKE3 hash — the catalog's identity substrate.
type Hash32 = [u8; 32];

/// Canonical-encoding schema version of an [`AssetVersion`]. Bumped on ANY change
/// to the canonical bytes (the version is a struct field, so it folds into the
/// [`VersionId`]). Mirrors [`crate::GRANT_SCHEMA_VERSION`].
pub const CATALOG_VERSION_SCHEMA_VERSION: u16 = 1;

/// The maximum number of `MoteId`s a [`Provenance`] may echo in `corpus_lineage`.
/// A hard bound on the hash-input size (the full corpus lineage is dereferenced
/// from `kx-dataset` via `dataset_id` when needed). Exceeding it is a loud,
/// fail-closed refusal — never a silent truncation.
pub const MAX_PROVENANCE_LINEAGE: usize = 4096;

/// The content-addressed identity of an [`AssetVersion`]:
/// `blake3(b"kx-catalog/asset-version/v1" ‖ canonical_bincode(version))`. The
/// domain tag prevents cross-type preimage aliasing — exactly the [`crate::GrantId`]
/// / [`crate::TaskSignatureHash`] discipline.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VersionId(pub Hash32);

impl VersionId {
    /// Construct from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }

    /// Lowercase 64-char hex.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl std::fmt::Debug for VersionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VersionId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

impl std::fmt::Display for VersionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The IMMUTABLE content a version pins. A closed enum — each variant is an
/// already-content-addressed identity from another crate, so "version content" is
/// always immutable (unlike an [`crate::AssetRef::Path`], which is a *mutable*
/// handle and is therefore deliberately NOT a valid version content). Growing the
/// set (e.g. a script or context-file variant for D88 bundles) is an additive,
/// deliberate [`CATALOG_VERSION_SCHEMA_VERSION`] bump — append new variants only,
/// so existing version ids never move.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum VersionedContent {
    /// A registered recipe (M7.0/M7.1), by its content hash.
    Recipe(TaskSignatureHash),
    /// A workflow recipe-as-product (`kx_workflow::Manifest`), by its id.
    Workflow(ManifestId),
    /// A produced corpus (`kx_dataset::Dataset`), by its id.
    Dataset(DatasetId),
}

/// Why a [`Provenance`] was rejected. Loud, typed refusal — never a silently
/// truncated lineage echo.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum VersionError {
    /// `corpus_lineage` exceeded [`MAX_PROVENANCE_LINEAGE`] entries.
    #[error("provenance corpus_lineage is {len} entries (max {MAX_PROVENANCE_LINEAGE})")]
    ProvenanceTooLarge {
        /// The over-large length.
        len: usize,
    },
}

/// Advisory provenance for a published version (D84): which recipe / run / corpus
/// produced this content. NEVER gates a runtime decision; recorded for audit and
/// lineage only. All fields are bytes/integers — NO float, NO wall-clock — and the
/// whole record folds into the [`VersionId`], so a forged provenance is
/// tamper-evident. Fields are private; built via [`Provenance::from_recipe`] + the
/// fallible/builder methods, so the [`MAX_PROVENANCE_LINEAGE`] bound cannot be
/// bypassed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Provenance {
    /// The recipe fingerprint that produced this content — equal to the value the
    /// submission layer records on the `RunRegistered` journal fact. Held as raw
    /// bytes so this crate stays off the `kx-journal` dependency (mirrors
    /// [`crate::RecipeSnapshot::recipe_fingerprint`]).
    recipe_fingerprint: [u8; 32],
    /// The generating run's `instance_id` (16-byte run nonce), raw + advisory —
    /// NOT a `kx-journal` dependency, NOT identity.
    generating_run: Option<[u8; 16]>,
    /// The pinned corpus this content was derived from / is, if any.
    dataset_id: Option<DatasetId>,
    /// An optional echo of the producing run's `Dataset.lineage` Motes (advisory;
    /// bounded by [`MAX_PROVENANCE_LINEAGE`]).
    corpus_lineage: Vec<MoteId>,
}

impl Provenance {
    /// Minimal provenance: just the recipe fingerprint that produced the content.
    #[must_use]
    pub fn from_recipe(recipe_fingerprint: [u8; 32]) -> Self {
        Self {
            recipe_fingerprint,
            generating_run: None,
            dataset_id: None,
            corpus_lineage: Vec::new(),
        }
    }

    /// Attach the generating run's `instance_id` (builder, advisory).
    #[must_use]
    pub fn with_run(mut self, instance_id: [u8; 16]) -> Self {
        self.generating_run = Some(instance_id);
        self
    }

    /// Pin the corpus this content was derived from (builder, advisory).
    #[must_use]
    pub fn with_dataset(mut self, dataset_id: DatasetId) -> Self {
        self.dataset_id = Some(dataset_id);
        self
    }

    /// Echo the producing run's corpus lineage (builder).
    ///
    /// # Errors
    ///
    /// [`VersionError::ProvenanceTooLarge`] if `lineage` exceeds
    /// [`MAX_PROVENANCE_LINEAGE`] — a loud, fail-closed refusal rather than a
    /// silent truncation (the full lineage is dereferenced from `kx-dataset`).
    pub fn with_corpus_lineage(
        mut self,
        lineage: impl IntoIterator<Item = MoteId>,
    ) -> Result<Self, VersionError> {
        let lineage: Vec<MoteId> = lineage.into_iter().collect();
        if lineage.len() > MAX_PROVENANCE_LINEAGE {
            return Err(VersionError::ProvenanceTooLarge { len: lineage.len() });
        }
        self.corpus_lineage = lineage;
        Ok(self)
    }

    /// The recipe fingerprint that produced this content.
    #[inline]
    #[must_use]
    pub const fn recipe_fingerprint(&self) -> &[u8; 32] {
        &self.recipe_fingerprint
    }

    /// The generating run's `instance_id`, if recorded.
    #[inline]
    #[must_use]
    pub const fn generating_run(&self) -> Option<[u8; 16]> {
        self.generating_run
    }

    /// The pinned corpus this content was derived from, if any.
    #[inline]
    #[must_use]
    pub const fn dataset_id(&self) -> Option<DatasetId> {
        self.dataset_id
    }

    /// The (bounded) echo of the producing run's corpus lineage.
    #[inline]
    #[must_use]
    pub fn corpus_lineage(&self) -> &[MoteId] {
        &self.corpus_lineage
    }
}

/// A content-versioning publish fact: the mutable-handle → immutable-content
/// mapping (D82, D88), with an append-only `prior` edge to this handle's previous
/// version (the lineage chain) and advisory [`Provenance`].
///
/// All fields are private; the only constructors are [`AssetVersion::root`] and
/// [`AssetVersion::successor`], so `schema_version` is never caller-set and the
/// `revision` is always chain-derived (never a forged or wall-clock value).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AssetVersion {
    /// Canonical-encoding schema version (constructor-set, never caller-set).
    schema_version: u16,
    /// The MUTABLE human handle this publish moves.
    handle: AssetPath,
    /// The IMMUTABLE content this version pins.
    content: VersionedContent,
    /// `None` for the first version of this handle; `Some(prior)` = the previous
    /// version of THIS SAME handle (the append-only lineage edge).
    prior: Option<VersionId>,
    /// Monotonic chain position: root = 0, successor = `prior_revision + 1`
    /// (saturating). Derived from the chain, NOT a clock — two byte-identical
    /// publishes share a [`VersionId`].
    revision: u32,
    /// Who published (the governance subject; `Register` is checked in
    /// [`crate::GovernedCatalog`], never here — authority is the fold's business).
    publisher: crate::PartyId,
    /// Advisory provenance (D84) — never gates.
    provenance: Provenance,
}

impl AssetVersion {
    /// The FIRST version of a handle (`prior = None`, `revision = 0`).
    #[must_use]
    pub fn root(
        handle: AssetPath,
        content: VersionedContent,
        publisher: crate::PartyId,
        provenance: Provenance,
    ) -> Self {
        Self {
            schema_version: CATALOG_VERSION_SCHEMA_VERSION,
            handle,
            content,
            prior: None,
            revision: 0,
            publisher,
            provenance,
        }
    }

    /// "Update" a handle by publishing a NEW version pinning `content` (D-LOCK-4 —
    /// the prior version is retained; the handle is moved by the ledger). The
    /// `prior` and `prior_revision` args name the version this one supersedes. The
    /// ledger's `publish` RE-VERIFIES them against the real chain — the prior must
    /// be present, on the SAME handle, and `revision == prior.revision + 1` — and
    /// refuses a mismatch (so a forged `prior` or an inflated `prior_revision` can
    /// never land). `revision` here is the chain-derived `prior_revision + 1`
    /// (saturating), folded into the id; it is never trusted for authority.
    #[must_use]
    pub fn successor(
        prior: VersionId,
        prior_revision: u32,
        handle: AssetPath,
        content: VersionedContent,
        publisher: crate::PartyId,
        provenance: Provenance,
    ) -> Self {
        Self {
            schema_version: CATALOG_VERSION_SCHEMA_VERSION,
            handle,
            content,
            prior: Some(prior),
            revision: prior_revision.saturating_add(1),
            publisher,
            provenance,
        }
    }

    /// The mutable handle this version was published at.
    #[inline]
    #[must_use]
    pub fn handle(&self) -> &AssetPath {
        &self.handle
    }

    /// The immutable content this version pins.
    #[inline]
    #[must_use]
    pub fn content(&self) -> &VersionedContent {
        &self.content
    }

    /// The prior version of this handle (`None` for the root).
    #[inline]
    #[must_use]
    pub fn prior(&self) -> Option<VersionId> {
        self.prior
    }

    /// The chain-derived revision number (root = 0).
    #[inline]
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// Who published this version.
    #[inline]
    #[must_use]
    pub fn publisher(&self) -> &crate::PartyId {
        &self.publisher
    }

    /// The advisory provenance.
    #[inline]
    #[must_use]
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// The canonical-encoding schema version this version was built under.
    #[inline]
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// The content-addressed identity: `blake3(b"kx-catalog/asset-version/v1" ‖
    /// canonical_bincode(self))`. Pure; two byte-identical versions share an id.
    #[must_use]
    pub fn version_id(&self) -> VersionId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-catalog/asset-version/v1");
        // SAFETY (expect): an AssetVersion composes a u16, an AssetPath (strings),
        // a VersionedContent ([u8;32]-bearing newtypes), Option<VersionId> ([u8;32]),
        // a u32, a PartyId (string), and a Provenance ([u8;32] / [u8;16] /
        // Option<DatasetId> / Vec<MoteId> — all bytes) — none of which carry a float
        // or a non-encodable variant, so canonical bincode encoding is infallible.
        // Mirrors the documented `Grant::grant_id` / `TaskSignature::task_signature_hash`.
        let body = bincode::serde::encode_to_vec(self, canonical_config()).expect(
            "AssetVersion canonical encoding is infallible (no floats, no non-encodable types)",
        );
        h.update(&body);
        VersionId(*h.finalize().as_bytes())
    }
}
