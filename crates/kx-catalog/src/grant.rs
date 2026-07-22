// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Grants + revocations (M7.2, D86) — content-addressed, narrowing-only,
//! revoke-by-new-fact.
//!
//! A [`Grant`] issues a grantee a set of [`CatalogActionSet`] catalog actions on
//! an [`AssetRef`], plus a runtime scope ([`Role`]) the grantee's runs narrow
//! under. A grant is either a **root** grant (issued by the asset owner, `prior =
//! None`) or a **delegated** sub-grant (`prior = Some(parent)`). It is
//! content-addressed by [`GrantId`] (`blake3(domain-tag ‖ canonical_bincode)`),
//! exactly the [`crate::TaskSignatureHash`] discipline — two byte-identical
//! grants share an id.
//!
//! Narrowing is NOT re-implemented here: the runtime warrant a grant chain
//! conveys is computed through the FROZEN `kx_warrant::intersect` seam
//! ([`effective_runtime_warrant`] / the ledger fold), so a grant can never widen
//! the granting party's own capability (a widen surfaces as
//! [`NarrowingError`](kx_warrant::NarrowingError), never a silently-wider warrant).
//!
//! A [`Revocation`] is a NEW fact (D-LOCK-4: revoke by new fact, never edit). It
//! is recorded MINIMALLY — append does not check the revoker's authority; the
//! ledger fold honors a revocation ONLY when the revoker is the grant's grantor
//! (you may revoke what you granted) or the asset owner (an owner may revoke any
//! grant on their asset). An unauthorized revocation is a recorded-but-inert
//! fact, exactly as an unauthorized grant conveys nothing.

use kx_warrant::{intersect, NarrowingError, Role, WarrantSpec};
use serde::{Deserialize, Serialize};

use crate::action::CatalogActionSet;
use crate::party::PartyId;
use crate::path::AssetRef;
use crate::signature::canonical_config;

/// A 32-byte BLAKE3 hash — the catalog's identity substrate.
type Hash32 = [u8; 32];

/// Canonical-encoding schema version of a [`Grant`]. Bumped on ANY change to the
/// canonical bytes (the version is a struct field, so it folds into [`GrantId`]).
/// Mirrors [`crate::TASK_SIGNATURE_SCHEMA_VERSION`].
pub const GRANT_SCHEMA_VERSION: u16 = 1;

/// The content-addressed identity of a [`Grant`]:
/// `blake3(b"kx-catalog/grant/v1" ‖ canonical_bincode(grant))`. The domain tag
/// prevents cross-type preimage aliasing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GrantId(pub Hash32);

