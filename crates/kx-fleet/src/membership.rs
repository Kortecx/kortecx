// SPDX-License-Identifier: Apache-2.0
//! The append-only membership fact vocabulary (M7, D112): [`Admit`], [`Removal`],
//! [`Disband`], and the [`MembershipFact`] union — content-addressed,
//! narrowing-only, revoke-by-new-fact.
//!
//! Each fact is content-addressed by [`MembershipId`] (`blake3(domain-tag ‖
//! canonical_bincode(payload))`), exactly the [`kx_catalog::Grant`] /
//! `Revocation` discipline. Two byte-identical facts share an id (idempotent
//! append); a same-id-different-bytes append is a hash-collision tripwire refused
//! loudly. Canonical bincode length-prefixes every string, so concatenating two
//! [`PartyId`]s in a key can never alias (the catalog's manual
//! fixed-length-before-variable rule is unnecessary here — the encoding is
//! self-delimiting).
//!
//! An [`Admit`] carries BOTH a [`CatalogActionSet`] action-cap (which catalog
//! actions the membership conveys) AND a [`Role`] runtime scope (the warrant a
//! `Use` narrows under) — EXACTLY a delegated [`kx_catalog::Grant`], which carries
//! both `actions` + `runtime_scope`. The cap + role are paired on the SAME fact, so
//! action/warrant decoupling is unrepresentable.

use kx_catalog::{canonical_config, CatalogActionSet, PartyId};
use kx_warrant::Role;
use serde::{Deserialize, Serialize};

use crate::team::{Team, FLEET_SCHEMA_VERSION};

/// A 32-byte BLAKE3 hash.
type Hash32 = [u8; 32];

/// Domain-tagged content id of a fleet fact payload:
/// `blake3(domain_tag ‖ canonical_bincode(value))`. Pure + infallible (the payload
/// types carry no floats and no non-encodable variants).
fn content_id<T: Serialize>(domain_tag: &[u8], value: &T) -> Hash32 {
    let mut h = blake3::Hasher::new();
    h.update(domain_tag);
    // SAFETY (expect): every fleet fact payload composes PartyId (string),
    // CatalogActionSet (u8-discriminant enum set), kx_warrant::Role (strings + u32 +
    // WarrantSpec — integer/bytes/bools only, NO float), and u16 — none of which
    // carry a float or a non-encodable variant, so canonical bincode encode is
    // infallible (mirrors `kx_catalog::Grant::grant_id`).
    let body = bincode::serde::encode_to_vec(value, canonical_config())
        .expect("fleet fact canonical encoding is infallible (no floats, no non-encodable types)");
    h.update(&body);
    *h.finalize().as_bytes()
}

/// The content-addressed identity of a [`MembershipFact`] — equal to the content id
/// of the fact's payload. The per-payload domain tags (`team` / `admit` / `removal`
/// / `disband`) keep cross-kind ids distinct. The ledger dedups + enforces
/// immutability by this id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MembershipId(pub Hash32);

impl MembershipId {
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

impl std::fmt::Debug for MembershipId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MembershipId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

/// Admit `member` to `team` under a runtime `role` + catalog `action_cap`, issued by
/// `admitter`.
///
/// Conveys membership ONLY when the fold finds `admitter` is the team owner OR an
/// active member of `team` whose cap holds [`kx_catalog::CatalogAction::Delegate`]
/// (an off-authority admit is recorded-but-inert, mirroring an off-owner root
/// grant). Private fields; built via [`Admit::new`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Admit {
    schema_version: u16,
    team: PartyId,
    member: PartyId,
    admitter: PartyId,
    role: Role,
    action_cap: CatalogActionSet,
}

impl Admit {
    /// Record an admission of `member` to `team` by `admitter`, under `role` +
    /// `action_cap`. Authority is decided by the ledger fold, not here.
    #[must_use]
    pub fn new(
        team: PartyId,
        member: PartyId,
        admitter: PartyId,
        role: Role,
        action_cap: CatalogActionSet,
    ) -> Self {
        Self {
            schema_version: FLEET_SCHEMA_VERSION,
            team,
            member,
            admitter,
            role,
            action_cap,
        }
    }

    /// The team admitted into.
    #[inline]
    #[must_use]
    pub fn team(&self) -> &PartyId {
        &self.team
    }

    /// The admitted member (may itself be a team principal — nesting).
    #[inline]
    #[must_use]
    pub fn member(&self) -> &PartyId {
        &self.member
    }

    /// Who issued the admission.
    #[inline]
    #[must_use]
    pub fn admitter(&self) -> &PartyId {
        &self.admitter
    }

    /// The runtime scope a member's `Use` narrows under.
    #[inline]
    #[must_use]
    pub fn role(&self) -> &Role {
        &self.role
    }

