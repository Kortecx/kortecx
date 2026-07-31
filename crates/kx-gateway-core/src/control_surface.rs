//! The ControlSurface: what each `KxGateway` RPC IS, so a capability cannot go
//! quietly missing.
//!
//! ## The split, and why it is a split
//!
//! A capability facade needs two kinds of fact about every RPC:
//!
//! - **What the wire declares** — the RPC exists, its request and response
//!   types, whether it streams. `kx_proto::control::GatewayRpc` is GENERATED
//!   from the compiled `FileDescriptorSet`, so it cannot drift from the schema
//!   and cannot be typo'd.
//! - **What the wire cannot know** — which domain the RPC belongs to, whether it
//!   READS or MUTATES, and what authority it demands. No descriptor can derive
//!   that `AdvanceBranch` is caller-scoped or that `ProposeWorkflow` writes
//!   nothing; a name-prefix heuristic would be a guess wearing a table's
//!   clothes. So [`facet`] is hand-authored, one arm per RPC.
//!
//! ## Why the match has no wildcard
//!
//! [`facet`] matches exhaustively with **no `_` arm**, over an enum that is
//! deliberately not `#[non_exhaustive]`. That is the entire guard:
//!
//! > Adding an RPC to `gateway.proto` makes `cargo check -p kx-gateway-core`
//! > fail with `error[E0004]: non-exhaustive patterns`.
//!
//! Not a test — `rustc`. A wildcard arm would convert that compile error into
//! silence, which is why `tests/control_surface.rs` scans this file's source and
//! fails if one appears. It is the same shape as the sidecar policy's
//! source-scan guard, and for the same reason: the failure being prevented is an
//! ABSENCE, and no behavioural test of the RPCs that WERE classified can see the
//! one that was not.
//!
//! ## Effect::Read means "writes nothing"
//!
//! `Propose*`, `Derive*`, `Discover*` and `Score*` are [`Effect::Read`] on
//! purpose: they validate, project or plan, and register nothing. The classifying
//! question is not "does this sound active" but "does a successful call change
//! durable state". Misclassifying a mutation as a read is the dangerous
//! direction, so anything genuinely ambiguous is classified [`Effect::Mutate`]
//! (`CallMcpTool` and `TestTrigger` both reach the world, and are marked
//! accordingly).

use kx_proto::control::GatewayRpc;

/// The subsystem an RPC belongs to.
///
/// Domains are the unit the NL authoring surface and the cloud control plane both
/// address, which is why adding a seventh authoring domain is a table entry
/// rather than a subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Domain {
    /// Runs, recipes, content, projections, signatures — the execution surface.
    Runs,
    /// The durable Workflow entity.
    Workflows,
    /// Apps, including scaffolding and hosted serving.
    Apps,
    /// The tool registry.
    Tools,
    /// MCP servers and their tools.
    Connectors,
    /// Secret NAMES and their lifecycle. Values never appear in this surface.
    Secrets,
    /// The script registry.
    Scripts,
    /// Triggers: webhook, cron, gRPC.
    Triggers,
    /// Teams, grants, and the durable Policy/Role registry.
    Policy,
    /// Operator decisions over staged effects (D114).
    Approvals,
    /// Project branches, their history, and restore.
    Branches,
    /// Context bundles.
    Context,
    /// Datasets, ingestion and retrieval.
    Datasets,
    /// Agent memory.
    Memory,
    /// Model lifecycle and selection.
    Models,
    /// Skills.
    Skills,
    /// Telemetry, alerts, capture, feedback, cost.
    Observability,
    /// Server identity and capability reporting.
    Server,
}

/// Whether a successful call changes durable state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    /// Writes nothing. Safe to execute directly from an NL proposal.
    Read,
    /// Changes durable state. Must be previewed and approved before it is issued.
    Mutate,
}

