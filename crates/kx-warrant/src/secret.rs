//! [`SecretRef`] (opaque secret identifier) + [`SecretScope`] (the warrant axis
//! authorizing which secrets a role may resolve, D110.3). Mirrors
//! [`crate::NetScope`]: the warrant carries *identifiers*, never *values* — the
//! resolver (`SecretStore`, at the transport, M5.3 PR-B) is the only thing that
//! turns a `SecretRef` into a secret. `is_subset_of` gives monotonic narrowing.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// An opaque secret identifier the warrant may authorize resolution of.
///
/// Stored as an opaque string so the warrant layer never reimplements secret
/// naming or holds a secret value (the resolver interprets it — D81/D110). The
/// same ref-not-value discipline as [`crate::NetScope`]'s [`crate::Host`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SecretRef(pub String);

/// Which secret references a role/warrant may resolve (D110.3).
///
/// Qualitative, subset-narrowed — the exact shape of [`crate::NetScope`].
/// `None` authorizes **no** secret resolution at all (the fail-closed default).
/// `AllowList(S)` authorizes exactly the refs in `S`. Narrowing respects
/// monotonic subset: `None ⊆ anything`; `AllowList(C) ⊆ None` is false;
/// `AllowList(C) ⊆ AllowList(P)` iff `C ⊆ P`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SecretScope {
    /// No secret may be resolved under this warrant (fail-closed default).
    #[default]
    None,
    /// Exactly the listed secret references may be resolved.
    AllowList(BTreeSet<SecretRef>),
}

impl SecretScope {
    /// `true` iff `self` authorizes no more secrets than `parent`.
    #[must_use]
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        match (self, parent) {
            (Self::None, _) => true,
            (Self::AllowList(_), Self::None) => false,
            (Self::AllowList(child), Self::AllowList(parent)) => child.is_subset(parent),
        }
    }
}
