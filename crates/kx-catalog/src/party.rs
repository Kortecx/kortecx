// SPDX-License-Identifier: Apache-2.0
//! [`PartyId`] — the opaque identity a grant is issued to / by (M7.2, D86).
//!
//! A `PartyId` is an opaque handle the catalog **never parses**: authorization
//! is decided by byte-equality of the handle against the grant facts, never by
//! interpreting its structure. Real principal resolution (a user, a service, a
//! team, a tenant) lives BEHIND the trait in the cloud identity layer
//! (`PrincipalResolver`, D94) — keeping it a plain string lets the cloud map it
//! without reshaping this type, and keeps this crate off `kx-content`.

use serde::{Deserialize, Serialize};

/// The opaque identity of a grantor or grantee. Equality-only; never parsed.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PartyId(String);

impl PartyId {
    /// Wrap an opaque identity handle.
    #[inline]
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying handle bytes.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PartyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