/// Whose authority a call runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Authority {
    /// Scoped to the calling principal; the caller sees only what it owns.
    CallerPrincipal,
    /// A serve-wide operator decision, not scoped to one principal's assets.
    OperatorGlobal,
    /// Refused unless the call arrives over loopback (secret writes, D81).
    LoopbackOnly,
}

/// The hand-authored half of an RPC's ControlSurface entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facet {
    /// The subsystem this RPC belongs to.
    pub domain: Domain,
    /// Whether it writes.
    pub effect: Effect,
    /// Whose authority it runs under.
    pub authority: Authority,
}

impl Facet {
    const fn new(domain: Domain, effect: Effect, authority: Authority) -> Self {
        Self {
            domain,
            effect,
            authority,
        }
    }
}

/// The domains the NL authoring surface covers.
///
/// A mutation in one of these must be reachable from the CLI as well as the
/// console and the SDKs — `tests/control_surface_reach.rs` enforces it. Domains
/// outside this list are addressable but are not authoring surfaces.
pub const AUTHORING: &[Domain] = &[
    Domain::Workflows,
    Domain::Tools,
    Domain::Connectors,
    Domain::Secrets,
    Domain::Scripts,
    Domain::Triggers,
    Domain::Policy,
];

/// `true` when `domain` is one of the [`AUTHORING`] domains.
#[must_use]
pub fn is_authoring(domain: Domain) -> bool {
    AUTHORING.contains(&domain)
}

/// Authoring domains that carry reads but no mutation YET, each with the reason.
///
/// An authoring domain with no mutation is a domain you can look at and not
/// author — which defeats the point of listing it. This list is the DECLARED,
/// temporary exception, and `every_authoring_domain_has_both_effects` holds the
/// rest of the table to the rule.
///
/// It is not a general escape hatch: `the_pending_list_has_no_dead_entries`
/// fails the moment a listed domain gains a mutation, so the entry cannot be
/// left behind after the work lands. Removing the last entry is the goal.
#[cfg(test)]
const AUTHORING_MUTATIONS_PENDING: &[(Domain, &str)] = &[(
    Domain::Policy,
    "teams/grants are readable, but the durable Policy/Role registry \
     (PutPolicyRole / DeletePolicyRole / AssignPolicyRole) is not on the wire yet",
)];

