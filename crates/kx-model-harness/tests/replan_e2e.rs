//! M6 — D76/D77 iterative replanning end-to-end.
//!
//! An agentic loop is NOT a DAG cycle: it lowers (via
//! `kx_planner::lower_loop_to_topology_decision`) to a `TopologyDecision` a ROND
//! shaper commits, which the SHIPPED `DefaultTopologyMaterializer` materializes
//! into children deterministically. This proves:
//!
//! - a planner-PRODUCED `TopologyDecision` drives the real materializer (children
//!   materialize, appear ready, parent = the shaper);
//! - a re-plan **appends** a fresh round (a distinct committed `TopologyDecision`
//!   fact) and NEVER mutates a committed Mote — round-1's entries + states are
//!   byte-unchanged after round-2 folds (D76 / D-LOCK-4);
//! - cold re-fold reproduces identical identities (R49).
//!
//! Fully deterministic (no model, clock, network). The planner's "samples" are
//! the two fixed `LoopProposal`s.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    canonical_config, derive_mote_id, EffectPattern, GraphPosition, InferenceParams, InputDataId,
    LogicRef, ModelId, MoteDef, MoteId, NdClass, PromptTemplateHash, RoleId, ToolName,
    TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};
use kx_planner::{
    lower_loop_to_topology_decision, InMemoryRoleRecipes, LoopProposal, PlanStep, PlanStepKind,
    RoleRecipe,
};
use kx_projection::{
    DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver, MoteState,
    Projection,
};
use kx_warrant::{
    ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    Role, WarrantSpec,
};
use smallvec::SmallVec;

fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("test-model".into()),
            max_input_tokens: 1024,
            max_output_tokens: 1024,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 20,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 20,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// A ROND topology-shaper MoteDef (R-14: a shaper is never WORLD-MUTATING). The
