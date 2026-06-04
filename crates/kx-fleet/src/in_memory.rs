// SPDX-License-Identifier: Apache-2.0
//! [`InMemoryMembershipLedger`] — the reference [`MembershipLedger`] backend.
//!
//! An append-only `Vec<MembershipFact>` truth + derived `BTreeMap` indices under a
//! single [`RwLock`]: O(log n) append + per-query lookup, sub-linear at scale,
//! deterministic. Process-local + rebuildable — not for production durability (a
//! persistent backend implements the same trait, D94). It proves
//! [`MembershipLedger`] carries no storage-substrate assumption (the role
//! `kx_catalog::InMemoryGrantLedger` plays for `kx_catalog::GrantLedger`).
//!
//! ## The fold ([`edges_of`])
//!
//! A member's active edges are computed by an iterative, depth-bounded,
//! cycle-guarded fold: for each team the member is admitted to that is not
//! disbanded (by the owner) and from which the member is not removed (by the owner
//! or an admitter), every admit whose admitter has admit-authority (the owner, or an
//! active `Delegate`-holding member — checked recursively, bounded by
//! [`MAX_TEAM_MEMBERS_WALK`] + a `visiting` cycle-guard) becomes a [`TeamEdge`]. A
//! pathologically deep admit-delegation chain caps WORK, never the stack (fail-closed
//! beyond the bound). Mirrors `kx_catalog`'s `fold_chain`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use kx_catalog::{CatalogAction, CatalogActionSet, PartyId};

use crate::error::MembershipLedgerError;
use crate::ledger::{
    MemberRole, MembershipLedger, MembershipOutcome, TeamEdge, MAX_TEAM_MEMBERS_WALK,
};
use crate::membership::{Admit, Disband, MembershipFact, MembershipId, Removal};
use crate::team::Team;

/// The append-only truth + derived indices.
#[derive(Debug, Default)]
struct Inner {
    /// The append-only fact log (the truth; everything else is a derived index).
    facts: Vec<MembershipFact>,
    /// Content id → position in `facts` (idempotency + immutability tripwire).
    by_id: BTreeMap<MembershipId, usize>,
    /// Team principal → owner (genesis foundings).
    owners: BTreeMap<PartyId, PartyId>,
    /// Team principal → its canonical (first) founding id. Genesis is set once; a
    /// re-founding by the same owner is idempotent (first display name wins).
    foundings: BTreeMap<PartyId, MembershipId>,
    /// (team, member) → the positions of `Admit` facts for that edge.
    admits_by_team_member: BTreeMap<(PartyId, PartyId), Vec<usize>>,
    /// Member → the teams it has been admitted to (for `member_edges` / `teams_of`).
    teams_by_member: BTreeMap<PartyId, BTreeSet<PartyId>>,
    /// Team → the members admitted to it (for `effective_members`).
    members_by_team: BTreeMap<PartyId, BTreeSet<PartyId>>,
    /// (team, member) → the `facts` positions of `Removal` facts for that edge. The
    /// fold honors a removal at position `Pr` for an admit at `Pa` only when
    /// `Pr > Pa` (time-ordered: a removal cancels only PRIOR admits, so a fresh
    /// re-admit restores access) AND the remover is AUTHORIZED (owner or an admitter).
    removed: BTreeMap<(PartyId, PartyId), Vec<usize>>,
    /// Team → the parties that have recorded a disband. The fold filters to the owner;
    /// an owner disband is TERMINAL (position-independent — re-founding cannot resurrect).
    disbanded: BTreeMap<PartyId, BTreeSet<PartyId>>,
}

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

/// `true` iff `team` has an AUTHORIZED disband: a disbander that is the team owner.
fn disbanded_authorized(inner: &Inner, team: &PartyId) -> bool {
    match inner.disbanded.get(team) {
        None => false,
        Some(disbanders) => {
            let owner = inner.owners.get(team);
            disbanders.iter().any(|d| Some(d) == owner)
        }
    }
}