    /// The catalog actions this membership conveys.
    #[inline]
    #[must_use]
    pub fn action_cap(&self) -> &CatalogActionSet {
        &self.action_cap
    }

    /// The canonical-encoding schema version this admit was built under.
    #[inline]
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// The content-addressed identity:
    /// `blake3(b"kx-fleet/admit/v1" ‖ canonical_bincode(self))`.
    #[must_use]
    pub fn admit_id(&self) -> MembershipId {
        MembershipId(content_id(b"kx-fleet/admit/v1", self))
    }
}

/// Remove `member` from `team` (revoke-by-new-fact, D-LOCK-4), recorded by
/// `remover`.
///
/// A removal is **member-level + time-ordered**: it cancels every active admit of
/// `(team, member)` that PRECEDES it in the log (so a fresh re-admit appended AFTER
/// the removal restores access — revoke-by-new-fact, re-admit-by-new-fact, exactly
/// the grant-ledger discipline). It is honored by the fold ONLY when `remover` is the
/// team owner OR a party who admitted `member` to `team` (you may undo what you, or a
/// fellow admitter, granted). An unauthorized remover's fact is recorded-but-inert.
/// Removal only ever REDUCES access (it can never escalate), so the owner-or-admitter
/// authority is intentionally permissive. Private fields; built via [`Removal::new`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Removal {
    schema_version: u16,
    team: PartyId,
    member: PartyId,
    remover: PartyId,
}

impl Removal {
    /// Record an intent to remove `member` from `team` by `remover`. Authority is
    /// decided by the ledger fold, not here.
    #[must_use]
    pub fn new(team: PartyId, member: PartyId, remover: PartyId) -> Self {
        Self {
            schema_version: FLEET_SCHEMA_VERSION,
            team,
            member,
            remover,
        }
    }

    /// The team the member is removed from.
    #[inline]
    #[must_use]
    pub fn team(&self) -> &PartyId {
        &self.team
    }

    /// The member being removed.
    #[inline]
    #[must_use]
    pub fn member(&self) -> &PartyId {
        &self.member
    }

    /// Who is attempting the removal.
    #[inline]
    #[must_use]
    pub fn remover(&self) -> &PartyId {
        &self.remover
    }

    /// The content-addressed identity:
    /// `blake3(b"kx-fleet/removal/v1" ‖ canonical_bincode(self))`.
    #[must_use]
    pub fn removal_id(&self) -> MembershipId {
        MembershipId(content_id(b"kx-fleet/removal/v1", self))
    }
}

/// Disband `team` (revoke-the-team), recorded by `by`.
///
/// Honored by the fold ONLY when `by` is the team owner — every membership in a
/// disbanded team goes inert. Private fields; built via [`Disband::new`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Disband {
    schema_version: u16,
    team: PartyId,
    by: PartyId,
}

impl Disband {
    /// Record an intent to disband `team` by `by`. Authority is decided by the
    /// ledger fold, not here.
    #[must_use]
    pub fn new(team: PartyId, by: PartyId) -> Self {
        Self {
            schema_version: FLEET_SCHEMA_VERSION,
            team,
            by,
        }
    }

    /// The team being disbanded.
    #[inline]
    #[must_use]
    pub fn team(&self) -> &PartyId {
        &self.team
    }

    /// Who is attempting the disband.
    #[inline]
    #[must_use]
    pub fn by(&self) -> &PartyId {
        &self.by
    }

    /// The content-addressed identity:
    /// `blake3(b"kx-fleet/disband/v1" ‖ canonical_bincode(self))`.
    #[must_use]
    pub fn disband_id(&self) -> MembershipId {
        MembershipId(content_id(b"kx-fleet/disband/v1", self))
    }
}

/// An append-only membership fact. A closed enum — there is no untyped fact. The
/// large variants are boxed so the enum stays pointer-sized.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MembershipFact {
    /// A team founding record (genesis: establishes the team owner).
    Found(Box<Team>),
    /// An admission of a member to a team under a role + cap.
    Admit(Box<Admit>),
    /// A removal of a member from a team (revoke by new fact, D-LOCK-4).
    Remove(Box<Removal>),
    /// A disband of a team (revoke the team).
    Disband(Box<Disband>),
}

impl MembershipFact {
    /// The content-addressed id of this fact (= the payload's content id). The
    /// per-payload domain tags keep cross-kind ids distinct.
    #[must_use]
    pub fn fact_id(&self) -> MembershipId {
        match self {
            Self::Found(t) => MembershipId(*t.team_id().as_bytes()),
            Self::Admit(a) => a.admit_id(),
            Self::Remove(r) => r.removal_id(),
            Self::Disband(d) => d.disband_id(),
        }
    }
}
