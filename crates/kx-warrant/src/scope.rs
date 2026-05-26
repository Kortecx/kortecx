//! [`FsScope`] (per-path mount-and-mode map) + [`NetScope`] (egress
//! allowlist / `None`). Both implement `is_subset_of` for monotonic narrowing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::classes::FsMode;
use crate::fields::Host;

/// Filesystem scope: a mapping from mount points to access modes.
///
/// Intersection is **set-intersection** on the path keys, with per-path mode
/// intersection on the values (see [`FsMode::is_subset_of`]). A child may
/// reference only paths the parent also references; a child's mode at any
/// path must be `is_subset_of` the parent's mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FsScope {
    /// Mount points and their access modes. `BTreeMap` for canonical
    /// iteration order (bincode-canonical encoding depends on this).
    pub mounts: BTreeMap<PathBuf, FsMode>,
}

impl FsScope {
    /// Construct an empty `FsScope` (no mounts; no filesystem access).
    #[inline]
    #[must_use]
    pub fn empty() -> Self {
        Self {
            mounts: BTreeMap::new(),
        }
    }

    /// `true` iff `self` is no wider than `parent`: every path of `self` is a
    /// path of `parent`, AND `self`'s mode at that path is a subset of
    /// `parent`'s mode there.
    #[must_use]
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        self.mounts.iter().all(|(path, mode)| {
            parent
                .mounts
                .get(path)
                .is_some_and(|p_mode| mode.is_subset_of(*p_mode))
        })
    }
}

/// Network egress scope.
///
/// `None` blocks all egress. `EgressAllowlist({h1, h2})` permits egress to
/// exactly the listed hosts. Intersection respects monotonic narrowing:
/// `None ∩ anything = None`; `EgressAllowlist(C) ∩ EgressAllowlist(P) = C`
/// only when `C ⊆ P` (else widening, refused).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetScope {
    /// No egress permitted.
    None,
    /// Egress permitted to exactly the listed hosts.
    EgressAllowlist(BTreeSet<Host>),
}

impl NetScope {
    /// `true` iff `self` permits no more egress than `parent`.
    #[must_use]
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        match (self, parent) {
            (Self::None, _) => true,
            (Self::EgressAllowlist(_), Self::None) => false,
            (Self::EgressAllowlist(child), Self::EgressAllowlist(parent)) => {
                child.is_subset(parent)
            }
        }
    }
}
