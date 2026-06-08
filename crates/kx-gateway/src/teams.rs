//! UI-3 host-side governance views + the demo-team seed.
//!
//! gateway-core stays off `kx-fleet` / `kx-catalog` (the dependency wall), so the
//! concrete, ledger-backed implementations of its read-only [`MembershipView`] /
//! [`GrantView`] seams live HERE, in the host binary. Both are VIEW-only (no fact is
//! ever written by an RPC); the only writes are the idempotent bootstrap
//! [`seed_demo_team`] at serve start. Managing teams/grants across parties + the
//! multi-tenant identity layer are CLOUD (D129).
//!
//! The membership resolve path reuses `kx_fleet::GovernedFleet::resolve_member_warrant`
//! (membership ∩ grant via the FROZEN `kx_warrant::intersect`) over the SAME durable
//! grant ledger the demo recipes seeded — so a member's resolved warrant is the real
//! composed result, never a re-derived approximation.

use std::collections::BTreeSet;
use std::sync::Arc;

use kx_catalog::{
    AssetRef, CatalogAction, CatalogActionSet, Grant, GrantId, GrantLedger, LedgerFact, PartyId,
};
use kx_fleet::{
    Admit, GovernedFleet, MembershipFact, MembershipLedger, SqliteMembershipLedger, Team,
};
use kx_gateway_core::{
    AssetGrantsView, GrantEntry, GrantView, MembershipView, TeamMemberEntry, TeamMembersView,
    TeamSummaryEntry, WarrantProjection,
};
use kx_warrant::{FsScope, NetScope, Role, WarrantSpec};

use crate::error::GatewayError;
use crate::provision::{parse_handle, DemoLibrary};

/// The principal of the demo team the OSS gateway seeds at serve start so the
/// Systems viewer is populated out-of-the-box. A team is a group `PartyId`.
pub const DEMO_TEAM_HANDLE: &str = "kx/teams/demo";

/// The catalog actions, in canonical order, the renderers project to display strings.
const ALL_ACTIONS: [CatalogAction; 4] = [
    CatalogAction::Read,
    CatalogAction::Use,
    CatalogAction::Register,
    CatalogAction::Delegate,
];

/// Project a [`CatalogActionSet`] to its human-readable action names (e.g.
/// `["Read", "Use", "Delegate"]`), in canonical order.
fn render_actions(set: &CatalogActionSet) -> Vec<String> {
    ALL_ACTIONS
        .iter()
        .filter(|a| set.contains(**a))
        .map(|a| format!("{a:?}"))
        .collect()
}

/// Project a [`WarrantSpec`] to a compact, human-readable display — NEVER the
/// warrant body/secret; the headline ceilings + scopes as display strings/scalars
/// (the host renders it once, like the `kx` CLI, so the UI never reconstructs
/// kx-warrant formatting and a future axis bump never forces a proto change).
fn render_warrant(spec: &WarrantSpec) -> WarrantProjection {
    WarrantProjection {
        executor_class: format!("{:?}", spec.executor_class),
        model_route: format!(
            "{} ×{} ({}/{} tok)",
            spec.model_route.model_id.0,
            spec.model_route.max_calls,
            spec.model_route.max_input_tokens,
            spec.model_route.max_output_tokens
        ),
        net_scope: render_net(&spec.net_scope),
        fs_scope: render_fs(&spec.fs_scope),
        max_calls: u64::from(spec.model_route.max_calls),
        cpu_milli: u64::from(spec.resource_ceiling.cpu_milli),
        wall_clock_ms: spec.resource_ceiling.wall_clock_ms,
    }
}

