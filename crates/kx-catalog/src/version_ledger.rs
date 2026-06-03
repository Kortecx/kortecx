// SPDX-License-Identifier: Apache-2.0
//! The version-ledger seam (M7.2, D82/D88): the backend-agnostic [`VersionLedger`]
//! trait — content-versioning handles + the provenance/lineage fold. The
//! in-memory reference backend is [`crate::InMemoryVersionLedger`]; the production
//! governed surface (publish requires `Register`) is [`crate::GovernedCatalog`].
//!
//! ## A separate truth, the same D-LOCK-4 discipline
//!
//! Like the M7.1 [`crate::CatalogRegistry`] and the M7.2 [`crate::GrantLedger`],
//! this is authoritative for *what versions exist* and does NOT depend on
//! `kx-journal` (the dependency direction is the wall). D88's "journaled fact" is
//! realized as the DISCIPLINE — append-only, content-addressed, immutable,
//! idempotent — inside the ledger. A durable / cloud backend (D94) implements the
//! SAME trait; the in-memory backend is rebuildable, not durable.
//!
//! ## Lineage is computed, never stored; advisory, never gating
//!
//! [`VersionLedger::lineage`] / [`VersionLedger::descendants`] are pure folds over
//! the `prior` edges — re-derived on every query, never materialized (the
//! `kx_projection::transitive_consumers` pattern, replicated WITHOUT a
//! `kx-projection` dependency). They are advisory (D84): they never gate a publish
//! or any runtime action. The ONLY publish gate is the `Register` grant check in
//! [`crate::GovernedCatalog`]; the raw [`VersionLedger::publish`] here is the
//! minimal mechanism (authority is the facade's business).

use std::sync::Arc;

use crate::path::AssetPath;
use crate::version::{AssetVersion, VersionId, VersionedContent};

/// The maximum version-chain depth the lineage/history fold will walk. A chain
/// deeper than this is depth-capped (the fold returns the bounded prefix actually
/// walked — lineage is ADVISORY, so a depth cap cannot escalate anything, unlike
/// the grant fold which returns "conveys nothing" because it gates). A hard
/// DoS / stack-growth bound; the fold is iterative, so this caps work, not the
/// stack. Larger than [`crate::MAX_DELEGATION_DEPTH`] because a long-lived asset's
/// publish history legitimately grows far deeper than a delegation chain.
pub const MAX_VERSION_CHAIN_DEPTH: usize = 1024;

/// The maximum number of forward descendants [`VersionLedger::descendants`] will
/// enumerate. A hard bound on the BFS work / output size; reaching it stops the
/// walk (a fail-safe DoS bound, never the stack — the walk is iterative).
pub const MAX_VERSION_DESCENDANTS: usize = 65_536;

/// The outcome of a publish: a fresh insert vs. an idempotent no-op (the version
/// fact was byte-identically present).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PublishOutcome {
    /// First publish of this version fact.
    Published(VersionId),
    /// A byte-identical version was already present — no-op (idempotent).
    AlreadyPresent(VersionId),
}

impl PublishOutcome {
    /// The version id this outcome refers to.
    #[inline]
    #[must_use]
    pub const fn version_id(&self) -> VersionId {
        match self {
            Self::Published(v) | Self::AlreadyPresent(v) => *v,
        }
    }

    /// `true` iff this was a fresh publish (not an idempotent no-op).
    #[inline]
    #[must_use]
    pub const fn is_published(&self) -> bool {
        matches!(self, Self::Published(_))
    }
}

/// A version-ledger publish failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionLedgerError {
    /// A DIFFERENT version already exists at this [`VersionId`]. Versions are
    /// content-addressed, so the same id MUST mean the same bytes — a mismatch is
    /// a hash-collision tripwire (cryptographically unreachable): refuse loudly
    /// rather than overwrite. Carries the conflicting id (hex).
    #[error("immutable version conflict at version_id {0}")]
    ImmutabilityConflict(String),
    /// A successor's `prior` version is not present. Publishing is causally
    /// ordered — a version's prior must already be published (in practice a
    /// successor's `prior` id is only obtainable by first publishing the prior).
    /// Carries the missing prior id (hex).
    #[error("prior version {0} not found (publish the prior first)")]
    PriorNotFound(String),
    /// A successor's declared lineage is inconsistent with its REAL prior: the
    /// prior is on a DIFFERENT handle, or the revision is not exactly
    /// `prior.revision + 1`. Refused fail-closed so the stored chain is always
    /// well-formed — a forged cross-handle graft or an inflated revision can never
    /// land, which keeps the handle-move rank ungameable and the forward
    /// (descendants) and backward (lineage) folds in agreement.
    #[error("invalid lineage for version {version_id}: {reason}")]
    InvalidLineage {
        /// The offending version id (hex).
        version_id: String,
        /// Why the lineage was refused.
        reason: String,
    },
}