/// `true` iff the admit at `admit_pos` for `(team, member)` is cancelled by a LATER
/// AUTHORIZED removal — a remover that is the team owner OR a party who admitted
/// `member` to `team` (you may undo what you granted). Append-only + time-ordered: a
/// removal at position `Pr` cancels an admit at `Pa` only when `Pr > Pa`, so a fresh
/// re-admit appended AFTER the removal restores access (revoke-by-new-fact,
/// re-admit-by-new-fact — exactly the grant-ledger discipline).
fn admit_is_removed(inner: &Inner, team: &PartyId, member: &PartyId, admit_pos: usize) -> bool {
    let key = (team.clone(), member.clone());
    let Some(removals) = inner.removed.get(&key) else {
        return false;
    };
    let owner = inner.owners.get(team);
    // The admitters of this edge (the parties who issued an admit for it).
    let admitters: BTreeSet<&PartyId> = inner
        .admits_by_team_member
        .get(&key)
        .into_iter()
        .flatten()
        .filter_map(|&pos| match &inner.facts[pos] {
            MembershipFact::Admit(a) => Some(a.admitter()),
            _ => None,
        })
        .collect();
    removals.iter().any(|&rpos| {
        rpos > admit_pos
            && match &inner.facts[rpos] {
                MembershipFact::Remove(r) => {
                    Some(r.remover()) == owner || admitters.contains(r.remover())
                }
                _ => false,
            }
    })
}

/// `true` iff `admitter` has admit-authority on `team`: the team owner, OR an active
/// member of `team` whose cap holds [`CatalogAction::Delegate`]. The recursive arm
/// is depth-bounded + `visiting`-guarded (cycle / over-depth → fail-closed).
fn admitter_authorized(
    inner: &Inner,
    team: &PartyId,
    admitter: &PartyId,
    depth: usize,
    visiting: &BTreeSet<PartyId>,
    member: &PartyId,
) -> bool {
    if Some(admitter) == inner.owners.get(team) {
        return true;
    }
    // The admitter must be an active member of `team` with Delegate. Recurse, adding
    // the member currently being resolved to the visiting set to break cycles.
    let mut visiting2 = visiting.clone();
    visiting2.insert(member.clone());
    edges_of(inner, admitter, depth + 1, &visiting2)
        .iter()
        .any(|e| e.team() == team && e.action_cap().contains(CatalogAction::Delegate))
}

/// The authority-checked active membership edges `member` holds as a CHILD. Iterative
/// over the member's teams; the only recursion is the bounded admit-authority check.
fn edges_of(
    inner: &Inner,
    member: &PartyId,
    depth: usize,
    visiting: &BTreeSet<PartyId>,
) -> Vec<TeamEdge> {
    if depth > MAX_TEAM_MEMBERS_WALK || visiting.contains(member) {
        return Vec::new(); // over-depth / cycle → fail-closed
    }
    let Some(teams) = inner.teams_by_member.get(member) else {
        return Vec::new();
    };
    let mut out: Vec<TeamEdge> = Vec::new();
    for team in teams {
        if disbanded_authorized(inner, team) {
            continue; // an owner disband is terminal for the whole team
        }
        let key = (team.clone(), member.clone());
        let Some(positions) = inner.admits_by_team_member.get(&key) else {
            continue;
        };
        for &pos in positions {
            // Time-ordered removal: a later authorized removal cancels this admit,
            // but a re-admit appended after the removal survives.
            if admit_is_removed(inner, team, member, pos) {
                continue;
            }
            let MembershipFact::Admit(admit) = &inner.facts[pos] else {
                continue;
            };
            if !admitter_authorized(inner, team, admit.admitter(), depth, visiting, member) {
                continue;
            }
            out.push(TeamEdge::new(
                team.clone(),
                admit.role().clone(),
                admit.action_cap().clone(),
                admit.admit_id(),
            ));
        }
    }
    // Deterministic order: (team, admit_id).
    out.sort_by(|a, b| {
        (a.team(), a.admit_id().as_bytes()).cmp(&(b.team(), b.admit_id().as_bytes()))
    });
    out
}

/// Merge a member's surviving edges in ONE team into the additive [`MemberRole`]
/// view: cap = union, role = from the smallest `admit_id`. `edges` MUST be the
/// member's edges already filtered to that team and be non-empty.
fn merge_member_role(member: &PartyId, mut edges: Vec<TeamEdge>) -> MemberRole {
    edges.sort_by(|a, b| a.admit_id().as_bytes().cmp(b.admit_id().as_bytes()));
    let mut cap = CatalogActionSet::None;
    for e in &edges {
        cap = cap.union(e.action_cap());
    }
    let role = edges[0].role().clone();
    MemberRole::new(member.clone(), role, cap)
}

