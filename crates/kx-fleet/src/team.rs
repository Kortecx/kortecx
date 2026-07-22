// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The [`Team`] founding record + its content-addressed [`TeamId`] (M7, D112).
//!
//! A team is a group [`PartyId`]; [`Team`] is the genesis fact that establishes a
//! team's **owner** (the only party whose admit conveys root authority). It is
//! content-addressed by [`TeamId`] (`blake3(b"kx-fleet/team/v1" ‖
//! canonical_bincode(team))`), exactly the [`kx_catalog::Grant`] / `AssetBinding`
//! discipline — two byte-identical teams share an id. All fields are private; the
//! only constructor is [`Team::found`], so `schema_version` is never caller-set.

use kx_catalog::{canonical_config, PartyId};
use serde::{Deserialize, Serialize};

/// A 32-byte BLAKE3 hash — the fleet layer's identity substrate.
type Hash32 = [u8; 32];

/// Canonical-encoding schema version of a [`Team`] / membership fact. Bumped on ANY
/// change to the canonical bytes (the version is a struct field, so it folds into a
/// fact's content id). Mirrors [`kx_catalog::GRANT_SCHEMA_VERSION`].
pub const FLEET_SCHEMA_VERSION: u16 = 1;

/// The content-addressed identity of a [`Team`]:
/// `blake3(b"kx-fleet/team/v1" ‖ canonical_bincode(team))`. The domain tag prevents
/// cross-type preimage aliasing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TeamId(pub Hash32);

impl TeamId {
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

impl std::fmt::Debug for TeamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TeamId({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

impl std::fmt::Display for TeamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// A team founding record — the genesis fact binding a team's principal to its
/// owner.
///
/// All fields are private; the only constructor is [`Team::found`], so
/// `schema_version` is never caller-set. The `team` principal is the group
/// [`PartyId`] grants are issued to (a team that owns/uses a recipe); `owner` is the
/// founding admin whose admit conveys root authority (the fold checks this — an
/// off-owner admit is recorded-but-inert).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Team {
    /// Canonical-encoding schema version (constructor-set, never caller-set).
    schema_version: u16,
    /// The team's own group principal (the grantee of a team grant).
    principal: PartyId,
    /// The founding admin — the only party whose ROOT admit conveys authority.
    owner: PartyId,
    /// Free-form human handle. NEVER parsed for enforcement.
    display_name: String,
}

impl Team {
    /// Found a team named by the group principal `team`, owned by `owner`.
    #[must_use]
    pub fn found(principal: PartyId, owner: PartyId, display_name: impl Into<String>) -> Self {
        Self {
            schema_version: FLEET_SCHEMA_VERSION,
            principal,
            owner,
            display_name: display_name.into(),
        }
    }

    /// The team's group principal.
    #[inline]
    #[must_use]
    pub fn team(&self) -> &PartyId {
        &self.principal
    }

    /// The founding owner.
    #[inline]
    #[must_use]
    pub fn owner(&self) -> &PartyId {
        &self.owner
    }

    /// The human display name (advisory; never parsed).
    #[inline]
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The canonical-encoding schema version this team was built under.
    #[inline]
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// The content-addressed identity: `blake3(b"kx-fleet/team/v1" ‖
    /// canonical_bincode(self))`. Pure; two byte-identical teams share an id.
    #[must_use]
    pub fn team_id(&self) -> TeamId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-fleet/team/v1");
        // SAFETY (expect): a Team is PartyId (string) + PartyId (string) + String +
        // u16 — no floats, no non-encodable variant; canonical bincode encoding is
        // infallible (mirrors `kx_catalog::Grant::grant_id`).
        let body = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("Team canonical encoding is infallible (no floats, no non-encodable types)");
        h.update(&body);
        TeamId(*h.finalize().as_bytes())
    }
}