fn render_net(net: &NetScope) -> String {
    match net {
        NetScope::None => "None".to_string(),
        NetScope::EgressAllowlist(hosts) => {
            let list = hosts
                .iter()
                .map(|h| h.0.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("EgressAllowlist({list})")
        }
    }
}

fn render_fs(fs: &FsScope) -> String {
    if fs.mounts.is_empty() {
        return "None".to_string();
    }
    fs.mounts
        .iter()
        .map(|(path, mode)| format!("{}:{mode:?}", path.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A [`MembershipView`] over the durable membership ledger (teams) + the demo
/// library's grant ledger (for the optional resolve). VIEW-only.
pub struct HostMembershipView {
    members: Arc<SqliteMembershipLedger>,
    lib: Arc<DemoLibrary>,
}

impl HostMembershipView {
    /// Compose the membership ledger with the demo library (sharing its grant ledger
    /// + owner-root warrants for the resolve path).
    #[must_use]
    pub fn new(members: Arc<SqliteMembershipLedger>, lib: Arc<DemoLibrary>) -> Self {
        Self { members, lib }
    }
}

impl MembershipView for HostMembershipView {
    fn list_teams(&self) -> Vec<TeamSummaryEntry> {
        // Fold the public `list_facts()` for `Found` facts (one per team, guaranteed
        // by the founding gate) — no trait method needed (mirrors `list_runs`).
        self.members
            .list_facts()
            .filter_map(|f| match f {
                MembershipFact::Found(t) => {
                    let team = t.team().clone();
                    let member_count = u32::try_from(self.members.effective_members(&team).len())
                        .unwrap_or(u32::MAX);
                    Some(TeamSummaryEntry {
                        team_id: team.to_string(),
                        display_name: t.display_name().to_string(),
                        owner: t.owner().to_string(),
                        member_count,
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn list_members(&self, team_id: &str, asset_ref: Option<&str>) -> Option<TeamMembersView> {
        let team = PartyId::new(team_id);
        let owner = self.members.owner_of_team(&team)?;
        // Resolve the (asset, owner_root) context once if an asset_ref was supplied.
        let resolve_ctx = asset_ref
            .and_then(parse_handle)
            .map(AssetRef::Path)
            .and_then(|asset| self.lib.owner_root_for(&asset).map(|root| (asset, root)));
        // The composed fleet (membership ∩ grant) — built once, reused per member.
        let fleet = resolve_ctx
            .as_ref()
            .map(|_| GovernedFleet::new(self.members.clone(), self.lib.grants_arc()));

        let members = self
            .members
            .effective_members(&team)
            .into_iter()
            .map(|m| {
                let resolved_warrant = match (&fleet, &resolve_ctx) {
                    (Some(fleet), Some((asset, root))) => fleet
                        .resolve_member_warrant(m.member(), asset, CatalogAction::Use, root)
                        .ok()
                        .flatten()
                        .map(|w| render_warrant(&w)),
                    _ => None,
                };
                TeamMemberEntry {
                    party: m.member().to_string(),
                    role: m.role().name.clone(),
                    action_caps: render_actions(m.action_cap()),
                    resolved_warrant,
                }
            })
            .collect();

        Some(TeamMembersView {
            owner: owner.to_string(),
            members,
        })
    }
}

/// A [`GrantView`] over the demo library's durable grant ledger (the SAME instance
/// the recipes + the demo team grant seed). Classifies each grant fact root/delegated
/// + active/revoked via an authorized-revocation fold. VIEW-only.
pub struct HostGrantView {
    lib: Arc<DemoLibrary>,
}

impl HostGrantView {
    /// Wrap the demo library (shares its grant ledger).
    #[must_use]
    pub fn new(lib: Arc<DemoLibrary>) -> Self {
        Self { lib }
    }
}

impl GrantView for HostGrantView {
    fn list_asset_grants(&self, asset_ref: &str) -> Option<AssetGrantsView> {
        let asset = parse_handle(asset_ref).map(AssetRef::Path)?;
        let ledger = self.lib.grant_ledger();
        let owner = ledger.owner_of(&asset);

        // One pass over the fact log: the grant facts on this asset + every recorded
        // revocation (the fold below filters to AUTHORIZED revocations).
        let mut grants: Vec<Grant> = Vec::new();
        let mut revocations: Vec<(GrantId, PartyId)> = Vec::new();
        for fact in ledger.list_facts() {
            match fact {
                LedgerFact::Grant(g) if *g.asset() == asset => grants.push(*g),
                LedgerFact::Revoke(r) => revocations.push((r.grant_id(), r.revoker().clone())),
                _ => {}
            }
        }
        // A truly unknown asset (no binding AND no grants) ⇒ None (not_found).
        if owner.is_none() && grants.is_empty() {
            return None;
        }

        let entries = grants
            .into_iter()
            .map(|g| {
                let gid = g.grant_id();
                // Authorized revocation: a revoker that is the grant's grantor (undo
                // what you granted) OR the asset owner (revoke any grant on it).
                let revoked = revocations.iter().any(|(target, by)| {
                    *target == gid && (by == g.grantor() || owner.as_ref() == Some(by))
                });
                GrantEntry {
                    grantor: g.grantor().to_string(),
                    grantee: g.grantee().to_string(),
                    actions: render_actions(g.actions()),
                    runtime_scope: g.runtime_scope().name.clone(),
                    is_root: g.prior().is_none(),
                    revoked,
                }
            })
            .collect();

        Some(AssetGrantsView {
            owner: owner.map(|p| p.to_string()).unwrap_or_default(),
            grants: entries,
        })
    }
}

/// Idempotently seed the demo team at serve start: found `kx/teams/demo` (owner =
/// the gateway principal), admit each `--auth-token` party (+ the dev `local-dev`
/// principal), the FIRST a `Delegate` (role variety), and grant the team `Use`+`Read`
/// on the demo `echo` recipe so a member's warrant resolves through membership ∩
/// grant. Re-running on every restart is a no-op (content-addressed fact dedup +
/// idempotent grant). Mirrors the demo-recipe grant seeding (`provision::seed_recipe`).
///
/// # Errors
/// [`GatewayError::Catalog`] on a membership/grant append failure.
pub fn seed_demo_team(
    members: &SqliteMembershipLedger,
    lib: &DemoLibrary,
    parties: &[String],
) -> Result<(), GatewayError> {
    let cat = |e: String| GatewayError::Catalog(e);
    let owner = DemoLibrary::owner_principal();
    let team = PartyId::new(DEMO_TEAM_HANDLE);
    members
        .append_founding(Team::found(team.clone(), owner.clone(), "Demo Team"))
        .map_err(|e| cat(e.to_string()))?;

    // The admit-role warrant + the team-grant runtime scope == the echo owner-root,
    // so every `intersect` in `resolve_member_warrant` is a no-op narrowing (the
    // member resolves the full team warrant). `WarrantSpec::default()` only if no
    // recipe is provisioned (then the team holds no grant, so the spec is never
    // intersected — harmless).
    let target = lib
        .demo_team_grant_asset()
        .and_then(|asset| lib.owner_root_for(&asset).map(|root| (asset, root)));
    let role_warrant = target
        .as_ref()
        .map_or_else(WarrantSpec::default, |(_, root)| root.clone());

    // Admit each distinct party (the auth-token parties + the dev principal). The
    // member set is sorted (a `BTreeSet`) so the admit order — and the
    // first-as-`Delegate` pick (role variety) — is DETERMINISTIC regardless of the
    // `cfg.auth_tokens` HashMap iteration order.
    let mut admitted: BTreeSet<&str> = parties.iter().map(String::as_str).collect();
    admitted.insert("local-dev");
    for (i, party) in admitted.iter().enumerate() {
        let (role_name, caps) = if i == 0 {
            (
                "demo-delegate",
                CatalogActionSet::allow([
                    CatalogAction::Read,
                    CatalogAction::Use,
                    CatalogAction::Delegate,
                ]),
            )
        } else {
            (
                "demo-member",
                CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
            )
        };
        let role = Role {
            name: role_name.to_string(),
            version: 1,
            spec: role_warrant.clone(),
            description: String::new(),
        };
        members
            .append_admit(Admit::new(
                team.clone(),
                PartyId::new(*party),
                owner.clone(),
                role,
                caps,
            ))
            .map_err(|e| cat(e.to_string()))?;
    }

    // Grant the TEAM `Use`+`Read` on the echo recipe so a member's warrant resolves
    // through membership ∩ grant (the kx-fleet thesis, demonstrated end-to-end). The
    // grant is a root grant from the asset owner (the gateway principal). Idempotent.
    if let Some((asset, owner_root)) = target {
        let role = Role {
            name: "demo-team-use".to_string(),
            version: 1,
            spec: owner_root,
            description: String::new(),
        };
        lib.grant_ledger()
            .append_grant(Grant::root(
                asset,
                owner,
                team,
                CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
                role,
            ))
            .map_err(|e| cat(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision::DEMO_RECIPE_HANDLE;
    use kx_warrant::ExecutorClass;

    /// A demo library + a freshly-seeded demo team in a scratch dir, with two
    /// auth-token parties (alice, bob).
    fn seeded(dir: &std::path::Path) -> (Arc<SqliteMembershipLedger>, Arc<DemoLibrary>) {
        let parties = vec!["alice@acme".to_string(), "bob@acme".to_string()];
        let lib = Arc::new(DemoLibrary::open(dir, ExecutorClass::Bwrap, &parties).unwrap());
        let members = Arc::new(SqliteMembershipLedger::open(dir.join("members.db")).unwrap());
        seed_demo_team(&members, &lib, &parties).unwrap();
        (members, lib)
    }

    #[test]
    fn seed_founds_one_team_with_the_parties_and_a_delegate() {
        let dir = tempfile::tempdir().unwrap();
        let (members, lib) = seeded(dir.path());
        let view = HostMembershipView::new(members, lib);

        let teams = view.list_teams();
        assert_eq!(teams.len(), 1, "exactly one demo team");
        assert_eq!(teams[0].team_id, DEMO_TEAM_HANDLE);
        assert_eq!(teams[0].owner, "kx-gateway");
        // alice + bob + local-dev = 3 members.
        assert_eq!(teams[0].member_count, 3);

        let members = view.list_members(DEMO_TEAM_HANDLE, None).unwrap();
        assert_eq!(members.owner, "kx-gateway");
        assert_eq!(members.members.len(), 3);
        // Exactly one member is a Delegate (role variety).
        let delegates = members
            .members
            .iter()
            .filter(|m| m.action_caps.contains(&"Delegate".to_string()))
            .count();
        assert_eq!(delegates, 1, "exactly one Delegate");
        // No asset_ref ⇒ no resolved warrant.
        assert!(members.members.iter().all(|m| m.resolved_warrant.is_none()));
    }

    #[test]
    fn resolve_member_warrant_populates_only_with_asset_ref() {
        let dir = tempfile::tempdir().unwrap();
        let (members, lib) = seeded(dir.path());
        let view = HostMembershipView::new(members, lib);

        // With the echo asset: each member resolves a warrant through membership ∩
        // grant (the team holds Use on echo; the role chain narrows to the echo
        // owner-root, max_calls 3 — never escalating past the team).
        let with_asset = view
            .list_members(DEMO_TEAM_HANDLE, Some(DEMO_RECIPE_HANDLE))
            .unwrap();
        let resolved = with_asset
            .members
            .iter()
            .find(|m| m.resolved_warrant.is_some())
            .expect("at least one member resolves a warrant");
        let w = resolved.resolved_warrant.as_ref().unwrap();
        assert!(w.max_calls <= 3, "no escalation past the team warrant");
        assert!(!w.executor_class.is_empty());
    }

    #[test]
    fn unknown_team_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let (members, lib) = seeded(dir.path());
        let view = HostMembershipView::new(members, lib);
        assert!(view.list_members("kx/teams/nope", None).is_none());
    }

    #[test]
    fn grant_view_shows_demo_recipe_and_team_grants() {
        let dir = tempfile::tempdir().unwrap();
        let (_members, lib) = seeded(dir.path());
        let view = HostGrantView::new(lib);

        let grants = view.list_asset_grants(DEMO_RECIPE_HANDLE).unwrap();
        assert_eq!(grants.owner, "kx-gateway");
        // The demo-recipe grants (alice, bob, local-dev) + the demo TEAM grant.
        assert!(grants.grants.len() >= 4, "party grants + the team grant");
        // The team grant is present (grantee == the demo team principal), root, active.
        let team_grant = grants
            .grants
            .iter()
            .find(|g| g.grantee == DEMO_TEAM_HANDLE)
            .expect("the demo team is granted Use on echo");
        assert!(team_grant.is_root);
        assert!(!team_grant.revoked);
        assert!(team_grant.actions.contains(&"Use".to_string()));
        // Every seeded grant is active (no revocations seeded).
        assert!(grants.grants.iter().all(|g| !g.revoked));
    }

    #[test]
    fn grant_view_unknown_asset_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let (_members, lib) = seeded(dir.path());
        let view = HostGrantView::new(lib);
        // A well-formed but unbound/unknown handle ⇒ None (not_found at the RPC).
        assert!(view.list_asset_grants("kx/recipes/nope").is_none());
        // A malformed handle ⇒ None.
        assert!(view.list_asset_grants("not-a-handle").is_none());
    }

    #[test]
    fn reseed_demo_team_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let parties = vec!["alice@acme".to_string()];
        let lib = DemoLibrary::open(dir.path(), ExecutorClass::Bwrap, &parties).unwrap();
        let members = SqliteMembershipLedger::open(dir.path().join("members.db")).unwrap();
        seed_demo_team(&members, &lib, &parties).unwrap();
        let before = members.len();
        // Re-seeding on a "restart" (same dir + ledgers) is a no-op.
        seed_demo_team(&members, &lib, &parties).unwrap();
        assert_eq!(
            members.len(),
            before,
            "idempotent re-seed never double-admits"
        );
    }
}