/// What this RPC is: its domain, whether it writes, and whose authority it needs.
///
/// Exhaustive by construction — see the module doc. Do not add a `_` arm.
#[must_use]
// One arm per RPC is the POINT. Both lints below want the table collapsed —
// `too_many_lines` by length, `match_same_arms` by merging arms that share an
// answer. Either collapse destroys the guarantee: the moment two RPCs share an
// arm, adding a third stops being a compile error. The table is meant to be
// long and repetitive; that is what makes an omission impossible.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub const fn facet(rpc: GatewayRpc) -> Facet {
    use Authority::{CallerPrincipal as CP, LoopbackOnly as LO, OperatorGlobal as OG};
    use Domain as D;
    use Effect::{Mutate as M, Read as R};

    match rpc {
        // ---- Runs / recipes / content / projection --------------------------
        // SubmitRun takes a client warrant verbatim and is refused any tool
        // authority (BLOCKER #5). It is deliberately absent from the CLI; the
        // DAG verbs use SubmitWorkflow instead.
        GatewayRpc::SubmitRun => Facet::new(D::Runs, M, CP),
        GatewayRpc::Invoke => Facet::new(D::Runs, M, CP),
        GatewayRpc::SubmitWorkflow => Facet::new(D::Runs, M, CP),
        GatewayRpc::GetProjection => Facet::new(D::Runs, R, CP),
        GatewayRpc::GetContent => Facet::new(D::Runs, R, CP),
        GatewayRpc::PutContent => Facet::new(D::Runs, M, CP),
        GatewayRpc::GetContentBatch => Facet::new(D::Runs, R, CP),
        GatewayRpc::StreamEvents => Facet::new(D::Runs, R, CP),
        GatewayRpc::StreamAllEvents => Facet::new(D::Runs, R, CP),
        GatewayRpc::StreamModelTokens => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListRuns => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListRecipes => Facet::new(D::Runs, R, CP),
        GatewayRpc::SearchRecipes => Facet::new(D::Runs, R, CP),
        GatewayRpc::GetRecipeForm => Facet::new(D::Runs, R, CP),
        GatewayRpc::GetRunInputs => Facet::new(D::Runs, R, CP),
        GatewayRpc::GetMoteDetail => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListReplanRounds => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListReactTurns => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListReRankTurns => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListSignatures => Facet::new(D::Runs, R, CP),
        GatewayRpc::GetSignature => Facet::new(D::Runs, R, CP),
        GatewayRpc::RegisterSignature => Facet::new(D::Runs, M, OG),
        GatewayRpc::ScoreTaskBundle => Facet::new(D::Runs, R, CP),
        GatewayRpc::ScoreRun => Facet::new(D::Runs, R, CP),
        GatewayRpc::ListToolManifests => Facet::new(D::Tools, R, CP),

        // ---- Workflows (authoring) ------------------------------------------
        // ProposeWorkflow is a READ: it decodes a goal, runs compile_plan, and
        // returns a plan or a refusal. It registers nothing, which is exactly
        // why it owes no CLI mutation verb.
        GatewayRpc::ProposeWorkflow => Facet::new(D::Workflows, R, CP),
        GatewayRpc::SaveWorkflow => Facet::new(D::Workflows, M, CP),
        GatewayRpc::ListWorkflows => Facet::new(D::Workflows, R, CP),
        GatewayRpc::GetWorkflow => Facet::new(D::Workflows, R, CP),
        GatewayRpc::RunWorkflow => Facet::new(D::Workflows, M, CP),
        GatewayRpc::DeleteWorkflow => Facet::new(D::Workflows, M, CP),

        // ---- Apps ------------------------------------------------------------
        // DeriveApp is the App-side NL authoring seam: it proposes an App and
        // writes nothing. The client saves it afterwards, if a human agrees.
        GatewayRpc::DeriveApp => Facet::new(D::Apps, R, CP),
        GatewayRpc::SaveApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::ListApps => Facet::new(D::Apps, R, CP),
        GatewayRpc::GetApp => Facet::new(D::Apps, R, CP),
        GatewayRpc::DeleteApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::GetAppManifest => Facet::new(D::Apps, R, CP),
        GatewayRpc::RunApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::ScaffoldApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::GetScaffoldStatus => Facet::new(D::Apps, R, CP),
        GatewayRpc::StartHostedApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::StopHostedApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::GetHostedAppStatus => Facet::new(D::Apps, R, CP),
        GatewayRpc::ListHostedApps => Facet::new(D::Apps, R, CP),
        GatewayRpc::LockApp => Facet::new(D::Apps, M, CP),
        GatewayRpc::UnlockApp => Facet::new(D::Apps, M, CP),

        // ---- Tools (authoring) -----------------------------------------------
        GatewayRpc::RegisterTool => Facet::new(D::Tools, M, CP),
        GatewayRpc::DeregisterTool => Facet::new(D::Tools, M, CP),
        GatewayRpc::DiscoverTools => Facet::new(D::Tools, R, CP),

        // ---- Connectors / MCP (authoring) ------------------------------------
        // CallMcpTool reaches the world through a connector, so it MUTATES even
        // though it reads from the caller's point of view.
        GatewayRpc::RegisterMcpServer => Facet::new(D::Connectors, M, CP),
        GatewayRpc::ListMcpServers => Facet::new(D::Connectors, R, CP),
        GatewayRpc::DiscoverServerTools => Facet::new(D::Connectors, R, CP),
        GatewayRpc::TestMcpServer => Facet::new(D::Connectors, R, CP),
        GatewayRpc::DeregisterMcpServer => Facet::new(D::Connectors, M, CP),
        GatewayRpc::CallMcpTool => Facet::new(D::Connectors, M, CP),

        // ---- Secrets (authoring) — NAMES never values ------------------------
        // Writes are loopback-gated: a credential must not be settable from off
        // the box (D81).
        GatewayRpc::PutSecret => Facet::new(D::Secrets, M, LO),
        GatewayRpc::DeleteSecret => Facet::new(D::Secrets, M, LO),
        GatewayRpc::ListSecretNames => Facet::new(D::Secrets, R, CP),

        // ---- Scripts (authoring) ---------------------------------------------
        GatewayRpc::RegisterScript => Facet::new(D::Scripts, M, CP),
        GatewayRpc::DeregisterScript => Facet::new(D::Scripts, M, CP),
        GatewayRpc::ListScripts => Facet::new(D::Scripts, R, CP),
        GatewayRpc::GetScript => Facet::new(D::Scripts, R, CP),

        // ---- Triggers (authoring) --------------------------------------------
        // TestTrigger fires the target, so it is a mutation regardless of its name.
        GatewayRpc::RegisterTrigger => Facet::new(D::Triggers, M, CP),
        GatewayRpc::ListTriggers => Facet::new(D::Triggers, R, CP),
        GatewayRpc::DeregisterTrigger => Facet::new(D::Triggers, M, CP),
        GatewayRpc::SubmitTrigger => Facet::new(D::Triggers, M, CP),
        GatewayRpc::TestTrigger => Facet::new(D::Triggers, M, CP),

        // ---- Policy / teams / grants (authoring) -----------------------------
        GatewayRpc::ListTeams => Facet::new(D::Policy, R, CP),
        GatewayRpc::ListTeamMembers => Facet::new(D::Policy, R, CP),
        GatewayRpc::ListAssetGrants => Facet::new(D::Policy, R, CP),

        // ---- Approvals (operator decisions over a SERVER-derived request id) --
        GatewayRpc::ListPendingApprovals => Facet::new(D::Approvals, R, OG),
        GatewayRpc::GrantApproval => Facet::new(D::Approvals, M, OG),
        GatewayRpc::DenyApproval => Facet::new(D::Approvals, M, OG),

        // ---- Branches ---------------------------------------------------------
        GatewayRpc::CreateBranch => Facet::new(D::Branches, M, CP),
        GatewayRpc::SnapshotInto => Facet::new(D::Branches, M, CP),
        GatewayRpc::ListBranches => Facet::new(D::Branches, R, CP),
        GatewayRpc::GetBranch => Facet::new(D::Branches, R, CP),
        GatewayRpc::DeleteBranch => Facet::new(D::Branches, M, CP),
        GatewayRpc::AdvanceBranch => Facet::new(D::Branches, M, CP),
        GatewayRpc::GetBranchContent => Facet::new(D::Branches, R, CP),
        GatewayRpc::ListBranchVersions => Facet::new(D::Branches, R, CP),
        GatewayRpc::RestoreBranch => Facet::new(D::Branches, M, CP),

        // ---- Context bundles ---------------------------------------------------
        GatewayRpc::PutContextBundle => Facet::new(D::Context, M, CP),
        GatewayRpc::ListContextBundles => Facet::new(D::Context, R, CP),
        GatewayRpc::GetContextBundle => Facet::new(D::Context, R, CP),
        GatewayRpc::DeleteContextBundle => Facet::new(D::Context, M, CP),

        // ---- Datasets / retrieval ----------------------------------------------
        GatewayRpc::ListDatasets => Facet::new(D::Datasets, R, CP),
        GatewayRpc::IngestDocuments => Facet::new(D::Datasets, M, CP),
        GatewayRpc::QueryDataset => Facet::new(D::Datasets, R, CP),
        GatewayRpc::FuzzyDiscovery => Facet::new(D::Datasets, R, CP),

        // ---- Memory -------------------------------------------------------------
        GatewayRpc::StoreMemory => Facet::new(D::Memory, M, CP),
        GatewayRpc::ListMemories => Facet::new(D::Memory, R, CP),
        GatewayRpc::RecallMemory => Facet::new(D::Memory, R, CP),
        GatewayRpc::ForgetMemory => Facet::new(D::Memory, M, CP),
        GatewayRpc::DecayMemory => Facet::new(D::Memory, M, CP),
        GatewayRpc::MemoryStats => Facet::new(D::Memory, R, CP),
        GatewayRpc::RestoreMemory => Facet::new(D::Memory, M, CP),

        // ---- Skills --------------------------------------------------------------
        GatewayRpc::ListSkills => Facet::new(D::Skills, R, CP),
        GatewayRpc::GetSkillForm => Facet::new(D::Skills, R, CP),
        GatewayRpc::AddSkill => Facet::new(D::Skills, M, CP),
        GatewayRpc::RemoveSkill => Facet::new(D::Skills, M, CP),

        // ---- Models ---------------------------------------------------------------
        GatewayRpc::ListModels => Facet::new(D::Models, R, CP),
        GatewayRpc::LoadModel => Facet::new(D::Models, M, OG),
        GatewayRpc::OffloadModel => Facet::new(D::Models, M, OG),
        GatewayRpc::PullModel => Facet::new(D::Models, M, OG),
        GatewayRpc::GetPullStatus => Facet::new(D::Models, R, OG),
        GatewayRpc::SetActiveModel => Facet::new(D::Models, M, OG),

        // ---- Observability ---------------------------------------------------------
        GatewayRpc::ListMoteTelemetry => Facet::new(D::Observability, R, CP),
        GatewayRpc::ListTelemetrySummary => Facet::new(D::Observability, R, CP),
        GatewayRpc::ListAlerts => Facet::new(D::Observability, R, CP),
        GatewayRpc::GetRunCost => Facet::new(D::Observability, R, CP),
        GatewayRpc::ListCaptureRecords => Facet::new(D::Observability, R, CP),
        GatewayRpc::SubmitFeedback => Facet::new(D::Observability, M, CP),
        GatewayRpc::ListFeedback => Facet::new(D::Observability, R, CP),

        // ---- Server -----------------------------------------------------------------
        GatewayRpc::GetServerInfo => Facet::new(D::Server, R, CP),
    }
}

