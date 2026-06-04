// SPDX-License-Identifier: Apache-2.0
//! The membership-ledger seam (M7, D112): the backend-agnostic [`MembershipLedger`]
//! trait, its query result types ([`TeamEdge`] / [`MemberRole`]), and the
//! [`MembershipOutcome`] append result. The in-memory reference backend is
//! [`crate::InMemoryMembershipLedger`].
//!
//! Like `kx_catalog::GrantLedger`, this is a **separate truth**: authoritative for
//! *who is in a team*, never depending on `kx-journal`. Authorization (admit
//! authority, removal/disband honoring) is computed by the fail-closed fold, never
//! trusted from a fact. The single core query is [`MembershipLedger::member_edges`]
//! — the authority-checked active edges a principal holds as a *member* (child);
//! everything else (membership tests, nested warrant resolution in
//! [`crate::GovernedFleet`]) derives from it.

use std::sync::Arc;

use kx_catalog::{CatalogActionSet, PartyId};
use kx_warrant::Role;

use crate::error::MembershipLedgerError;
use crate::membership::{Admit, Disband, MembershipFact, MembershipId, Removal};
use crate::team::Team;

/// The maximum membership-chain depth the fold / the nested resolution will walk. A
/// chain (admit-delegation OR fleet-of-teams nesting) deeper than this fails closed
/// (conveys nothing) rather than walking unbounded — a hard DoS / stack-growth
/// bound. The walks are iterative + `seen`-guarded, so this caps WORK, never the
/// call stack. Mirrors `kx_catalog::MAX_DELEGATION_DEPTH`.
pub const MAX_TEAM_MEMBERS_WALK: usize = 64;

/// One authority-checked active membership edge: `member ∈ team` under `role` +
/// `action_cap`, established by the admit `admit_id`. The role + cap come from the
/// SAME admit fact — there is no constructor pairing a cap from one admit with a
/// role from another, so the action/warrant decoupling is unrepresentable (mirrors
/// `kx_catalog::GrantWarrant`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TeamEdge {
    team: PartyId,
    role: Role,
    action_cap: CatalogActionSet,
    admit_id: MembershipId,
}

impl TeamEdge {
    /// Internal constructor — only the ledger fold builds these.
    pub(crate) fn new(
        team: PartyId,
        role: Role,
        action_cap: CatalogActionSet,
        admit_id: MembershipId,
    ) -> Self {
        Self {
            team,
            role,
            action_cap,
            admit_id,
        }
    }

    /// The team this edge admits the member into.
    #[inline]
    #[must_use]
    pub fn team(&self) -> &PartyId {
        &self.team
    }

    /// The runtime scope a `Use` along this edge narrows under.
    #[inline]
    #[must_use]
    pub fn role(&self) -> &Role {
        &self.role
    }

    /// The catalog actions this edge conveys.
    #[inline]
    #[must_use]
    pub fn action_cap(&self) -> &CatalogActionSet {
        &self.action_cap
    }

    /// The id of the admit fact establishing this edge (the stable, content-addressed
    /// tie-break key).
    #[inline]
    #[must_use]
    pub fn admit_id(&self) -> MembershipId {
        self.admit_id
    }
}

/// A member's merged effective role in a team — the additive view across all their
/// parallel active admits: `action_cap` is the UNION of conveyed actions (the
/// user-chosen additive model), and `role` is the one from the lexicographically
/// smallest `admit_id` (a stable, content-addressed pick). For sound, action-aligned
/// WARRANT resolution use [`crate::GovernedFleet`], which pairs each action with its
/// own admit's role rather than this merged view.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MemberRole {
    member: PartyId,
    role: Role,
    action_cap: CatalogActionSet,
}

impl MemberRole {
    /// Internal constructor — only the ledger fold builds these.
    pub(crate) fn new(member: PartyId, role: Role, action_cap: CatalogActionSet) -> Self {
        Self {
            member,
            role,
            action_cap,
        }
    }

    /// The member.
    #[inline]
    #[must_use]
    pub fn member(&self) -> &PartyId {
        &self.member
    }

    /// The merged runtime scope (from the smallest `admit_id`).
    #[inline]
    #[must_use]
    pub fn role(&self) -> &Role {
        &self.role
    }

    /// The union of catalog actions conveyed across the member's parallel admits.
    #[inline]
    #[must_use]
    pub fn action_cap(&self) -> &CatalogActionSet {
        &self.action_cap
    }
}

/// The outcome of an append: a fresh insert vs. an idempotent no-op (the fact was
/// byte-identically present). Mirrors `kx_catalog::AppendOutcome`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MembershipOutcome {
    /// First append of this fact.
    Appended(MembershipId),
    /// A byte-identical fact was already present — no-op (idempotent).
    AlreadyPresent(MembershipId),
}

impl MembershipOutcome {
    /// The membership id this outcome refers to.
    #[inline]
    #[must_use]
    pub const fn membership_id(&self) -> MembershipId {
        match self {
            Self::Appended(f) | Self::AlreadyPresent(f) => *f,
        }
    }

    /// `true` iff this was a fresh append (not an idempotent no-op).
    #[inline]
    #[must_use]
    pub const fn is_appended(&self) -> bool {
        matches!(self, Self::Appended(_))
    }
}

