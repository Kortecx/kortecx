// SPDX-License-Identifier: Apache-2.0
//! [`GovernedFleet`] (M7, D112) — the composed resolution over a
//! [`MembershipLedger`] + a [`kx_catalog::GrantLedger`].
//!
//! A team is a `PartyId`, and "team `T` may `Use` recipe `A` under warrant `W`" is an
//! ordinary catalog grant. `GovernedFleet` answers the question the fleet layer
//! exists for: **what warrant does member `M` get when invoking recipe `A`, through
//! their (possibly nested) team membership?** It walks the membership edges upward
//! from `M` (bounded, cycle-guarded), and at every reachable team `T` that holds the
//! action on `A` it narrows `T`'s grant warrant by the chain of membership roles via
//! the FROZEN [`kx_warrant::intersect`] — so the result is structurally `⊆` the
//! team's grant (a member can never exceed the team), and `⊆` `owner_root` (the
//! grant fold already guarantees that). The two ledgers are composed WITHOUT coupling
//! their impls (Rule 1, exactly `kx_catalog::GovernedCatalog`).

use std::collections::BTreeSet;

use kx_catalog::{canonical_config, CatalogAction, CatalogActionSet, GrantLedger, PartyId};
use kx_warrant::{intersect, Role, WarrantSpec};

use crate::error::GovernedFleetError;
use crate::ledger::{MembershipLedger, MAX_TEAM_MEMBERS_WALK};
use kx_catalog::AssetRef;

/// A hard cap on the number of membership edges a single resolution will expand — a
/// DoS backstop for a pathologically dense membership DAG (where the count of simple
/// paths could otherwise be exponential in the depth bound). Generous: a real fleet
/// is shallow + narrow and never approaches it. Hitting it yields a fail-closed
/// partial result (the most-permissive warrant found within budget). `MAX² ` so a
/// full `MAX`-deep, `MAX`-wide frontier is still covered.
const MAX_RESOLUTION_STEPS: usize = MAX_TEAM_MEMBERS_WALK * MAX_TEAM_MEMBERS_WALK;

/// The composed governed-fleet surface: a [`MembershipLedger`] + a
/// [`kx_catalog::GrantLedger`], composed so a member's effective warrant on a recipe
/// is resolved through their team membership.
///
/// Generic over both backends (zero-cost); pass `Arc<InMemory…>` (both impl their
/// trait for `Arc<L>`) when the underlying ledgers must be shared with other holders.
#[derive(Debug, Default)]
pub struct GovernedFleet<M: MembershipLedger, G: GrantLedger> {
    members: M,
    grants: G,
}

impl<M: MembershipLedger, G: GrantLedger> GovernedFleet<M, G> {
    /// Compose a membership ledger and a grant ledger into a governed surface.
    #[must_use]
    pub fn new(members: M, grants: G) -> Self {
        Self { members, grants }
    }

    /// Borrow the underlying membership ledger (e.g. to seed teams/admits, or to
    /// query membership directly).
    #[inline]
    #[must_use]
    pub fn members(&self) -> &M {
        &self.members
    }

    /// Borrow the underlying grant ledger (e.g. to seed bindings/team grants).
    #[inline]
    #[must_use]
    pub fn grants(&self) -> &G {
        &self.grants
    }

