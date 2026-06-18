//! G3 enterprise scenario (the adoption unlock): publish a parametrized recipe,
//! grant a TEAM `Use`, a team MEMBER invokes it with arguments, and the bound run
//! executes to Committed on the durable spine — exactly-once. Plus the edge cases
//! a mature runtime must hold: ungranted refused, no-widen, fail-closed args,
//! distinct-args→distinct-run, identical-args→idempotent, unbound-slot refused.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{
    encode_param_schema, AssetBinding, AssetPath, AssetRef, AssetVersion, BodyLedger,
    CatalogAction, CatalogActionSet, FreeParamContract, FreeParamSlot, Grant, GrantLedger,
    InMemoryBodyLedger, InMemoryGrantLedger, InMemoryVersionLedger, ParamType, PartyId, Provenance,
    SchemaResolver, SlotBinding, VersionLedger, VersionedContent,
};
use kx_content::ContentRef;
use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_fleet::{Admit, GovernedFleet, InMemoryMembershipLedger, MembershipLedger, Team};
use kx_invoke::{bind_snapshot, BoundRun, InvokeError, UseWarrantResolver};
use kx_journal::SqliteJournal;
use kx_mote::{ConfigKey, ConfigVal, EdgeMeta, LogicRef, ModelId, ToolName};
use kx_projection::Projection;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, Role, WarrantSpec,
};
use kx_workflow::{transform, WorkflowDef};
use std::collections::BTreeMap;

// --- fixtures --------------------------------------------------------------

const TOPIC_SCHEMA_REF: [u8; 32] = [0x55; 32];

/// A permissive warrant differing only in `model_route.max_calls`, so warrant
/// narrowing is observable on that axis while every other axis stays compatible
/// (no spurious `AttemptedWiden`).
fn warrant(max_calls: u32) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::default(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: std::collections::BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 4_096,
            max_calls,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

fn role(name: &str, max_calls: u32) -> Role {
    Role {
        name: name.into(),
        version: 1,
        spec: warrant(max_calls),
        description: String::new(),
    }
}

/// A recipe body: a 2-step pure chain A → B where leaf A declares a variable
/// `topic` config slot (a placeholder bind overwrites). B is the sink (output).
fn recipe_body() -> WorkflowDef {
    let mut wf = WorkflowDef::new(7);
    let mut a = transform(
        LogicRef::from_bytes([1; 32]),
        ModelId("m".into()),
        warrant(16),
        ToolName("demo".into()),
    );
    a.config_subset
        .insert(ConfigKey("topic".into()), ConfigVal(Vec::new()));
    let a_ref = wf.add_step(a);
    let b_ref = wf.add_step(transform(
        LogicRef::from_bytes([2; 32]),
        ModelId("m".into()),
        warrant(16),
        ToolName("demo".into()),
    ));
    wf.add_edge(a_ref, b_ref, EdgeMeta::data()).unwrap();
    wf
}

/// One variable slot `topic` typed `Str`.
fn topic_contract() -> FreeParamContract {
    let mut slots = BTreeMap::new();
    slots.insert(
        "topic".to_string(),
        FreeParamSlot {
            binding: SlotBinding::Variable,
            schema_ref: Some(TOPIC_SCHEMA_REF),
        },
    );
    FreeParamContract { slots }
}

struct StrResolver;
impl SchemaResolver for StrResolver {
    fn resolve_schema(&self, schema_ref: &[u8; 32]) -> Option<Vec<u8>> {
        if *schema_ref == TOPIC_SCHEMA_REF {
            Some(encode_param_schema(&ParamType::Str { max_len: 256 }))
        } else {
            None
        }
    }
}

/// A `UseWarrantResolver` backed by a team membership fold (the production seam).
struct FleetUse<'a, M: MembershipLedger, G: GrantLedger> {
    fleet: &'a GovernedFleet<M, G>,
    owner_root: WarrantSpec,
}
impl<M: MembershipLedger, G: GrantLedger> UseWarrantResolver for FleetUse<'_, M, G> {
    fn resolve_use(&self, party: &PartyId, asset: &AssetRef) -> Option<WarrantSpec> {
        self.fleet
            .resolve_member_warrant(party, asset, CatalogAction::Use, &self.owner_root)
            .ok()
            .flatten()
    }
}

/// The whole governance + catalog world for the scenario.
struct World {
    versions: InMemoryVersionLedger,
    bodies: InMemoryBodyLedger,
    fleet: GovernedFleet<InMemoryMembershipLedger, InMemoryGrantLedger>,
    owner_root: WarrantSpec,
    handle: AssetPath,
}

