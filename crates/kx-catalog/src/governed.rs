// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`GovernedCatalog`] (M7.2) — the production governed surface over the
//! [`crate::VersionLedger`], composed with the [`crate::GrantLedger`].
//!
//! The raw [`crate::VersionLedger::publish`] is the ungoverned mechanism (authority
//! is the fold's business, exactly like [`crate::GrantLedger::append_grant`]).
//! `GovernedCatalog` is the surface production code uses: **publishing a new
//! version requires a [`CatalogAction::Register`] grant on the handle**, and the
//! governed reads require [`CatalogAction::Read`]. The two ledgers are composed
//! WITHOUT coupling their impls (Rule 1): neither references the other's concrete
//! type — the facade holds both and gates one against the other.
//!
//! ## What gates what (D84 / SN-8)
//!
//! The ONLY thing that gates a publish is the live `Register` grant fold. A
//! version's provenance/lineage is ADVISORY and NEVER gates a publish or any
//! runtime action — the `Read` gate on the governed read methods is plain
//! access-control on *viewing* the catalog, a distinct concern from "advisory
//! data never gates an action". Because the gate consults the LIVE grant ledger,
//! revoking a party's `Register` (a new [`crate::Revocation`] fact) immediately
//! blocks their future publishes; already-published versions are retained forever
//! (D-LOCK-4).

use crate::action::CatalogAction;
use crate::ledger::GrantLedger;
use crate::party::PartyId;
use crate::path::{AssetPath, AssetRef};
use crate::version::{AssetVersion, VersionId, VersionedContent};
use crate::version_ledger::{PublishOutcome, VersionLedger, VersionLedgerError};

/// A governed-catalog operation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GovernedError {
    /// The party lacks the required [`CatalogAction`] on the asset (fail-closed).
    #[error("unauthorized {action:?} on {asset}")]
    Unauthorized {
        /// The action that was required.
        action: CatalogAction,
        /// The asset it was required on (display form).
        asset: String,
    },
    /// The underlying version ledger refused the publish.
    #[error(transparent)]
    Ledger(#[from] VersionLedgerError),
}

/// The governed catalog surface: a [`GrantLedger`] + a [`VersionLedger`], composed
/// so that publishing requires `Register` and viewing requires `Read`.
///
/// Generic over both backends (zero-cost); pass `Arc<InMemoryGrantLedger>` /
/// `Arc<InMemoryVersionLedger>` (both impl their trait for `Arc<L>`) when the
/// underlying ledgers must be shared with other holders.
#[derive(Debug, Default)]
pub struct GovernedCatalog<G: GrantLedger, V: VersionLedger> {
    grants: G,
    versions: V,
}

impl<G: GrantLedger, V: VersionLedger> GovernedCatalog<G, V> {
    /// Compose a grant ledger and a version ledger into a governed surface.
    #[must_use]
    pub fn new(grants: G, versions: V) -> Self {
        Self { grants, versions }
    }

    /// Borrow the underlying grant ledger (e.g. to seed bindings/grants, or to
    /// query authorization directly).
    #[inline]
    #[must_use]
    pub fn grants(&self) -> &G {
        &self.grants
    }

    /// Borrow the underlying version ledger (e.g. for ungated discovery reads,
    /// which are off the commit/audit path).
    #[inline]
    #[must_use]
    pub fn versions(&self) -> &V {
        &self.versions
    }

    /// Publish a new version, **requiring `Register`** on the version's handle.
    ///
    /// # Errors
    ///
    /// [`GovernedError::Unauthorized`] if the publisher lacks `Register` on the
    /// handle (nothing is appended); [`GovernedError::Ledger`] on the version
    /// ledger's immutability tripwire.
    pub fn publish(&self, version: AssetVersion) -> Result<PublishOutcome, GovernedError> {
        let asset = AssetRef::Path(version.handle().clone());
        if !self
            .grants
            .is_authorized(version.publisher(), &asset, CatalogAction::Register)
        {
            return Err(GovernedError::Unauthorized {
                action: CatalogAction::Register,
                asset: asset.to_string(),
            });
        }
        Ok(self.versions.publish(version)?)
    }

    /// Resolve a handle to its current content, **requiring `Read`**.
    ///
    /// # Errors
    ///
    /// [`GovernedError::Unauthorized`] if `party` lacks `Read` on the handle.
    pub fn resolve(
        &self,
        party: &PartyId,
        handle: &AssetPath,
    ) -> Result<Option<(VersionedContent, VersionId)>, GovernedError> {
        self.require_read(party, handle)?;
        Ok(self.versions.resolve(handle))
    }

    /// The version history of a handle, **requiring `Read`**.
    ///
    /// # Errors
    ///
    /// [`GovernedError::Unauthorized`] if `party` lacks `Read` on the handle.
    pub fn history(
        &self,
        party: &PartyId,
        handle: &AssetPath,
    ) -> Result<Vec<AssetVersion>, GovernedError> {
        self.require_read(party, handle)?;
        Ok(self.versions.history(handle))
    }

    /// The ancestor lineage of a version, **requiring `Read`** on that version's
    /// handle. An absent version returns `Ok([])` (it gates nothing — there is no
    /// handle to gate on, and lineage is advisory).
    ///
    /// # Errors
    ///
    /// [`GovernedError::Unauthorized`] if `party` lacks `Read` on the version's
    /// handle.
    pub fn lineage(
        &self,
        party: &PartyId,
        id: &VersionId,
    ) -> Result<Vec<AssetVersion>, GovernedError> {
        let Some(v) = self.versions.get_version(id) else {
            return Ok(Vec::new());
        };
        self.require_read(party, v.handle())?;
        Ok(self.versions.lineage(id))
    }

    /// The forward descendants of a version, **requiring `Read`** on that version's
    /// handle (symmetry with [`Self::lineage`]). An absent version returns
    /// `Ok([])`. NOTE: an absent vs. present-but-unauthorized version is
    /// distinguishable (an existence signal); acceptable because a `VersionId` is a
    /// content hash — not enumerable/guessable — and the data is advisory (D84).
    ///
    /// # Errors
    ///
    /// [`GovernedError::Unauthorized`] if `party` lacks `Read` on the version's
    /// handle.
    pub fn descendants(
        &self,
        party: &PartyId,
        id: &VersionId,
    ) -> Result<Vec<VersionId>, GovernedError> {
        let Some(v) = self.versions.get_version(id) else {
            return Ok(Vec::new());
        };
        self.require_read(party, v.handle())?;
        Ok(self.versions.descendants(id))
    }

    /// `Read`-gate helper.
    fn require_read(&self, party: &PartyId, handle: &AssetPath) -> Result<(), GovernedError> {
        let asset = AssetRef::Path(handle.clone());
        if self
            .grants
            .is_authorized(party, &asset, CatalogAction::Read)
        {
            Ok(())
        } else {
            Err(GovernedError::Unauthorized {
                action: CatalogAction::Read,
                asset: asset.to_string(),
            })
        }
    }
}