    /// The runtime warrant `member` gets when invoking `action` on `asset` through
    /// their (possibly nested) team membership: the MOST-PERMISSIVE
    /// `intersect(team_warrant, …membership roles…)` among all active membership
    /// paths whose cap conveys `action` and whose top team holds `action` on `asset`.
    /// `Ok(None)` if no such path exists (fail-closed + action-aligned).
    ///
    /// # Errors
    ///
    /// Propagates [`GovernedFleetError::Narrowing`] if a membership role proposes a
    /// runtime-scope widen at any hop (a member can never widen the team's warrant).
    pub fn resolve_member_warrant(
        &self,
        member: &PartyId,
        asset: &AssetRef,
        action: CatalogAction,
        owner_root: &WarrantSpec,
    ) -> Result<Option<WarrantSpec>, GovernedFleetError> {
        // Collect every path-candidate warrant, then select the most-permissive with a
        // stable, content-addressed tie-break (order-independent — mirrors
        // `kx_catalog`'s `most_permissive`).
        let mut candidates: Vec<([u8; 32], WarrantSpec)> = Vec::new();
        // DFS frame: (principal, role-chain member→here, accumulated cap, seen teams).
        let mut stack: Vec<(PartyId, Vec<Role>, CatalogActionSet, BTreeSet<PartyId>)> = vec![(
            member.clone(),
            Vec::new(),
            CatalogActionSet::all(),
            BTreeSet::new(),
        )];
        let mut steps = 0usize;
        while let Some((principal, roles, cap, seen)) = stack.pop() {
            if roles.len() >= MAX_TEAM_MEMBERS_WALK {
                continue; // depth bound → fail-closed (stop deepening this path)
            }
            for edge in self.members.member_edges(&principal) {
                if steps >= MAX_RESOLUTION_STEPS {
                    break; // DoS budget exhausted → fail-closed partial result
                }
                steps += 1;
                let team = edge.team();
                if seen.contains(team) {
                    continue; // cycle guard
                }
                let new_cap = cap.narrow(edge.action_cap());
                let mut new_roles = roles.clone();
                new_roles.push(edge.role().clone());

                // Candidate: does this team hold `action` on `asset`, and does the
                // path cap still convey it?
                if new_cap.contains(action) {
                    if let Some(team_warrant) = self
                        .grants
                        .resolve_effective_warrant_for(team, asset, action, owner_root)?
                    {
                        // Narrow the team's warrant by the role chain, outermost edge
                        // first (reverse of member→top), via the FROZEN seam.
                        let mut w = team_warrant;
                        for r in new_roles.iter().rev() {
                            w = intersect(&w, r)?;
                        }
                        candidates.push((path_anchor(team, &new_roles), w));
                    }
                }

                // Continue upward (this team may itself be a member of a fleet).
                let mut seen2 = seen.clone();
                seen2.insert(team.clone());
                stack.push((team.clone(), new_roles, new_cap, seen2));
            }
            if steps >= MAX_RESOLUTION_STEPS {
                break;
            }
        }
        Ok(most_permissive(candidates))
    }

    /// Like [`GovernedFleet::resolve_member_warrant`] but fail-closed to a typed
    /// [`GovernedFleetError::Unauthorized`] instead of `Ok(None)`.
    ///
    /// # Errors
    ///
    /// [`GovernedFleetError::Unauthorized`] if no active membership path conveys
    /// `action` on `asset`; [`GovernedFleetError::Narrowing`] on a role widen.
    pub fn require_member_warrant(
        &self,
        member: &PartyId,
        asset: &AssetRef,
        action: CatalogAction,
        owner_root: &WarrantSpec,
    ) -> Result<WarrantSpec, GovernedFleetError> {
        match self.resolve_member_warrant(member, asset, action, owner_root)? {
            Some(w) => Ok(w),
            None => Err(GovernedFleetError::Unauthorized {
                member: member.to_string(),
                action,
                asset: asset.to_string(),
            }),
        }
    }

    /// `true` iff `member` may perform `action` on `asset` through some active
    /// (possibly nested) team membership — the team holds `action` AND the membership
    /// path's cap conveys it. Pure boolean, fail-closed; does not require an
    /// `owner_root` (no warrant is resolved).
    #[must_use]
    pub fn is_member_authorized(
        &self,
        member: &PartyId,
        asset: &AssetRef,
        action: CatalogAction,
    ) -> bool {
        let mut stack: Vec<(PartyId, CatalogActionSet, BTreeSet<PartyId>)> =
            vec![(member.clone(), CatalogActionSet::all(), BTreeSet::new())];
        let mut steps = 0usize;
        while let Some((principal, cap, seen)) = stack.pop() {
            if seen.len() >= MAX_TEAM_MEMBERS_WALK {
                continue;
            }
            for edge in self.members.member_edges(&principal) {
                if steps >= MAX_RESOLUTION_STEPS {
                    return false;
                }
                steps += 1;
                let team = edge.team();
                if seen.contains(team) {
                    continue;
                }
                let new_cap = cap.narrow(edge.action_cap());
                if new_cap.contains(action) && self.grants.is_authorized(team, asset, action) {
                    return true;
                }
                let mut seen2 = seen.clone();
                seen2.insert(team.clone());
                stack.push((team.clone(), new_cap, seen2));
            }
        }
        false
    }
}