#[cfg(test)]
mod tests {
    use super::{facet, is_authoring, Domain, Effect, AUTHORING, AUTHORING_MUTATIONS_PENDING};
    use kx_proto::control::GatewayRpc;
    use std::collections::BTreeSet;

    #[test]
    fn every_rpc_has_a_facet() {
        // `facet` is total by construction (exhaustive match, no wildcard), so
        // this cannot fail at runtime — it fails to COMPILE if an arm is missing.
        // What it pins is that the table is actually reachable for every RPC and
        // returns a well-formed answer.
        for rpc in GatewayRpc::ALL {
            let f = facet(*rpc);
            assert!(
                matches!(f.effect, Effect::Read | Effect::Mutate),
                "{}: effect is classified",
                rpc.as_str()
            );
        }
        assert!(
            GatewayRpc::ALL.len() >= 115,
            "expected the full surface, got {}",
            GatewayRpc::ALL.len()
        );
    }

    /// A streaming RPC is a read: you cannot stream a mutation's result.
    #[test]
    fn streaming_rpcs_are_reads() {
        for rpc in GatewayRpc::ALL.iter().filter(|r| r.server_streaming()) {
            assert_eq!(
                facet(*rpc).effect,
                Effect::Read,
                "{} streams, so it must be a Read",
                rpc.as_str()
            );
        }
    }

