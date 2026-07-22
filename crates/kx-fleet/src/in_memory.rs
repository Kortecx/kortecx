// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`InMemoryMembershipLedger`] — the reference [`MembershipLedger`] backend.
//!
//! An append-only `Vec<MembershipFact>` truth + derived `BTreeMap` indices under a
//! single [`RwLock`]: O(log n) append + per-query lookup, sub-linear at scale,
//! deterministic. Process-local + rebuildable — not for production durability (the
//! durable [`crate::SqliteMembershipLedger`] implements the SAME trait over the SAME
//! [`Inner`] fold, D94). It proves [`MembershipLedger`] carries no storage-substrate
//! assumption (the role `kx_catalog::InMemoryGrantLedger` plays for
//! `kx_catalog::GrantLedger`).
//!
//! The fold + the derived indices live in [`crate::membership_inner`]; this module
//! is only the in-memory append gate (owner-conflict / idempotency / immutability)
//! before [`Inner::apply_fact`], and the read delegation to the shared `read_*` folds.

use std::sync::RwLock;

use kx_catalog::PartyId;

use crate::error::MembershipLedgerError;
use crate::ledger::{MemberRole, MembershipLedger, MembershipOutcome, TeamEdge};
use crate::membership::{Admit, Disband, MembershipFact, MembershipId, Removal};
use crate::membership_inner::{
    read_effective_members, read_member_edges, read_owner_of_team, snapshot_facts, Inner,
};
use crate::team::Team;

/// An ephemeral, process-local [`MembershipLedger`]. Multiple readers, one writer.
///
/// # Examples
///
/// ```
/// use kx_fleet::{InMemoryMembershipLedger, MembershipLedger, Team, Admit};
/// use kx_catalog::{CatalogAction, CatalogActionSet, PartyId};
/// use kx_warrant::{Role, WarrantSpec};
///
/// let fleet = InMemoryMembershipLedger::new();
/// let team = PartyId::new("team:sre@acme");
/// let owner = PartyId::new("admin@acme");
/// let alice = PartyId::new("alice@acme");
///
/// fleet.append_founding(Team::found(team.clone(), owner.clone(), "SRE")).unwrap();
/// let role = Role { name: "oncall".into(), version: 1, spec: WarrantSpec::default(), description: String::new() };
/// fleet.append_admit(Admit::new(
///     team.clone(), alice.clone(), owner.clone(), role,
///     CatalogActionSet::allow([CatalogAction::Use]),
/// )).unwrap();
///
/// assert!(fleet.is_member(&alice, &team));
/// assert!(!fleet.is_member(&PartyId::new("mallory"), &team));
/// ```
#[derive(Debug, Default)]
pub struct InMemoryMembershipLedger {
    inner: RwLock<Inner>,
}

impl InMemoryMembershipLedger {
    /// Construct an empty ledger.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl MembershipLedger for InMemoryMembershipLedger {
    fn append_founding(&self, team: Team) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fid = MembershipId::from_bytes(*team.team_id().as_bytes());
        let team_principal = team.team().clone();
        let owner = team.owner().clone();
        let mut guard = self.inner.write().expect("poisoned lock");
        // Genesis is set ONCE per team principal. A re-founding by the SAME owner is
        // idempotent — the FIRST founding is canonical, and a differing display_name is
        // ignored rather than appended as a second genesis fact. A DIFFERENT owner is an
        // OwnerConflict. (Byte-identical re-foundings naturally collapse here too.)
        if let Some(existing_owner) = guard.owner_of_team_principal(&team_principal) {
            if existing_owner != &owner {
                return Err(MembershipLedgerError::OwnerConflict(format!(
                    "team {team_principal} already founded with a different owner"
                )));
            }
            let canonical = guard.canonical_founding(&team_principal).unwrap_or(fid);
            return Ok(MembershipOutcome::AlreadyPresent(canonical));
        }
        guard.apply_fact(MembershipFact::Found(Box::new(team)));
        Ok(MembershipOutcome::Appended(fid))
    }

    fn append_admit(&self, admit: Admit) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fact = MembershipFact::Admit(Box::new(admit));
        let fid = fact.fact_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(existing) = guard.contains_fact(&fid) {
            return if *existing == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        guard.apply_fact(fact);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn append_remove(&self, removal: Removal) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fact = MembershipFact::Remove(Box::new(removal));
        let fid = fact.fact_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(existing) = guard.contains_fact(&fid) {
            return if *existing == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        guard.apply_fact(fact);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn append_disband(&self, disband: Disband) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fact = MembershipFact::Disband(Box::new(disband));
        let fid = fact.fact_id();
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(existing) = guard.contains_fact(&fid) {
            return if *existing == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        guard.apply_fact(fact);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn owner_of_team(&self, team: &PartyId) -> Option<PartyId> {
        read_owner_of_team(&self.inner.read().expect("poisoned lock"), team)
    }

    fn member_edges(&self, member: &PartyId) -> Vec<TeamEdge> {
        read_member_edges(&self.inner.read().expect("poisoned lock"), member)
    }

    fn effective_members(&self, team: &PartyId) -> Vec<MemberRole> {
        read_effective_members(&self.inner.read().expect("poisoned lock"), team)
    }

    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = MembershipFact> + 'a> {
        let facts = snapshot_facts(&self.inner.read().expect("poisoned lock"));
        Box::new(facts.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").len_facts()
    }
}

// Compile-time proof the ledger is shareable across threads (so `Arc<…>` works for
// the concurrency tests + a multi-threaded gateway).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InMemoryMembershipLedger>();
};