/// A stable, content-addressed tie-break key for a membership path:
/// `blake3(b"kx-fleet/path/v1" ‖ canonical_bincode((team, roles)))`. The `(team,
/// roles)` tuple is encoded as ONE canonical bincode value so the team string is
/// length-delimited from the role chain (no concatenation-boundary aliasing).
fn path_anchor(team: &PartyId, roles: &[Role]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"kx-fleet/path/v1");
    // SAFETY (expect): PartyId is a string and a Role is strings + u32 + WarrantSpec
    // (integer/bytes/bools, NO float) — canonical bincode encode is infallible
    // (mirrors the fact content ids).
    let body = bincode::serde::encode_to_vec((team, roles), canonical_config())
        .expect("path anchor canonical encoding is infallible (no floats, no non-encodable types)");
    h.update(&body);
    *h.finalize().as_bytes()
}

/// Select the MOST-PERMISSIVE warrant among `candidates`, breaking ties / incomparable
/// maxima by the lexicographically-smallest path anchor (stable, content-addressed).
/// Never synthesizes a warrant — each candidate is already
/// `intersect(team_warrant, …roles…) ⊆ team_warrant ⊆ owner_root`, so this can neither
/// escalate past the team nor compose axes across paths. `None` when empty. Mirrors
/// `kx_catalog`'s `most_permissive`.
fn most_permissive(mut candidates: Vec<([u8; 32], WarrantSpec)>) -> Option<WarrantSpec> {
    candidates.sort_by(|x, y| x.0.cmp(&y.0));
    let mut best: Option<&([u8; 32], WarrantSpec)> = None;
    for c in &candidates {
        best = match best {
            None => Some(c),
            // `best ⊆ c` ⇒ c is at least as permissive ⇒ prefer c. Otherwise keep best
            // (smaller anchor, since `candidates` is sorted).
            Some(b) if warrant_within(&b.1, &c.1) => Some(c),
            Some(b) => Some(b),
        };
    }
    best.map(|(_, w)| w.clone())
}

/// `true` iff warrant `a` conveys NO MORE capability than `b` on every axis
/// (`a ⊆ b`). A local mirror of `kx_catalog`'s `pub(crate) warrant_within` (which is
/// not exported); the destructure of `a` is the drift guard — adding a `WarrantSpec`
/// axis fails to compile here, forcing this comparison to be revisited rather than
/// silently ignoring the new axis. (Promoting `warrant_within` to a public
/// `kx_warrant` primitive is a parked refactor — a STRICT-seam change, its own PR.)
fn warrant_within(a: &WarrantSpec, b: &WarrantSpec) -> bool {
    let WarrantSpec {
        mote_class,
        nd_class,
        fs_scope,
        net_scope,
        syscall_profile_ref,
        tool_grants,
        model_route,
        resource_ceiling,
        environment_ref,
        executor_class,
        secret_scope,
        cost_ceiling,
        tls_required,
    } = a;

    fs_scope.is_subset_of(&b.fs_scope)
        && net_scope.is_subset_of(&b.net_scope)
        && secret_scope.is_subset_of(&b.secret_scope)
        && tool_grants.is_subset(&b.tool_grants)
        && model_route.is_within(&b.model_route)
        && resource_ceiling.cpu_milli <= b.resource_ceiling.cpu_milli
        && resource_ceiling.mem_bytes <= b.resource_ceiling.mem_bytes
        && resource_ceiling.wall_clock_ms <= b.resource_ceiling.wall_clock_ms
        && resource_ceiling.fd_count <= b.resource_ceiling.fd_count
        && resource_ceiling.disk_bytes <= b.resource_ceiling.disk_bytes
        && cost_ceiling.micro_usd <= b.cost_ceiling.micro_usd
        && (*tls_required || !b.tls_required)
        && *mote_class == b.mote_class
        && *nd_class == b.nd_class
        && *syscall_profile_ref == b.syscall_profile_ref
        && *environment_ref == b.environment_ref
        && *executor_class == b.executor_class
}