    /// Secret writes are loopback-only; a secret READ is not.
    ///
    /// This pins D81's boundary in the table rather than leaving it a comment: a
    /// credential must not be settable from off the box, and the name index must
    /// stay readable by the owning principal.
    #[test]
    fn secret_writes_are_loopback_only_and_reads_are_not() {
        use super::Authority;
        for rpc in GatewayRpc::ALL {
            let f = facet(*rpc);
            if f.domain != Domain::Secrets {
                continue;
            }
            match f.effect {
                Effect::Mutate => assert_eq!(
                    f.authority,
                    Authority::LoopbackOnly,
                    "{} writes a secret and must be loopback-only",
                    rpc.as_str()
                ),
                Effect::Read => assert_ne!(
                    f.authority,
                    Authority::LoopbackOnly,
                    "{} only lists NAMES; loopback is not required",
                    rpc.as_str()
                ),
            }
        }
    }

    /// Every authoring domain carries at least one read AND at least one mutation.
    ///
    /// Without this the classification could degenerate into an elaborate way to
    /// say "no opinion" — a table where everything is a Read would still satisfy
    /// exhaustiveness while conveying nothing.
    #[test]
    fn every_authoring_domain_has_both_effects() {
        let mut reads: BTreeSet<Domain> = BTreeSet::new();
        let mut mutations: BTreeSet<Domain> = BTreeSet::new();
        for rpc in GatewayRpc::ALL {
            let f = facet(*rpc);
            match f.effect {
                Effect::Read => reads.insert(f.domain),
                Effect::Mutate => mutations.insert(f.domain),
            };
        }
        let missing_reads: Vec<&Domain> = AUTHORING.iter().filter(|d| !reads.contains(d)).collect();
        let missing_mutations: Vec<&Domain> = AUTHORING
            .iter()
            .filter(|d| !mutations.contains(d))
            .filter(|d| !AUTHORING_MUTATIONS_PENDING.iter().any(|(p, _)| p == *d))
            .collect();
        assert!(
            missing_reads.is_empty(),
            "authoring domains with no Read: {missing_reads:?}"
        );
        assert!(
            missing_mutations.is_empty(),
            "authoring domains with no Mutate and no declared reason: {missing_mutations:?} \
             — either classify the mutation or add it to AUTHORING_MUTATIONS_PENDING with why"
        );
    }