impl GrantId {
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

impl std::fmt::Debug for GrantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrantId({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

impl std::fmt::Display for GrantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// A capability grant on a catalog asset.
///
/// All fields are private; the only constructors are [`Grant::root`] and
/// [`Grant::delegated`], so `schema_version` is never caller-set and a grant's
/// root/delegated shape is structurally honest (`prior` matches the constructor).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Grant {
    /// Canonical-encoding schema version (constructor-set, never caller-set).
    schema_version: u16,
    /// The asset this grant is on.
    asset: AssetRef,
    /// Who issued the grant: the asset owner (root) or a delegator (delegated).
    grantor: PartyId,
    /// Who receives it.
    grantee: PartyId,
    /// The catalog actions conveyed (bounded by the grantor's own, via the fold).
    actions: CatalogActionSet,
    /// The runtime scope a `Use` runs under — narrowed through `kx_warrant::intersect`.
    runtime_scope: Role,
    /// `None` for a root grant; `Some(parent)` for a delegated sub-grant.
    prior: Option<GrantId>,
}

impl Grant {
    /// A **root** grant issued by an asset owner (`prior = None`). It conveys
    /// authority only when the grantor is in fact the asset's bound owner (the
    /// ledger fold checks this — an off-owner root grant is recorded-but-inert).
    #[must_use]
    pub fn root(
        asset: AssetRef,
        grantor: PartyId,
        grantee: PartyId,
        actions: CatalogActionSet,
        runtime_scope: Role,
    ) -> Self {
        Self {
            schema_version: GRANT_SCHEMA_VERSION,
            asset,
            grantor,
            grantee,
            actions,
            runtime_scope,
            prior: None,
        }
    }

    /// A **delegated** sub-grant (`prior = Some(parent)`). It conveys authority
    /// only when the parent grant conveys `Delegate`, the parent's grantee is
    /// this grant's grantor, and both are on the same asset (the fold checks all
    /// three — a broken chain is recorded-but-inert).
    #[must_use]
    pub fn delegated(
        prior: GrantId,
        asset: AssetRef,
        grantor: PartyId,
        grantee: PartyId,
        actions: CatalogActionSet,
        runtime_scope: Role,
    ) -> Self {
        Self {
            schema_version: GRANT_SCHEMA_VERSION,
            asset,
            grantor,
            grantee,
            actions,
            runtime_scope,
            prior: Some(prior),
        }
    }

    /// The asset this grant is on.
    #[inline]
    #[must_use]
    pub fn asset(&self) -> &AssetRef {
        &self.asset
    }

    /// The grantor (owner for root; delegator for delegated).
    #[inline]
    #[must_use]
    pub fn grantor(&self) -> &PartyId {
        &self.grantor
    }

    /// The grantee.
    #[inline]
    #[must_use]
    pub fn grantee(&self) -> &PartyId {
        &self.grantee
    }

    /// The conveyed catalog actions (before the fold's chain-narrowing).
    #[inline]
    #[must_use]
    pub fn actions(&self) -> &CatalogActionSet {
        &self.actions
    }

    /// The runtime scope (`Role`) a `Use` narrows under.
    #[inline]
    #[must_use]
    pub fn runtime_scope(&self) -> &Role {
        &self.runtime_scope
    }

    /// The parent grant id (`None` for a root grant).
    #[inline]
    #[must_use]
    pub fn prior(&self) -> Option<GrantId> {
        self.prior
    }

    /// The canonical-encoding schema version this grant was built under.
    #[inline]
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// The content-addressed identity: `blake3(b"kx-catalog/grant/v1" ‖
    /// canonical_bincode(self))`. Pure; two byte-identical grants share an id.
    #[must_use]
    pub fn grant_id(&self) -> GrantId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-catalog/grant/v1");
        // SAFETY (expect): a Grant composes AssetRef (strings + [u8;32]), PartyId
        // (string), CatalogActionSet (u8-discriminant enum set), a kx_warrant::Role
        // (strings + u32 + WarrantSpec — integer/bytes only, NO float), u16, and
        // Option<GrantId> ([u8;32]) — none of which carry a float or a
        // non-encodable variant, so canonical bincode encoding is infallible.
        // Mirrors the documented `TaskSignature::task_signature_hash`.
        let body = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("Grant canonical encoding is infallible (no floats, no non-encodable types)");
        h.update(&body);
        GrantId(*h.finalize().as_bytes())
    }
}

/// The content-addressed identity of a [`Revocation`] — equal to
/// [`revocation_idempotency_key`], so two identical revocations dedupe.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RevocationId(pub Hash32);

impl RevocationId {
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

impl std::fmt::Debug for RevocationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RevocationId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

/// The idempotency key (and content id) of a revocation of `grant_id` by
/// `revoker`: `blake3(b"kx-catalog/revocation/v1" ‖ grant_id(32B) ‖ revoker)`.
///
/// The fixed-length `grant_id` precedes the variable-length `revoker` so the
/// byte boundary is unambiguous (no length-extension aliasing). Two identical
/// `(grant_id, revoker)` revocations collapse to one fact.
#[must_use]
pub fn revocation_idempotency_key(grant_id: &GrantId, revoker: &PartyId) -> RevocationId {
    let mut h = blake3::Hasher::new();
    h.update(b"kx-catalog/revocation/v1");
    h.update(grant_id.as_bytes());
    h.update(revoker.as_str().as_bytes());
    RevocationId(*h.finalize().as_bytes())
}

/// A revocation of a grant (D-LOCK-4: revoke by new fact, never edit).
///
/// Private fields; built via [`Revocation::new`]. The schema is pinned by the
/// domain tag in [`revocation_idempotency_key`], so no struct version field is
/// needed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Revocation {
    grant_id: GrantId,
    revoker: PartyId,
}

impl Revocation {
    /// Record an intent to revoke `grant_id` by `revoker`. Authority is decided
    /// by the ledger fold, not here.
    #[must_use]
    pub fn new(grant_id: GrantId, revoker: PartyId) -> Self {
        Self { grant_id, revoker }
    }

    /// The grant being revoked.
    #[inline]
    #[must_use]
    pub fn grant_id(&self) -> GrantId {
        self.grant_id
    }

    /// Who is attempting the revocation.
    #[inline]
    #[must_use]
    pub fn revoker(&self) -> &PartyId {
        &self.revoker
    }

    /// The content-addressed identity (= [`revocation_idempotency_key`]).
    #[must_use]
    pub fn revocation_id(&self) -> RevocationId {
        revocation_idempotency_key(&self.grant_id, &self.revoker)
    }
}

/// One-hop effective warrant: narrow `grantor_effective` (the granting party's
/// own effective warrant) by `grant`'s runtime scope, via the FROZEN
/// `kx_warrant::intersect` seam. The ledger fold applies this at every chain hop.
///
/// # Errors
///
/// Propagates [`NarrowingError`](kx_warrant::NarrowingError) when the grant's
/// runtime scope proposes a widen on any qualitative axis — a grant can never
/// authorize a wider runtime warrant than the granting party holds.
pub fn effective_runtime_warrant(
    grantor_effective: &WarrantSpec,
    grant: &Grant,
) -> Result<WarrantSpec, NarrowingError> {
    intersect(grantor_effective, grant.runtime_scope())
}