/// The membership-ledger seam — backend-agnostic (in-memory now; durable / cloud
/// behind the same trait, D94). A SEPARATE TRUTH from the journal; it never writes
/// the journal. Authorization is computed by the fold, never trusted from a fact.
pub trait MembershipLedger {
    /// Found a team (genesis). Idempotent on an identical founding;
    /// [`MembershipLedgerError::OwnerConflict`] if the team is already founded with
    /// a different owner.
    fn append_founding(&self, team: Team) -> Result<MembershipOutcome, MembershipLedgerError>;

    /// Append an admission. Minimal + idempotent (authority is the fold's business).
    fn append_admit(&self, admit: Admit) -> Result<MembershipOutcome, MembershipLedgerError>;

    /// Append a removal (revoke by new fact). Minimal + idempotent; the remover's
    /// authority is decided by the fold.
    fn append_remove(&self, removal: Removal) -> Result<MembershipOutcome, MembershipLedgerError>;

    /// Append a disband (revoke the team). Minimal + idempotent; the disbander's
    /// authority is decided by the fold.
    fn append_disband(&self, disband: Disband) -> Result<MembershipOutcome, MembershipLedgerError>;

    /// The founded owner of `team`, if any.
    fn owner_of_team(&self, team: &PartyId) -> Option<PartyId>;

    /// The authority-checked active membership edges `member` holds as a CHILD — one
    /// per surviving admit (admitter authorized, not removed, team not disbanded),
    /// sorted by `(team, admit_id)` for determinism. The core query everything else
    /// derives from; nested resolution in [`crate::GovernedFleet`] walks these edges
    /// upward.
    fn member_edges(&self, member: &PartyId) -> Vec<TeamEdge>;

    /// The authority-checked active members of `team` (the additive merged view per
    /// member), sorted by member id. The team-keyed dual of
    /// [`MembershipLedger::member_edges`].
    fn effective_members(&self, team: &PartyId) -> Vec<MemberRole>;

    /// Enumerate every appended fact in append order.
    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = MembershipFact> + 'a>;

    /// Count of appended facts.
    fn len(&self) -> usize;

    /// `true` when no facts are appended.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` iff `member` is a DIRECT active member of `team`. (Transitive
    /// reachability through nested teams is resolved by [`crate::GovernedFleet`].)
    fn is_member(&self, member: &PartyId, team: &PartyId) -> bool {
        self.member_edges(member).iter().any(|e| e.team() == team)
    }

    /// The member's merged effective role in `team` (additive caps; role from the
    /// smallest `admit_id`), or `None` if not an active member.
    fn member_role(&self, member: &PartyId, team: &PartyId) -> Option<MemberRole> {
        let mut edges: Vec<TeamEdge> = self
            .member_edges(member)
            .into_iter()
            .filter(|e| e.team() == team)
            .collect();
        if edges.is_empty() {
            return None;
        }
        // Deterministic: role from the smallest admit_id; cap = union of all.
        edges.sort_by(|a, b| a.admit_id().as_bytes().cmp(b.admit_id().as_bytes()));
        let mut cap = CatalogActionSet::None;
        for e in &edges {
            cap = cap.union(e.action_cap());
        }
        let role = edges[0].role().clone();
        Some(MemberRole::new(member.clone(), role, cap))
    }

    /// The teams `member` is a DIRECT active member of, sorted + de-duplicated.
    fn teams_of(&self, member: &PartyId) -> Vec<PartyId> {
        let mut teams: Vec<PartyId> = self
            .member_edges(member)
            .into_iter()
            .map(|e| e.team().clone())
            .collect();
        teams.sort();
        teams.dedup();
        teams
    }
}

impl<L: MembershipLedger + ?Sized> MembershipLedger for Arc<L> {
    fn append_founding(&self, team: Team) -> Result<MembershipOutcome, MembershipLedgerError> {
        (**self).append_founding(team)
    }
    fn append_admit(&self, admit: Admit) -> Result<MembershipOutcome, MembershipLedgerError> {
        (**self).append_admit(admit)
    }
    fn append_remove(&self, removal: Removal) -> Result<MembershipOutcome, MembershipLedgerError> {
        (**self).append_remove(removal)
    }
    fn append_disband(&self, disband: Disband) -> Result<MembershipOutcome, MembershipLedgerError> {
        (**self).append_disband(disband)
    }
    fn owner_of_team(&self, team: &PartyId) -> Option<PartyId> {
        (**self).owner_of_team(team)
    }
    fn member_edges(&self, member: &PartyId) -> Vec<TeamEdge> {
        (**self).member_edges(member)
    }
    fn effective_members(&self, team: &PartyId) -> Vec<MemberRole> {
        (**self).effective_members(team)
    }
    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = MembershipFact> + 'a> {
        (**self).list_facts()
    }
    fn len(&self) -> usize {
        (**self).len()
    }
    fn is_member(&self, member: &PartyId, team: &PartyId) -> bool {
        (**self).is_member(member, team)
    }
    fn member_role(&self, member: &PartyId, team: &PartyId) -> Option<MemberRole> {
        (**self).member_role(member, team)
    }
    fn teams_of(&self, member: &PartyId) -> Vec<PartyId> {
        (**self).teams_of(member)
    }
}