    /// A domain that HAS a mutation must not still be listed as pending.
    ///
    /// This is what stops [`AUTHORING_MUTATIONS_PENDING`] from becoming a
    /// blanket: the moment the Policy registry lands on the wire, this test goes
    /// red until the entry is deleted, so the exception cannot outlive the gap it
    /// describes.
    #[test]
    fn the_pending_list_has_no_dead_entries() {
        let mutations: BTreeSet<Domain> = GatewayRpc::ALL
            .iter()
            .map(|r| facet(*r))
            .filter(|f| f.effect == Effect::Mutate)
            .map(|f| f.domain)
            .collect();
        let dead: Vec<Domain> = AUTHORING_MUTATIONS_PENDING
            .iter()
            .filter(|(d, _)| mutations.contains(d))
            .map(|(d, _)| *d)
            .collect();
        assert!(
            dead.is_empty(),
            "these domains now HAVE a mutation and must be removed from \
             AUTHORING_MUTATIONS_PENDING: {dead:?}"
        );

        for (domain, why) in AUTHORING_MUTATIONS_PENDING {
            assert!(
                AUTHORING.contains(domain),
                "{domain:?} is listed as pending but is not an authoring domain"
            );
            assert!(
                why.len() > 30,
                "{domain:?}'s pending entry needs a real reason, got {why:?}"
            );
        }
    }

    #[test]
    fn authoring_membership_is_consistent() {
        assert!(is_authoring(Domain::Workflows));
        assert!(is_authoring(Domain::Scripts));
        assert!(!is_authoring(Domain::Runs));
        assert!(!is_authoring(Domain::Observability));
        let unique: BTreeSet<&Domain> = AUTHORING.iter().collect();
        assert_eq!(unique.len(), AUTHORING.len(), "AUTHORING has no duplicates");
    }
}
