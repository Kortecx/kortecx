// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Typed fleet errors (M7, D112): [`MembershipLedgerError`] (append failures) and
//! [`GovernedFleetError`] (composed-resolution failures). Mirrors
//! `kx_catalog::LedgerError` / `kx_catalog::GovernedError`.

use kx_catalog::CatalogAction;
use kx_warrant::NarrowingError;

/// A membership-ledger append failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MembershipLedgerError {
    /// A DIFFERENT fact already exists at this `MembershipId`. Facts are
    /// content-addressed, so the same id MUST mean the same bytes — a mismatch is a
    /// hash-collision tripwire (cryptographically unreachable): refuse loudly rather
    /// than overwrite. Carries the conflicting id (hex).
    #[error("immutable membership ledger conflict at membership_id {0}")]
    ImmutabilityConflict(String),
    /// A team is already founded with a DIFFERENT owner. A team has exactly one
    /// owner; re-founding to a new owner is refused (the founding is genesis).
    #[error("team ownership conflict: {0}")]
    OwnerConflict(String),
    /// A durable-backend storage failure (SQLite open / I/O / schema mismatch /
    /// corrupt row) — only the durable [`crate::SqliteMembershipLedger`] raises it;
    /// the in-memory backend is infallible. Mirrors `kx_catalog::LedgerError::Storage`.
    #[error("membership ledger storage: {0}")]
    Storage(String),
}

/// A composed [`crate::GovernedFleet`] resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GovernedFleetError {
    /// The member is not authorized for `action` on `asset` through any active
    /// (possibly nested) team membership (fail-closed). Returned only by the strict
    /// `require_*` resolver; the non-strict resolver returns `Ok(None)` instead.
    #[error("member {member} not authorized for {action:?} on {asset} via any team")]
    Unauthorized {
        /// The member (display form).
        member: String,
        /// The action that was required.
        action: CatalogAction,
        /// The asset it was required on (display form).
        asset: String,
    },
    /// A membership role proposed a runtime-scope widen at some hop — surfaced by
    /// the FROZEN `kx_warrant::intersect`, never a silently-wider warrant.
    #[error(transparent)]
    Narrowing(#[from] NarrowingError),
}