fn setup() -> World {
    let admin = PartyId::new("admin@acme");
    let sre_team = PartyId::new("team:sre@acme");
    let alice = PartyId::new("alice@acme");
    let owner_root = warrant(100);

    let handle = AssetPath::new("acme", "recipes", "triage").unwrap();
    let asset = AssetRef::Path(handle.clone());

    // Grants: admin owns the recipe; the SRE team is granted Use+Read under a
    // warrant capped at 50 calls.
    let grants = InMemoryGrantLedger::new();
    grants
        .append_binding(AssetBinding::new(asset.clone(), admin.clone()))
        .unwrap();
    grants
        .append_grant(Grant::root(
            asset.clone(),
            admin.clone(),
            sre_team.clone(),
            CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
            role("team-use", 50),
        ))
        .unwrap();

    // Fleet: found the SRE team; admit Alice (Use-capped) under a 20-call role.
    let members = InMemoryMembershipLedger::new();
    members
        .append_founding(Team::found(sre_team.clone(), admin.clone(), "SRE"))
        .unwrap();
    members
        .append_admit(Admit::new(
            sre_team.clone(),
            alice.clone(),
            admin.clone(),
            role("oncall", 20),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let fleet = GovernedFleet::new(members, grants);

    // Publish the recipe body + a version handle pointing at it.
    let bodies = InMemoryBodyLedger::new();
    let (manifest_id, _) = bodies.publish_body(recipe_body()).unwrap();
    let versions = InMemoryVersionLedger::new();
    versions
        .publish(AssetVersion::root(
            handle.clone(),
            VersionedContent::Workflow(manifest_id),
            admin.clone(),
            Provenance::from_recipe(manifest_id.0),
        ))
        .unwrap();

    World {
        versions,
        bodies,
        fleet,
        owner_root,
        handle,
    }
}

fn bind_for(world: &World, party: &str, args: &[u8]) -> Result<BoundRun, InvokeError> {
    let resolver = FleetUse {
        fleet: &world.fleet,
        owner_root: world.owner_root.clone(),
    };
    bind_snapshot(
        &world.versions,
        &world.bodies,
        &resolver,
        &PartyId::new(party),
        &world.handle,
        &topic_contract(),
        &StrResolver,
        args,
        &[],
    )
}

// --- the happy path: a member invokes, the run reaches Committed -----------

#[test]
fn g3_member_invokes_published_recipe_to_committed_exactly_once() {
    let world = setup();
    let bound = bind_for(&world, "alice@acme", br#"{"topic":"incidents"}"#).unwrap();
    assert_eq!(bound.motes.len(), 2, "the A->B recipe compiled to 2 Motes");

    // No-widen: every Mote runs under a warrant ⊆ Alice's effective authority
    // (min(owner 100, team 50, alice 20) = 20) AND ⊆ the recipe step (16) ⇒ 16.
    for (_, w) in &bound.motes {
        assert!(
            w.model_route.max_calls <= 20,
            "bound warrant must not exceed the member's authority"
        );
    }

    // Drive the bound run to Committed on the real single-node durable spine.
    let journal = SqliteJournal::open_in_memory().unwrap();
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    for (mote, w) in &bound.motes {
        run_pure_mote(mote, w, &journal, &rm, &executor).unwrap();
    }
    let projection = Projection::from_journal(&journal).unwrap();
    assert_eq!(projection.committed_count(), 2, "both Motes committed");
    let result = projection.result_ref_of(&bound.terminal_mote_id);
    assert!(result.is_some(), "the terminal Mote produced a result_ref");
}

#[test]
fn distinct_args_yield_distinct_runs_identical_args_are_idempotent() {
    let world = setup();
    let a = bind_for(&world, "alice@acme", br#"{"topic":"incidents"}"#).unwrap();
    let b = bind_for(&world, "alice@acme", br#"{"topic":"outages"}"#).unwrap();
    let a2 = bind_for(&world, "alice@acme", br#"{"topic":"incidents"}"#).unwrap();

    // Distinct args → distinct terminal identity (a fresh exactly-once run).
    assert_ne!(a.terminal_mote_id, b.terminal_mote_id);
    // Identical args → identical identity (the coordinator dedups → idempotent).
    assert_eq!(a.terminal_mote_id, a2.terminal_mote_id);
    assert_eq!(
        a.recipe_fingerprint, b.recipe_fingerprint,
        "same recipe template"
    );
}

// --- authorization edges ---------------------------------------------------

#[test]
fn ungranted_party_is_refused() {
    let world = setup();
    let err = bind_for(&world, "mallory@evil", br#"{"topic":"x"}"#).unwrap_err();
    assert!(matches!(err, InvokeError::Unauthorized));
}

// --- fail-closed argument validation ---------------------------------------

#[test]
fn malformed_args_are_refused_fail_closed() {
    let world = setup();
    // Wrong type (Int where Str required).
    assert!(matches!(
        bind_for(&world, "alice@acme", br#"{"topic":5}"#),
        Err(InvokeError::ArgValidation(_))
    ));
    // Unknown field smuggled in.
    assert!(matches!(
        bind_for(&world, "alice@acme", br#"{"topic":"x","secret":"y"}"#),
        Err(InvokeError::ArgValidation(_))
    ));
    // Missing the required slot.
    assert!(matches!(
        bind_for(&world, "alice@acme", br#"{}"#),
        Err(InvokeError::ArgValidation(_))
    ));
}

#[test]
fn a_variable_slot_that_binds_no_step_is_refused() {
    let world = setup();
    // A contract declaring a slot `ghost` that the recipe body declares on no
    // step → fail-closed (never silently drop a supplied parameter).
    let mut slots = BTreeMap::new();
    slots.insert(
        "ghost".to_string(),
        FreeParamSlot {
            binding: SlotBinding::Variable,
            schema_ref: Some(TOPIC_SCHEMA_REF),
        },
    );
    let resolver = FleetUse {
        fleet: &world.fleet,
        owner_root: world.owner_root.clone(),
    };
    let err = bind_snapshot(
        &world.versions,
        &world.bodies,
        &resolver,
        &PartyId::new("alice@acme"),
        &world.handle,
        &FreeParamContract { slots },
        &StrResolver,
        br#"{"ghost":"boo"}"#,
        &[],
    )
    .unwrap_err();
    assert!(matches!(err, InvokeError::SlotUnbound(s) if s == "ghost"));
}

// --- unpublished handle ----------------------------------------------------

// --- execute() proxy ordering (register-before-submit) ---------------------

#[derive(Clone, Default)]
struct MockSubmitter {
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[tonic::async_trait]
impl kx_gateway_core::RunSubmitter for MockSubmitter {
    async fn register_run(
        &self,
        _recipe_fingerprint: [u8; 32],
    ) -> Result<[u8; 16], kx_gateway_core::SubmitterError> {
        self.calls.lock().unwrap().push("register".into());
        Ok([0x42; 16])
    }
    async fn submit_mote(
        &self,
        mote: kx_mote::Mote,
        _warrant: WarrantSpec,
        _accept: bool,
        _react_seed: bool,
    ) -> Result<kx_gateway_core::SubmitMoteOutcome, kx_gateway_core::SubmitterError> {
        self.calls.lock().unwrap().push("submit".into());
        Ok(kx_gateway_core::SubmitMoteOutcome {
            mote_id: *mote.id.as_bytes(),
            instance_id: [0x42; 16],
            status: kx_gateway_core::SubmitStatus::Accepted,
        })
    }
}

#[tokio::test]
async fn execute_registers_before_submitting_each_mote() {
    let world = setup();
    let bound = bind_for(&world, "alice@acme", br#"{"topic":"incidents"}"#).unwrap();
    let mock = MockSubmitter::default();
    let submitted = kx_invoke::execute(&mock, &bound).await.unwrap();

    assert_eq!(submitted.instance_id, [0x42; 16]);
    assert_eq!(submitted.terminal_mote_id, bound.terminal_mote_id);
    let calls = mock.calls.lock().unwrap().clone();
    // register first, then one submit per bound Mote.
    assert_eq!(calls.first().map(String::as_str), Some("register"));
    assert_eq!(
        calls.iter().filter(|c| *c == "submit").count(),
        bound.motes.len()
    );
    assert!(
        calls.iter().position(|c| c == "register") < calls.iter().position(|c| c == "submit"),
        "must register the run before submitting any Mote (never ack ahead of the journal)"
    );
}

// --- unpublished handle ----------------------------------------------------

#[test]
fn unpublished_handle_is_not_found_for_authorized_caller() {
    let world = setup();
    // Alice is authorized to Use the namespace asset, but this specific handle
    // was never published.
    let resolver = FleetUse {
        fleet: &world.fleet,
        owner_root: world.owner_root.clone(),
    };
    let unknown = AssetPath::new("acme", "recipes", "triage").unwrap();
    // Resolve a DIFFERENT (unpublished) version ledger to force NotFound.
    let empty_versions = InMemoryVersionLedger::new();
    let err = bind_snapshot(
        &empty_versions,
        &world.bodies,
        &resolver,
        &PartyId::new("alice@acme"),
        &unknown,
        &topic_contract(),
        &StrResolver,
        br#"{"topic":"x"}"#,
        &[],
    )
    .unwrap_err();
    assert!(matches!(err, InvokeError::NotFound));
}