/// The backend-agnostic content-versioning + lineage ledger.
///
/// Publishing is intentionally MINIMAL + ungoverned here (idempotent + immutable);
/// the `Register`-gated production surface is [`crate::GovernedCatalog`]. Resolving
/// a handle returns its CURRENT immutable content (the mutable handle → immutable
/// version mapping); lineage/descendants are advisory folds.
pub trait VersionLedger {
    /// Append a publish fact (the mechanism). MINIMAL governance-wise — the
    /// `Register` gate is [`crate::GovernedCatalog`]'s business (mirroring how
    /// [`crate::GrantLedger::append_grant`] is minimal) — but lineage-strict: a
    /// successor's `prior` must be present, on the SAME handle, and exactly one
    /// revision below, so the stored chain is always well-formed. Idempotent +
    /// immutable on the content id. Moves the handle to this version iff it ranks
    /// ahead of the current latest (higher `revision`, ties broken by `version_id`
    /// bytes — a total, deterministic, insertion-order-independent order; because
    /// `revision` is validated against the real prior it cannot be inflated to game
    /// the move).
    ///
    /// # Errors
    ///
    /// [`VersionLedgerError::ImmutabilityConflict`] on the cryptographically
    /// unreachable same-id-different-bytes tripwire;
    /// [`VersionLedgerError::PriorNotFound`] if a successor's prior is not yet
    /// published; [`VersionLedgerError::InvalidLineage`] if the prior is on a
    /// different handle or the revision is not `prior.revision + 1`.
    fn publish(&self, version: AssetVersion) -> Result<PublishOutcome, VersionLedgerError>;

    /// Resolve a handle to its CURRENT immutable content + the resolving version
    /// id (the latest publish on that path). `None` if never published. This IS
    /// "the mutable handle resolving to an immutable content-addressed version".
    fn resolve(&self, handle: &AssetPath) -> Option<(VersionedContent, VersionId)>;

    /// Fetch one version by its content id (the EXACT-selection pin, D87 — a
    /// recipient pins an immutable version, never a fuzzy match). `None` if absent.
    fn get_version(&self, id: &VersionId) -> Option<AssetVersion>;

    /// The ancestor provenance chain of a version (walk `prior`, latest → oldest),
    /// depth-bounded + cycle-safe + lineage-integrity-checked, fail-closed.
    /// COMPUTED, never stored. ADVISORY (D84) — never gates.
    ///
    /// Cost note: returns owned [`AssetVersion`]s, so it clones up to
    /// [`MAX_VERSION_CHAIN_DEPTH`] nodes (each carrying a [`crate::Provenance`]
    /// whose `corpus_lineage` is bounded by [`crate::MAX_PROVENANCE_LINEAGE`]).
    /// Both bounds are hard, so the cost is bounded; a future ids-only / borrowed
    /// view is a seam-level optimization (its own PR, Rule 1).
    fn lineage(&self, id: &VersionId) -> Vec<AssetVersion>;

    /// Forward lineage: every version that (transitively) descends from `id` via
    /// `prior`. BFS-with-visited, bounded by [`MAX_VERSION_DESCENDANTS`]. COMPUTED,
    /// never stored. ADVISORY (D84) — never gates.
    fn descendants(&self, id: &VersionId) -> Vec<VersionId>;

    /// The full version chain of a handle (latest → oldest). Default: resolve the
    /// handle, then walk its lineage (each version's `prior` is the same handle's
    /// previous version, so the lineage of the latest IS the handle's history).
    fn history(&self, handle: &AssetPath) -> Vec<AssetVersion> {
        match self.resolve(handle) {
            Some((_, vid)) => self.lineage(&vid),
            None => Vec::new(),
        }
    }

    /// Enumerate every published version in append order.
    fn list_versions<'a>(&'a self) -> Box<dyn Iterator<Item = AssetVersion> + 'a>;

    /// Count of published versions.
    fn len(&self) -> usize;

    /// `true` when no versions are published.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<L: VersionLedger + ?Sized> VersionLedger for Arc<L> {
    fn publish(&self, version: AssetVersion) -> Result<PublishOutcome, VersionLedgerError> {
        (**self).publish(version)
    }

    fn resolve(&self, handle: &AssetPath) -> Option<(VersionedContent, VersionId)> {
        (**self).resolve(handle)
    }

    fn get_version(&self, id: &VersionId) -> Option<AssetVersion> {
        (**self).get_version(id)
    }

    fn lineage(&self, id: &VersionId) -> Vec<AssetVersion> {
        (**self).lineage(id)
    }

    fn descendants(&self, id: &VersionId) -> Vec<VersionId> {
        (**self).descendants(id)
    }

    fn list_versions<'a>(&'a self) -> Box<dyn Iterator<Item = AssetVersion> + 'a> {
        (**self).list_versions()
    }

    fn len(&self) -> usize {
        (**self).len()
    }
}