/// `seed` distinguishes round-1 (planner) from round-2 (replanner).
fn shaper_def(seed: u8) -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef([seed; 32]),
        model_id: ModelId("planner".into()),
        prompt_template_hash: PromptTemplateHash([seed; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn shaper_id(def: &MoteDef, position: u8) -> MoteId {
    derive_mote_id(
        &def.hash(),
        &InputDataId::from_bytes([0u8; 32]),
        &GraphPosition(vec![position]),
    )
}

fn pure_recipe(seed: u8) -> RoleRecipe {
    RoleRecipe {
        logic_ref: LogicRef::from_bytes([seed; 32]),
        model_id: ModelId("worker".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([seed; 32]),
        tool_contract: BTreeMap::new(),
        capability: ToolName("kx-model".into()),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        inference_params: InferenceParams::default(),
        deterministic_check: None,
    }
}

fn plain_step(role: &str) -> PlanStep {
    PlanStep {
        role: role.into(),
        intent: format!("do {role}"),
        kind: PlanStepKind::Plain,
        producer: None,
    }
}

fn stage_td(store: &InMemoryContentStore, td: &TopologyDecision) -> ContentRef {
    let bytes = bincode::serde::encode_to_vec(td, canonical_config()).unwrap();
    store.put(&bytes).unwrap()
}

fn stage_warrant(store: &InMemoryContentStore) -> ContentRef {
    let bytes = bincode::serde::encode_to_vec(permissive_warrant(), canonical_config()).unwrap();
    store.put(&bytes).unwrap()
}

fn committed_shaper(
    shaper: MoteId,
    seq: u64,
    td_ref: ContentRef,
    warrant_ref: ContentRef,
    def_hash: kx_mote::MoteDefHash,
) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: shaper,
        idempotency_key: shaper.0,
        seq,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: td_ref,
        parents: SmallVec::new(),
        warrant_ref,
        mote_def_hash: def_hash,
    }
}

#[test]
fn replanning_appends_a_fresh_round_and_never_mutates_round_one() {
    // ---- Recipes (for kx_planner) + roles (for the materializer warrant) ----
    let recipes = InMemoryRoleRecipes::new();
    recipes.register(RoleId("reader".into()), pure_recipe(0xA1));
    recipes.register(RoleId("summarizer".into()), pure_recipe(0xB2));
    recipes.register(RoleId("refiner".into()), pure_recipe(0xC3));

    // Round 1 + round 2 proposals → distinct TopologyDecisions via the planner.
    let round1 = LoopProposal {
        next_steps: vec![plain_step("reader"), plain_step("summarizer")],
    };
    let round2 = LoopProposal {
        next_steps: vec![plain_step("refiner")],
    };
    let td1 = lower_loop_to_topology_decision(&round1, &recipes).unwrap();
    let td2 = lower_loop_to_topology_decision(&round2, &recipes).unwrap();
    assert_ne!(
        td1.hash(),
        td2.hash(),
        "a re-plan is a DISTINCT committed fact"
    );
    assert_eq!(td1.children.len(), 2);
    assert_eq!(td2.children.len(), 1);

    // ---- Materializer wiring (the SHIPPED runtime path) ----
    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    let role_registry = Arc::new(InMemoryRoleRegistry::new());
    let role = Role {
        name: "default".into(),
        version: 1,
        spec: permissive_warrant(),
        description: String::new(),
    };
    for r in ["reader", "summarizer", "refiner"] {
        role_registry.register(RoleId(r.into()), role.clone());
    }
    let s1 = shaper_def(0x01);
    let s2 = shaper_def(0x02);
    def_registry.register(s1.clone());
    def_registry.register(s2.clone());
    let s1_id = shaper_id(&s1, 0x01);
    let s2_id = shaper_id(&s2, 0x02);
    let warrant_ref = stage_warrant(&store);
    let td1_ref = stage_td(&store, &td1);
    let td2_ref = stage_td(&store, &td2);

    // ---- Round 1: planner shaper commits td1 → materialize 2 children ----
    let journal = InMemoryJournal::new();
    let e1 = committed_shaper(s1_id, 1, td1_ref, warrant_ref, s1.hash());
    journal.append(e1).unwrap();

    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        Arc::clone(&role_registry),
        InheritFromShaperResolver,
    ));
    let mut projection =
        Projection::from_journal_with_materializer(&journal, materializer).unwrap();
    assert_eq!(projection.len(), 3, "shaper-1 + 2 children");
    let round1_children = projection.ready_set();
    assert_eq!(
        round1_children.len(),
        2,
        "round-1 children materialized + ready"
    );
    for c in &round1_children {
        assert_eq!(
            projection.parents_of(c)[0].0,
            s1_id,
            "child parent = the shaper"
        );
    }

    // Commit round-1 children.
    for (i, c) in round1_children.iter().enumerate() {
        let e = JournalEntry::Committed {
            mote_id: *c,
            idempotency_key: c.0,
            seq: 2 + i as u64,
            nondeterminism: NdClass::Pure,
            result_ref: ContentRef::from_bytes([(0x30 + i as u8); 32]),
            parents: SmallVec::new(),
            warrant_ref,
            mote_def_hash: kx_mote::MoteDefHash::from_bytes([(0x40 + i as u8); 32]),
        };
        journal.append(e.clone()).unwrap();
        projection.fold(&e).unwrap();
    }
    assert_eq!(
        projection.committed_count(),
        3,
        "shaper-1 + 2 children committed"
    );
    let round1_committed: Vec<MoteId> = round1_children.clone();

    // ---- Round 2: replanner shaper APPENDS td2 → materialize a fresh child ----
    let e_replan = committed_shaper(s2_id, 4, td2_ref, warrant_ref, s2.hash());
    journal.append(e_replan.clone()).unwrap();
    projection.fold(&e_replan).unwrap();

    // The fresh round materialized; round-1 committed Motes are UNCHANGED (D76).
    assert_eq!(projection.len(), 5, "shaper-1 + 2 + shaper-2 + 1 refiner");
    assert_eq!(projection.state_of(&s1_id), MoteState::Committed);
    for c in &round1_committed {
        assert_eq!(
            projection.state_of(c),
            MoteState::Committed,
            "a re-plan never mutates a committed Mote (D76)"
        );
    }
    let refiner = projection
        .ready_set()
        .into_iter()
        .find(|m| projection.parents_of(m).first().map(|p| p.0) == Some(s2_id))
        .expect("round-2 refiner materialized under the replanner");
    assert_eq!(projection.state_of(&refiner), MoteState::Pending);

    // ---- Cold re-fold reproduces identical identities (R49) ----
    let materializer2 = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        Arc::clone(&role_registry),
        InheritFromShaperResolver,
    ));
    let re = Projection::from_journal_with_materializer(&journal, materializer2).unwrap();
    let live: Vec<MoteId> = projection.iter_motes().map(|(id, _)| id).collect();
    let cold: Vec<MoteId> = re.iter_motes().map(|(id, _)| id).collect();
    assert_eq!(
        live, cold,
        "R49: cold re-fold = live fold across two rounds"
    );
}
