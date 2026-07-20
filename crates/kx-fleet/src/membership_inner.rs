// SPDX-License-Identifier: Apache-2.0
//! The shared membership fold: the append-only [`Inner`] truth + derived indices, the
//! single fold step ([`Inner::apply_fact`]), and the `pub(crate)` read folds the
//! [`crate::InMemoryMembershipLedger`] AND the durable [`crate::SqliteMembershipLedger`]
//! both delegate to — so the in-memory and replayed-from-disk views can never diverge
//! by construction (mirrors `kx_catalog`'s `in_memory_ledger`).
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

use kx_catalog::{CatalogAction, CatalogActionSet, PartyId};

use crate::ledger::{MemberRole, TeamEdge, MAX_TEAM_MEMBERS_WALK};
use crate::membership::{MembershipFact, MembershipId};

/// The append-only truth + derived indices.
///
/// `pub(crate)` so the durable [`crate::SqliteMembershipLedger`] holds the SAME
/// `Inner` and shares the SAME fold/read/apply logic (no in-memory-vs-replayed
/// divergence by construction). Fields stay private to this module; cross-module
/// callers use [`Inner::apply_fact`] (write) + the `pub(crate)` read functions below.
#[derive(Debug, Default)]
pub(crate) struct Inner {
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

impl Inner {
    /// Apply an already-validated, non-duplicate fact: assign it the next append
    /// position, update the derived indices, and push it onto the log. The SINGLE
    /// fold step — used by BOTH the in-memory append (after its conflict/dedup gate)
    /// and the durable rebuild (replaying the persisted log in `seq` order), so the
    /// two backends can never diverge.
    pub(crate) fn apply_fact(&mut self, fact: MembershipFact) {
        let pos = self.facts.len();
        let fid = fact.fact_id();
        match &fact {
            MembershipFact::Found(team) => {
                self.owners
                    .insert(team.team().clone(), team.owner().clone());
                self.foundings.insert(team.team().clone(), fid);
            }
            MembershipFact::Admit(a) => {
                self.admits_by_team_member
                    .entry((a.team().clone(), a.member().clone()))
                    .or_default()
                    .push(pos);
                self.teams_by_member
                    .entry(a.member().clone())
                    .or_default()
                    .insert(a.team().clone());
                self.members_by_team
                    .entry(a.team().clone())
                    .or_default()
                    .insert(a.member().clone());
            }
            MembershipFact::Remove(r) => {
                // Record the removal POSITION; the fold honors it only for admits at
                // an earlier position (time-ordered — a later re-admit survives).
                self.removed
                    .entry((r.team().clone(), r.member().clone()))
                    .or_default()
                    .push(pos);
            }
            MembershipFact::Disband(d) => {
                self.disbanded
                    .entry(d.team().clone())
                    .or_default()
                    .insert(d.by().clone());
            }
        }
        self.by_id.insert(fid, pos);
        self.facts.push(fact);
    }

    /// `true` (returns the stored fact) iff a fact with this content id is already
    /// present — the idempotency/immutability tripwire the in-memory + durable admit/
    /// remove/disband appends consult before applying.
    pub(crate) fn contains_fact(&self, fid: &MembershipId) -> Option<&MembershipFact> {
        self.by_id.get(fid).map(|&pos| &self.facts[pos])
    }

    /// The founded owner of `team`, if any (the founding owner-conflict gate).
    pub(crate) fn owner_of_team_principal(&self, team: &PartyId) -> Option<&PartyId> {
        self.owners.get(team)
    }

    /// The canonical (first) founding id of `team`, if founded (the idempotent
    /// re-founding return value).
    pub(crate) fn canonical_founding(&self, team: &PartyId) -> Option<MembershipId> {
        self.foundings.get(team).copied()
    }

    /// The count of appended facts.
    pub(crate) fn len_facts(&self) -> usize {
        self.facts.len()
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
/// AUTHORIZED removal — a remover that is the team owner OR a party whose admit for
/// `member` carries ACTIVE admit-authority (you may undo what you were entitled to
/// grant). Append-only + time-ordered: a removal at position `Pr` cancels an admit at
/// `Pa` only when `Pr > Pa`, so a fresh re-admit appended AFTER the removal restores
/// access (revoke-by-new-fact, re-admit-by-new-fact — exactly the grant-ledger
/// discipline).
///
/// D232: the `admitters` set is filtered through the SAME [`admitter_authorized`] check
/// [`edges_of`] applies to admits, so ONE predicate governs both what an admit GRANTS
/// and what it AUTHORIZES. Without it, an `Admit` — appendable by anyone, since
/// authority is decided here in the fold and not at construction — let a party
/// self-issue an admit that is INERT as an edge yet still conferred eviction rights
/// over an owner-admitted member (a removal-DoS).
fn admit_is_removed(
    inner: &Inner,
    team: &PartyId,
    member: &PartyId,
    admit_pos: usize,
    depth: usize,
    visiting: &BTreeSet<PartyId>,
) -> bool {
    let key = (team.clone(), member.clone());
    let Some(removals) = inner.removed.get(&key) else {
        return false;
    };
    let owner = inner.owners.get(team);
    // The AUTHORIZED admitters of this edge — a party who issued an admit for it AND
    // holds active admit-authority (owner, or an active `Delegate`-holder). The extra
    // authority resolution runs only when a removal exists for the key (early return
    // above) and reuses the same depth-bounded, `visiting`-guarded walk `edges_of` uses,
    // so it adds no unbounded recursion.
    let admitters: BTreeSet<&PartyId> = inner
        .admits_by_team_member
        .get(&key)
        .into_iter()
        .flatten()
        .filter_map(|&pos| match &inner.facts[pos] {
            MembershipFact::Admit(a) => Some(a.admitter()),
            _ => None,
        })
        .filter(|&a| admitter_authorized(inner, team, a, depth, visiting, member))
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
pub(crate) fn edges_of(
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
            if admit_is_removed(inner, team, member, pos, depth, visiting) {
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

// ---------------------------------------------------------------------------
// Shared read folds (pub(crate)) — the in-memory AND the durable
// `SqliteMembershipLedger` trait impls call these over their respective `Inner`, so
// the read semantics are ONE source of truth (no backend divergence).
// ---------------------------------------------------------------------------

/// The founded owner of `team`, if any.
pub(crate) fn read_owner_of_team(inner: &Inner, team: &PartyId) -> Option<PartyId> {
    inner.owners.get(team).cloned()
}

/// The authority-checked active membership edges `member` holds as a CHILD.
pub(crate) fn read_member_edges(inner: &Inner, member: &PartyId) -> Vec<TeamEdge> {
    edges_of(inner, member, 0, &BTreeSet::new())
}

/// The authority-checked active members of `team` (the additive merged view per
/// member), sorted by member id (BTree iteration order).
pub(crate) fn read_effective_members(inner: &Inner, team: &PartyId) -> Vec<MemberRole> {
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

/// A snapshot of the append-only fact log (append order).
pub(crate) fn snapshot_facts(inner: &Inner) -> Vec<MembershipFact> {
    inner.facts.clone()
}