impl MembershipLedger for InMemoryMembershipLedger {
    fn append_founding(&self, team: Team) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fid = MembershipId::from_bytes(*team.team_id().as_bytes());
        let team_principal = team.team().clone();
        let owner = team.owner().clone();
        let fact = MembershipFact::Found(Box::new(team));
        let mut guard = self.inner.write().expect("poisoned lock");
        // Genesis is set ONCE per team principal. A re-founding by the SAME owner is
        // idempotent — the FIRST founding is canonical, and a differing display_name is
        // ignored rather than appended as a second genesis fact. A DIFFERENT owner is an
        // OwnerConflict. (Byte-identical re-foundings naturally collapse here too.)
        if let Some(existing_owner) = guard.owners.get(&team_principal) {
            if existing_owner != &owner {
                return Err(MembershipLedgerError::OwnerConflict(format!(
                    "team {team_principal} already founded with a different owner"
                )));
            }
            let canonical = guard.foundings.get(&team_principal).copied().unwrap_or(fid);
            return Ok(MembershipOutcome::AlreadyPresent(canonical));
        }
        let pos = guard.facts.len();
        guard.facts.push(fact);
        guard.by_id.insert(fid, pos);
        guard.owners.insert(team_principal.clone(), owner);
        guard.foundings.insert(team_principal, fid);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn append_admit(&self, admit: Admit) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fid = admit.admit_id();
        let team = admit.team().clone();
        let member = admit.member().clone();
        let fact = MembershipFact::Admit(Box::new(admit));
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(&pos) = guard.by_id.get(&fid) {
            return if guard.facts[pos] == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        let pos = guard.facts.len();
        guard.facts.push(fact);
        guard.by_id.insert(fid, pos);
        guard
            .admits_by_team_member
            .entry((team.clone(), member.clone()))
            .or_default()
            .push(pos);
        guard
            .teams_by_member
            .entry(member.clone())
            .or_default()
            .insert(team.clone());
        guard
            .members_by_team
            .entry(team)
            .or_default()
            .insert(member);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn append_remove(&self, removal: Removal) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fid = removal.removal_id();
        let team = removal.team().clone();
        let member = removal.member().clone();
        let fact = MembershipFact::Remove(Box::new(removal));
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(&pos) = guard.by_id.get(&fid) {
            return if guard.facts[pos] == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        let pos = guard.facts.len();
        guard.facts.push(fact);
        guard.by_id.insert(fid, pos);
        // Record the removal POSITION; the fold honors it only for admits at an
        // earlier position (time-ordered — a later re-admit survives).
        guard.removed.entry((team, member)).or_default().push(pos);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn append_disband(&self, disband: Disband) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fid = disband.disband_id();
        let team = disband.team().clone();
        let by = disband.by().clone();
        let fact = MembershipFact::Disband(Box::new(disband));
        let mut guard = self.inner.write().expect("poisoned lock");
        if let Some(&pos) = guard.by_id.get(&fid) {
            return if guard.facts[pos] == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        let pos = guard.facts.len();
        guard.facts.push(fact);
        guard.by_id.insert(fid, pos);
        guard.disbanded.entry(team).or_default().insert(by);
        Ok(MembershipOutcome::Appended(fid))
    }

    fn owner_of_team(&self, team: &PartyId) -> Option<PartyId> {
        self.inner
            .read()
            .expect("poisoned lock")
            .owners
            .get(team)
            .cloned()
    }

    fn member_edges(&self, member: &PartyId) -> Vec<TeamEdge> {
        let guard = self.inner.read().expect("poisoned lock");
        edges_of(&guard, member, 0, &BTreeSet::new())
    }

    fn effective_members(&self, team: &PartyId) -> Vec<MemberRole> {
        let guard = self.inner.read().expect("poisoned lock");
        let inner: &Inner = &guard;
        let Some(members) = inner.members_by_team.get(team) else {
            return Vec::new();
        };
        let mut out: Vec<MemberRole> = Vec::new();
        for m in members {
            let edges: Vec<TeamEdge> = edges_of(inner, m, 0, &BTreeSet::new())
                .into_iter()
                .filter(|e| e.team() == team)
                .collect();
            if !edges.is_empty() {
                out.push(merge_member_role(m, edges));
            }
        }
        out
    }

    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = MembershipFact> + 'a> {
        let guard = self.inner.read().expect("poisoned lock");
        // Snapshot under the read lock (append order), then release before iterating.
        let facts: Vec<MembershipFact> = guard.facts.clone();
        Box::new(facts.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").facts.len()
    }
}

// Compile-time proof the ledger is shareable across threads (so `Arc<…>` works for
// the concurrency tests + a multi-threaded gateway).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InMemoryMembershipLedger>();
};
