// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Catalog permissions (M7.2, D86): the closed [`CatalogAction`] vocabulary and
//! the fail-closed, narrowing-only [`CatalogActionSet`].
//!
//! These are the catalog-governance actions, ORTHOGONAL to a `WarrantSpec`'s
//! runtime capabilities (a grant carries BOTH: which catalog actions the grantee
//! may perform, and the runtime warrant a `Use` runs under). [`CatalogActionSet`]
//! mirrors `kx_warrant::SecretScope` — `None` is the fail-closed default and
//! [`CatalogActionSet::narrow`] is set-intersection, so a delegated grant can
//! never name an action the delegator did not hold (laundering is impossible by
//! construction).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A single catalog-governance action. A closed `#[repr(u8)]` enum — growing it
/// is a deliberate [`crate::GRANT_SCHEMA_VERSION`] bump (the variant set folds
/// into a grant's content hash).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum CatalogAction {
    /// May read the asset's catalog metadata / discover it.
    Read = 0,
    /// May invoke the asset (a fresh registered run inherits the grant warrant).
    Use = 1,
    /// May register / publish a new version under the asset's namespace.
    Register = 2,
    /// May delegate a (narrowed) sub-grant of this asset to another party.
    Delegate = 3,
}

impl CatalogAction {
    /// The canonical full set of every action — the owner's complete authority.
    /// MUST enumerate every variant (a drift guard in the proptests fails to
    /// compile if a variant is added without being listed here).
    #[must_use]
    pub fn all() -> BTreeSet<Self> {
        [Self::Read, Self::Use, Self::Register, Self::Delegate]
            .into_iter()
            .collect()
    }
}

/// The set of catalog actions a grant conveys.
///
/// `None` is the fail-closed default (conveys nothing). `AllowList(S)` conveys
/// exactly `S`. There is ONE canonical deny representation: [`CatalogActionSet::allow`]
/// normalizes an empty allow-list to `None`, so `None` and `AllowList({})` can
/// never both occur (illegal duplicate-state unrepresentable).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub enum CatalogActionSet {
    /// Conveys no action (fail-closed default).
    #[default]
    None,
    /// Conveys exactly the listed actions (never empty — see [`CatalogActionSet::allow`]).
    AllowList(BTreeSet<CatalogAction>),
}

impl CatalogActionSet {
    /// The full-authority set (every [`CatalogAction`]).
    #[must_use]
    pub fn all() -> Self {
        Self::AllowList(CatalogAction::all())
    }

    /// Build from an iterator of actions. An empty set normalizes to [`Self::None`]
    /// (the single canonical deny representation).
    #[must_use]
    pub fn allow(actions: impl IntoIterator<Item = CatalogAction>) -> Self {
        let set: BTreeSet<CatalogAction> = actions.into_iter().collect();
        if set.is_empty() {
            Self::None
        } else {
            Self::AllowList(set)
        }
    }

    /// `true` iff `self` conveys no more than `parent`: `None ⊆ anything`;
    /// `AllowList(_) ⊄ None`; `AllowList(c) ⊆ AllowList(p)` iff `c ⊆ p`.
    #[must_use]
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        match (self, parent) {
            (Self::None, _) => true,
            (Self::AllowList(_), Self::None) => false,
            (Self::AllowList(c), Self::AllowList(p)) => c.is_subset(p),
        }
    }

    /// Set-intersection narrowing: the actions a delegate may convey are bounded
    /// by what the delegator held. Monotone — the result is `⊆` both operands,
    /// so an action absent from either is unrepresentable in the result.
    #[must_use]
    pub fn narrow(&self, cap: &Self) -> Self {
        match (self, cap) {
            (Self::None, _) | (_, Self::None) => Self::None,
            (Self::AllowList(a), Self::AllowList(b)) => Self::allow(a.intersection(b).copied()),
        }
    }

    /// Set-union: combine the actions conveyed by two grants the same party
    /// holds on one asset (authorization is additive across grants).
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::None, x) | (x, Self::None) => x.clone(),
            (Self::AllowList(a), Self::AllowList(b)) => Self::allow(a.union(b).copied()),
        }
    }

    /// `true` iff `action` is conveyed.
    #[must_use]
    pub fn contains(&self, action: CatalogAction) -> bool {
        match self {
            Self::None => false,
            Self::AllowList(s) => s.contains(&action),
        }
    }

    /// `true` iff this set conveys nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }
}
