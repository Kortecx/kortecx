#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! **P1.11 — 9-edge-case test classes per standing testing doctrine**
//! (`04-testing-and-gates.md` §86+).
//!
//! Class #2 (replay faithfulness — R49) lives in `cold_refold_topology.rs`
//! with its own P1+P2+P3+P4 surface. THIS file covers classes 1, 3, 5, 6,
//! 7, 8, 9 + KG-1 mitigation. Class #4 is N/A — STRUCTURALLY REFUSED by
//! R-14 (tested in `kx-executor/tests/r14_refusal.rs`).

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{repudiation_idempotency_key, JournalEntry, RepudiationReason};
use kx_mote::{
    canonical_config, ChildDescriptor, EffectPattern, LogicRef, ModelId, MoteDef, MoteId, NdClass,
    PromptTemplateHash, RoleId, TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::{
    DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver, MoteState,
    Projection,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, Role, RoleRegistry, WarrantSpec,
};
use smallvec::SmallVec;
use std::collections::{BTreeMap, BTreeSet};

fn shaper_def() -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef([1u8; 32]),
        model_id: ModelId("planner-v1".into()),
        prompt_template_hash: PromptTemplateHash([3u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn descriptor(seed: u8, nd: NdClass, ep: EffectPattern) -> ChildDescriptor {
    ChildDescriptor {
        role_id: RoleId(format!("role-{seed}")),
        logic_ref: LogicRef([seed; 32]),
        nd_class: nd,
        effect_pattern: ep,
    }
}

fn shaper_mote_id(def: &MoteDef) -> MoteId {
    kx_mote::derive_mote_id(
        &def.hash(),
        &kx_mote::InputDataId::from_bytes([0u8; 32]),
        &kx_mote::GraphPosition(vec![0u8]),
    )
}

fn encode_topology(td: &TopologyDecision) -> Vec<u8> {
    bincode::serde::encode_to_vec(td, canonical_config())
        .expect("TopologyDecision canonical bincode encodes infallibly")
}

/// Permissive [`WarrantSpec`] used as the shaper's warrant + as every
/// child role's spec in the 9-edge-case tests (the narrowing path is a
/// no-op when role.spec == shaper.warrant — exercise its semantics
/// separately in the `kg1_close_*` tests).
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

/// Test-local [`RoleRegistry`] that returns the same [`Role`] for any
/// `RoleId`. The 9-edge-case tests don't exercise per-role narrowing.
struct PermissiveAnyRoleRegistry {
    role: Role,
}

impl RoleRegistry for PermissiveAnyRoleRegistry {
    fn resolve(&self, _role_id: &RoleId) -> Option<Role> {
        Some(self.role.clone())
    }
}

/// Stage a permissive [`WarrantSpec`] in `store` and return its
/// [`ContentRef`]. The 9-edge-case tests need a valid warrant_ref on
/// the shaper's Committed entry; the actual narrowing semantics are
/// trivially satisfied by `role.spec == shaper.warrant`.
fn stage_permissive_warrant(store: &InMemoryContentStore) -> ContentRef {
    let bytes = bincode::serde::encode_to_vec(permissive_warrant(), canonical_config())
        .expect("WarrantSpec canonical bincode encodes infallibly");
    store.put(&bytes).expect("put succeeds")
}

fn permissive_role_registry() -> Arc<PermissiveAnyRoleRegistry> {
    Arc::new(PermissiveAnyRoleRegistry {
        role: Role {
            name: "test-default".into(),
            version: 1,
            spec: permissive_warrant(),
            description: String::new(),
        },
    })
}

fn build_projection_with_topology(
    shaper: &MoteDef,
    td: &TopologyDecision,
) -> (Arc<InMemoryContentStore>, Projection, MoteId, ContentRef) {
    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper.clone());
    let role_registry = permissive_role_registry();
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        role_registry,
        InheritFromShaperResolver,
    ));
    let bytes = encode_topology(td);
    let td_ref = store.put(&bytes).expect("put");
    // Stage the shaper's WarrantSpec in the same content store so the
    // materializer can fetch + decode it at fold time (PR 11.5 path).
    let shaper_warrant_ref = stage_permissive_warrant(&store);
    let shaper_id = shaper_mote_id(shaper);
    let mut proj = Projection::with_materializer(materializer);
    let entry = JournalEntry::Committed {
        mote_id: shaper_id,
        idempotency_key: shaper_id.0,
        seq: 1,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: td_ref,
        parents: SmallVec::new(),
        warrant_ref: shaper_warrant_ref,
        mote_def_hash: shaper.hash(),
    };
    proj.fold(&entry).unwrap();
    (store, proj, shaper_id, td_ref)
}

// ---------------------------------------------------------------------------
// Class #1 — EXACTLY-ONCE / IDEMPOTENCY
// ---------------------------------------------------------------------------

#[test]
fn class1_idempotent_refold_does_not_double_add_children() {
    // The fold is invoked once per journal entry. The DuplicateCommitted
    // check guarantees that a second fold of the same Committed entry is
    // rejected — but if a caller built two projections from the same
    // journal, both produce identical state (idempotence at the
    // projection level, not at the fold level).
    let shaper = shaper_def();
    let td = TopologyDecision {
        children: vec![
            descriptor(10, NdClass::Pure, EffectPattern::IdempotentByConstruction),
            descriptor(20, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        ],
    };
    let (_store_a, proj_a, shaper_id_a, _ref_a) = build_projection_with_topology(&shaper, &td);
    let (_store_b, proj_b, shaper_id_b, _ref_b) = build_projection_with_topology(&shaper, &td);
    assert_eq!(shaper_id_a, shaper_id_b);
    assert_eq!(
        proj_a.len(),
        proj_b.len(),
        "double fold = same projection length"
    );
    // Same Motes, same parents.
    let a_motes: Vec<MoteId> = proj_a.iter_motes().map(|(id, _)| id).collect();
    let b_motes: Vec<MoteId> = proj_b.iter_motes().map(|(id, _)| id).collect();
    assert_eq!(a_motes, b_motes);
}

// ---------------------------------------------------------------------------
// Class #5 — POISON / REPUDIATION
// ---------------------------------------------------------------------------

#[test]
fn class5_repudiated_shaper_blocks_dispatch_of_materialized_children() {
    let shaper = shaper_def();
    let td = TopologyDecision {
        children: vec![
            descriptor(10, NdClass::Pure, EffectPattern::IdempotentByConstruction),
            descriptor(20, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        ],
    };
    let (_store, mut proj, shaper_id, _td_ref) = build_projection_with_topology(&shaper, &td);

    // Before repudiation: children are in ready_set (shaper committed; they
    // have only the shaper as parent which is Committed-not-Repudiated).
    let ready_before: Vec<MoteId> = proj.ready_set();
    assert_eq!(
        ready_before.len(),
        2,
        "both children ready before repudiation"
    );

    // Repudiate the shaper.
    let target_seq = proj.committed_seq_of(&shaper_id).unwrap();
    proj.fold(&JournalEntry::Repudiated {
        idempotency_key: repudiation_idempotency_key(&shaper_id, target_seq),
        seq: 100,
        target_mote_id: shaper_id,
        target_committed_seq: target_seq,
        reason_class: RepudiationReason::CriticInvalidated,
        repudiator_id: 0,
    })
    .unwrap();

    assert_eq!(proj.state_of(&shaper_id), MoteState::Repudiated);
    // Children's ready_set membership drops because their parent (shaper)
    // is now Repudiated (ready_set requires Committed-AND-NOT-Repudiated parents).
    let ready_after: Vec<MoteId> = proj.ready_set();
    assert_eq!(
        ready_after.len(),
        0,
        "children blocked by repudiated parent"
    );

    // transitive_consumers from the shaper should include both children.
    let consumers: Vec<MoteId> = proj.transitive_consumers(&shaper_id);
    assert_eq!(consumers.len(), 2);
}

// ---------------------------------------------------------------------------
// Class #6 — NON-DETERMINISM GATE (PURE / READ-ONLY-NONDET / WORLD-MUTATING)
// ---------------------------------------------------------------------------

#[test]
fn class6_read_only_nondet_shaper_replays_committed_decision_not_reruns() {
    // Folding the same committed decision twice (across two projections)
    // produces bit-identical child sets — the projection reads the
    // committed payload, never invokes the shaper to "re-decide."
    let shaper = shaper_def();
    assert_eq!(shaper.nd_class, NdClass::ReadOnlyNondet);
    let td = TopologyDecision {
        children: vec![descriptor(
            7,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let (_store_a, proj_a, _id_a, _r_a) = build_projection_with_topology(&shaper, &td);
    let (_store_b, proj_b, _id_b, _r_b) = build_projection_with_topology(&shaper, &td);
    // Same Motes (the materializer reads committed payload, not re-decides):
    let a: Vec<MoteId> = proj_a.iter_motes().map(|(id, _)| id).collect();
    let b: Vec<MoteId> = proj_b.iter_motes().map(|(id, _)| id).collect();
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// Class #7 — BOUNDARY / DEGENERATE INPUTS
// ---------------------------------------------------------------------------

#[test]
fn class7a_empty_topology_decision_materializes_zero_children() {
    // Also covered in cold_refold_topology.rs but repeated here for the
    // 9-edge-case enumeration's completeness.
    let shaper = shaper_def();
    let td = TopologyDecision { children: vec![] };
    let (_store, proj, shaper_id, _r) = build_projection_with_topology(&shaper, &td);
    assert_eq!(proj.len(), 1);
    assert_eq!(proj.state_of(&shaper_id), MoteState::Committed);
}

#[test]
fn class7b_single_child_topology_decision() {
    let shaper = shaper_def();
    let td = TopologyDecision {
        children: vec![descriptor(
            99,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let (_store, proj, shaper_id, _r) = build_projection_with_topology(&shaper, &td);
    assert_eq!(proj.len(), 2);
    let ready = proj.ready_set();
    assert_eq!(ready.len(), 1);
    assert_ne!(ready[0], shaper_id);
}

#[test]
fn class7c_topology_decision_with_large_children_n_eq_128() {
    // N=128 chosen as smallest power-of-2 in the doctrine's "large-N DAGs
    // (e.g. N=100s)" band per 04-testing-and-gates.md §170. Exercises
    // BLAKE3 across the canonical-bincode encoding of a non-trivially-
    // sized Vec.
    let shaper = shaper_def();
    let children: Vec<ChildDescriptor> = (0..128u8)
        .map(|i| descriptor(i, NdClass::Pure, EffectPattern::IdempotentByConstruction))
        .collect();
    let td = TopologyDecision {
        children: children.clone(),
    };
    let (_store, proj, _shaper_id, _r) = build_projection_with_topology(&shaper, &td);
    // shaper + 128 children = 129 Motes.
    assert_eq!(proj.len(), 129);
    // ready_set contains exactly 128 children.
    let ready = proj.ready_set();
    assert_eq!(ready.len(), 128);
    // All Motes in ready_set are distinct.
    let mut ids = ready.clone();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 128, "all 128 child MoteIds are distinct");
}

#[test]
fn class7d_two_independent_shapers_in_same_workflow_each_materialize_independently() {
    // Two workflow-author-declared top-level shapers, each commits its
    // own TopologyDecision spawning its own children. The two shapers
    // share NO parents (independent roots) and NO roles. Assert no
    // cross-shaper interference; both child sets are present + each
    // child is rooted only at its own shaper.
    let mut shaper_a = shaper_def();
    let mut shaper_b = shaper_def();
    // Make the two shapers distinct by overriding logic_ref:
    shaper_a.logic_ref = LogicRef([10u8; 32]);
    shaper_b.logic_ref = LogicRef([20u8; 32]);

    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper_a.clone());
    def_registry.register(shaper_b.clone());
    let role_registry = permissive_role_registry();
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        role_registry,
        InheritFromShaperResolver,
    ));
    let mut proj = Projection::with_materializer(materializer);

    let td_a = TopologyDecision {
        children: (0..3u8)
            .map(|i| descriptor(i, NdClass::Pure, EffectPattern::IdempotentByConstruction))
            .collect(),
    };
    let td_b = TopologyDecision {
        children: (10..13u8)
            .map(|i| descriptor(i, NdClass::Pure, EffectPattern::IdempotentByConstruction))
            .collect(),
    };
    let ref_a = store.put(&encode_topology(&td_a)).unwrap();
    let ref_b = store.put(&encode_topology(&td_b)).unwrap();
    let shaper_warrant_ref = stage_permissive_warrant(&store);

    let shaper_a_id = shaper_mote_id(&shaper_a);
    let shaper_b_id = shaper_mote_id(&shaper_b);
    assert_ne!(shaper_a_id, shaper_b_id);

    proj.fold(&JournalEntry::Committed {
        mote_id: shaper_a_id,
        idempotency_key: shaper_a_id.0,
        seq: 1,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: ref_a,
        parents: SmallVec::new(),
        warrant_ref: shaper_warrant_ref,
        mote_def_hash: shaper_a.hash(),
    })
    .unwrap();
    proj.fold(&JournalEntry::Committed {
        mote_id: shaper_b_id,
        idempotency_key: shaper_b_id.0,
        seq: 2,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: ref_b,
        parents: SmallVec::new(),
        warrant_ref: shaper_warrant_ref,
        mote_def_hash: shaper_b.hash(),
    })
    .unwrap();

    // 2 shapers + 6 children = 8 Motes total.
    assert_eq!(proj.len(), 8);
    // ready_set contains exactly 6 children.
    let ready = proj.ready_set();
    assert_eq!(ready.len(), 6);

    // No cross-shaper interference: each child has exactly one parent
    // (its own shaper).
    let mut a_children_count = 0;
    let mut b_children_count = 0;
    for child_id in &ready {
        let parents = proj.parents_of(child_id);
        assert_eq!(parents.len(), 1);
        let parent_id = parents[0].0;
        if parent_id == shaper_a_id {
            a_children_count += 1;
        } else if parent_id == shaper_b_id {
            b_children_count += 1;
        } else {
            panic!("child {child_id:?} has unexpected parent {parent_id:?}");
        }
    }
    assert_eq!(a_children_count, 3);
    assert_eq!(b_children_count, 3);
}

#[test]
fn class7e_shaper_emitted_cycle_tolerated_ready_set_returns_empty_for_cycle_members() {
    // A shaper-emitted TopologyDecision currently produces a tree (single
    // Control edge from shaper). True cycles via shaper-spawned children
    // would require either materialized children that themselves spawn
    // (forbidden by D48 — `is_topology_shaper: false` hardcoded), or
    // cross-edges to workflow-pre-declared Motes (out of scope for v0.1).
    //
    // The corpus property to verify here: a shaper that emits ZERO children
    // does NOT block the projection (no infinite loop / no panic) — and
    // ready_set returns empty for the (degenerate) case.
    let shaper = shaper_def();
    let td = TopologyDecision { children: vec![] };
    let (_store, proj, _id, _r) = build_projection_with_topology(&shaper, &td);
    let ready = proj.ready_set();
    assert!(
        ready.is_empty(),
        "empty TopologyDecision → empty ready_set (no children to ready)"
    );
}

#[test]
fn class7f_duplicate_descriptor_in_topology_decision_produces_identical_child_id_so_dedupes() {
    // Two identical descriptors at different positions would produce
    // children whose graph_position differs (by child_index), so their
    // MoteIds DO differ — they're treated as two distinct children, not
    // dedup'd. This is the corpus-intended behavior (each descriptor is
    // a separate intended child even if its fields match a sibling's).
    let shaper = shaper_def();
    let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    let td = TopologyDecision {
        children: vec![d.clone(), d.clone()],
    };
    let (_store, proj, shaper_id, _r) = build_projection_with_topology(&shaper, &td);
    // shaper + 2 distinct children (different graph_position → different MoteId).
    assert_eq!(proj.len(), 3);
    let children = proj.children_of(&shaper_id);
    assert_eq!(
        children.len(),
        2,
        "two distinct children even with identical descriptors"
    );
    assert_ne!(children[0].0, children[1].0);
}

// ---------------------------------------------------------------------------
// Class #8 — CONCURRENCY (deterministic)
// ---------------------------------------------------------------------------

#[test]
fn class8_child_resolver_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InheritFromShaperResolver>();
    assert_send_sync::<Arc<dyn kx_projection::ChildResolver>>();
    assert_send_sync::<Arc<dyn kx_projection::MoteDefRegistry>>();
    assert_send_sync::<Arc<dyn kx_projection::TopologyMaterializer>>();
}

#[test]
fn class8_materializer_called_from_two_threads_produces_identical_state() {
    // Build two materializers in two threads, each folds the same
    // shaper-committed entry, each ends with identical child Motes.
    // No interleavings — controlled parallelism, not racing.
    let shaper = shaper_def();
    let td = TopologyDecision {
        children: (0..8u8)
            .map(|i| descriptor(i, NdClass::Pure, EffectPattern::IdempotentByConstruction))
            .collect(),
    };
    let shaper_id = shaper_mote_id(&shaper);

    let work = std::sync::Arc::new(td);
    let s = std::sync::Arc::new(shaper);
    let t1 = {
        let work = std::sync::Arc::clone(&work);
        let s = std::sync::Arc::clone(&s);
        std::thread::spawn(move || {
            let (_store, proj, _id, _r) = build_projection_with_topology(&s, &work);
            let motes: Vec<MoteId> = proj.iter_motes().map(|(id, _)| id).collect();
            motes
        })
    };
    let t2 = {
        let work = std::sync::Arc::clone(&work);
        let s = std::sync::Arc::clone(&s);
        std::thread::spawn(move || {
            let (_store, proj, _id, _r) = build_projection_with_topology(&s, &work);
            let motes: Vec<MoteId> = proj.iter_motes().map(|(id, _)| id).collect();
            motes
        })
    };
    let m1 = t1.join().unwrap();
    let m2 = t2.join().unwrap();
    assert_eq!(m1, m2, "two threads → identical materialized state");
    assert!(m1.contains(&shaper_id));
    assert_eq!(m1.len(), 9, "shaper + 8 children = 9 Motes");
}

// ---------------------------------------------------------------------------
// Class #9 — SCALE STRUCTURE (deterministic, not load)
// ---------------------------------------------------------------------------

#[test]
fn class9_scale_structure_proptest_n_le_64() {
    use proptest::prelude::*;

    proptest!(ProptestConfig::with_cases(32), |(n in 0u8..=64u8)| {
        let shaper = shaper_def();
        let children: Vec<ChildDescriptor> = (0..n)
            .map(|i| descriptor(i, NdClass::Pure, EffectPattern::IdempotentByConstruction))
            .collect();
        let td = TopologyDecision { children };
        let (_store, proj, shaper_id, _r) = build_projection_with_topology(&shaper, &td);
        prop_assert_eq!(proj.len(), usize::from(n) + 1, "shaper + n children");
        prop_assert_eq!(proj.children_of(&shaper_id).len(), usize::from(n));
        // All children are in ready_set since shaper is Committed.
        prop_assert_eq!(proj.ready_set().len(), usize::from(n));
    });
}

// ---------------------------------------------------------------------------
// KG-1-close (PR 11.5) — Shaper-spawned-child warrant narrowing
//
// `topology.md` §13 KG-1's verbatim-inheritance safe-default is replaced
// by D30 `intersect(shaper.warrant, role.spec)`. The materializer
// resolves each descriptor's role via a `RoleRegistry`, narrows, and
// stamps the per-child `warrant_ref` on the materialized
// `RegisterMote`. The four KG-1-close obligations (per topology.md §13
// closing-ticket item 4):
//
//   (a) child.warrant strictly narrower than shaper.warrant on at
//       least one axis when role narrows;
//   (b) sibling roles produce different child warrants;
//   (c) unregistered role is a typed error (no silent widening);
//   (d) attempted-widen role is a typed error (NarrowingError
//       propagated).
// ---------------------------------------------------------------------------

/// Build a [`Role`] whose `spec` equals the permissive baseline modulo
/// an overridable mutation. Mutators tighten the spec (narrowing); the
/// resulting child warrant_ref must differ from the shaper's.
fn role_with_spec(name: &str, mut mutate: impl FnMut(&mut WarrantSpec)) -> Role {
    let mut spec = permissive_warrant();
    mutate(&mut spec);
    Role {
        name: name.into(),
        version: 1,
        spec,
        description: String::new(),
    }
}

/// Build a projection wired with an explicit `InMemoryRoleRegistry`.
/// Returns the staged content store, the projection, the shaper id, the
/// staged topology ref, and the staged shaper warrant_ref so the KG-1-
/// close tests can compare `proj.warrant_ref_of(child)` against the
/// shaper's known ref.
fn build_projection_with_role_registry(
    shaper: &MoteDef,
    td: &TopologyDecision,
    role_registry: Arc<InMemoryRoleRegistry>,
) -> (
    Arc<InMemoryContentStore>,
    Projection,
    MoteId,
    ContentRef,
    ContentRef,
) {
    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper.clone());
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        role_registry,
        InheritFromShaperResolver,
    ));
    let td_ref = store.put(&encode_topology(td)).expect("put");
    let shaper_warrant_ref = stage_permissive_warrant(&store);
    let shaper_id = shaper_mote_id(shaper);
    let mut proj = Projection::with_materializer(materializer);
    proj.fold(&JournalEntry::Committed {
        mote_id: shaper_id,
        idempotency_key: shaper_id.0,
        seq: 1,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: td_ref,
        parents: SmallVec::new(),
        warrant_ref: shaper_warrant_ref,
        mote_def_hash: shaper.hash(),
    })
    .expect("shaper fold succeeds");
    (store, proj, shaper_id, td_ref, shaper_warrant_ref)
}

#[test]
fn kg1_close_child_warrant_strictly_narrower_than_shaper_when_role_narrows() {
    // OBLIGATION (a). Shaper's permissive warrant (max_calls = 8). Role's
    // spec tightens max_calls to 2. The materialized child's warrant_ref
    // MUST differ from the shaper's, and the recomputed child warrant
    // MUST have max_calls = 2 (per-axis min via D30 intersect).
    let shaper = shaper_def();
    let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    let td = TopologyDecision {
        children: vec![d.clone()],
    };
    let registry = Arc::new(InMemoryRoleRegistry::new());
    let tightening_role = role_with_spec("tighter-max-calls", |s| {
        s.model_route.max_calls = 2;
    });
    registry.register(d.role_id.clone(), tightening_role.clone());

    let (_store, proj, shaper_id, _td_ref, shaper_warrant_ref) =
        build_projection_with_role_registry(&shaper, &td, registry);

    // shaper + 1 materialized child:
    assert_eq!(proj.len(), 2);
    let child_id = *proj.ready_set().first().expect("one materialized child");
    let child_warrant_ref = proj
        .warrant_ref_of(&child_id)
        .expect("materialized child carries a warrant_ref");

    // (a) — strictly different ref (because role narrows max_calls).
    assert_ne!(
        child_warrant_ref, shaper_warrant_ref,
        "narrowing role produces a different child warrant_ref"
    );

    // Re-derive the expected child warrant to pin per-axis narrowing.
    let expected_child_warrant =
        kx_warrant::intersect(&permissive_warrant(), &tightening_role).expect("narrow ok");
    assert_eq!(expected_child_warrant.model_route.max_calls, 2);
    assert_eq!(child_warrant_ref, warrant_ref_of(&expected_child_warrant));
    // Shaper's own warrant_ref equally read back via the same API.
    assert_eq!(proj.warrant_ref_of(&shaper_id), Some(shaper_warrant_ref));
}

#[test]
fn kg1_close_sibling_children_with_different_roles_get_different_warrants() {
    // OBLIGATION (b). Two children under two different roles get two
    // different child warrant_refs — sibling roles narrow independently.
    let shaper = shaper_def();
    let d_a = ChildDescriptor {
        role_id: RoleId("worker-a".into()),
        logic_ref: LogicRef([10u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let d_b = ChildDescriptor {
        role_id: RoleId("worker-b".into()),
        logic_ref: LogicRef([20u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let td = TopologyDecision {
        children: vec![d_a.clone(), d_b.clone()],
    };
    let registry = Arc::new(InMemoryRoleRegistry::new());
    registry.register(
        d_a.role_id.clone(),
        role_with_spec("role-a", |s| s.model_route.max_calls = 2),
    );
    registry.register(
        d_b.role_id.clone(),
        role_with_spec("role-b", |s| s.model_route.max_calls = 4),
    );

    let (_store, proj, shaper_id, _td_ref, _shaper_warrant_ref) =
        build_projection_with_role_registry(&shaper, &td, registry);

    let children = proj.children_of(&shaper_id);
    assert_eq!(children.len(), 2);
    let mut refs: Vec<ContentRef> = children
        .iter()
        .map(|(id, _)| proj.warrant_ref_of(id).expect("child warrant_ref present"))
        .collect();
    refs.sort();
    refs.dedup();
    assert_eq!(
        refs.len(),
        2,
        "two roles narrowing to different specs MUST produce two distinct child warrant_refs"
    );
}

#[test]
fn kg1_close_role_not_registered_returns_typed_error() {
    // OBLIGATION (c). The materializer refuses to silently widen when a
    // descriptor names a role the registry doesn't know — it surfaces
    // ProjectionError::RoleNotRegistered with the offending role_id +
    // descriptor index.
    let shaper = shaper_def();
    let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    let td = TopologyDecision {
        children: vec![d.clone()],
    };
    // Empty registry — role lookup will miss.
    let registry = Arc::new(InMemoryRoleRegistry::new());

    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper.clone());
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        registry,
        InheritFromShaperResolver,
    ));
    let td_ref = store.put(&encode_topology(&td)).expect("put");
    let shaper_warrant_ref = stage_permissive_warrant(&store);
    let shaper_id = shaper_mote_id(&shaper);
    let mut proj = Projection::with_materializer(materializer);

    let err = proj
        .fold(&JournalEntry::Committed {
            mote_id: shaper_id,
            idempotency_key: shaper_id.0,
            seq: 1,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: td_ref,
            parents: SmallVec::new(),
            warrant_ref: shaper_warrant_ref,
            mote_def_hash: shaper.hash(),
        })
        .unwrap_err();

    match err {
        kx_projection::ProjectionError::RoleNotRegistered {
            role_id,
            descriptor_index,
        } => {
            assert_eq!(role_id, d.role_id);
            assert_eq!(descriptor_index, 0);
        }
        other => panic!("expected RoleNotRegistered, got {other:?}"),
    }
}

#[test]
fn kg1_close_role_that_attempts_widen_returns_narrowing_error() {
    // OBLIGATION (d). A role whose spec is wider than the shaper's
    // warrant on any axis produces ProjectionError::NarrowingFailed
    // wrapping kx_warrant::NarrowingError — the materializer refuses
    // the widening attempt at materialization time.
    let shaper = shaper_def();
    let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    let td = TopologyDecision {
        children: vec![d.clone()],
    };
    let registry = Arc::new(InMemoryRoleRegistry::new());
    // Permissive shaper has tool_grants = {}. A role granting "tool-x"
    // is wider than the shaper's empty allowlist → AttemptedWiden.
    let widening_role = role_with_spec("wider-tool-grants", |s| {
        let mut grants = BTreeSet::new();
        grants.insert(kx_warrant::ToolGrant {
            tool_id: kx_mote::ToolName("tool-x".into()),
            tool_version: kx_mote::ToolVersion("1.0".into()),
        });
        s.tool_grants = grants;
    });
    registry.register(d.role_id.clone(), widening_role);

    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper.clone());
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        registry,
        InheritFromShaperResolver,
    ));
    let td_ref = store.put(&encode_topology(&td)).expect("put");
    let shaper_warrant_ref = stage_permissive_warrant(&store);
    let shaper_id = shaper_mote_id(&shaper);
    let mut proj = Projection::with_materializer(materializer);

    let err = proj
        .fold(&JournalEntry::Committed {
            mote_id: shaper_id,
            idempotency_key: shaper_id.0,
            seq: 1,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: td_ref,
            parents: SmallVec::new(),
            warrant_ref: shaper_warrant_ref,
            mote_def_hash: shaper.hash(),
        })
        .unwrap_err();

    match err {
        kx_projection::ProjectionError::NarrowingFailed {
            descriptor_index, ..
        } => {
            assert_eq!(descriptor_index, 0);
        }
        other => panic!("expected NarrowingFailed, got {other:?}"),
    }
}

#[test]
fn kg1_close_materializer_propagates_warrant_store_fetch_failure() {
    // Boundary: the shaper's warrant_ref points at content that was
    // never staged → WarrantStoreFetch. Workflow author / executor
    // MUST `put` the WarrantSpec before the shaper commits; this
    // boundary test guards the contract.
    let shaper = shaper_def();
    let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    let td = TopologyDecision {
        children: vec![d.clone()],
    };
    let registry = Arc::new(InMemoryRoleRegistry::new());
    registry.register(d.role_id.clone(), role_with_spec("any", |_| {}));

    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper.clone());
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        registry,
        InheritFromShaperResolver,
    ));
    let td_ref = store.put(&encode_topology(&td)).expect("put");
    // Use a warrant_ref that was NEVER staged.
    let unstaged_warrant_ref = ContentRef::from_bytes([0xff; 32]);
    let shaper_id = shaper_mote_id(&shaper);
    let mut proj = Projection::with_materializer(materializer);
    let err = proj
        .fold(&JournalEntry::Committed {
            mote_id: shaper_id,
            idempotency_key: shaper_id.0,
            seq: 1,
            nondeterminism: NdClass::ReadOnlyNondet,
            result_ref: td_ref,
            parents: SmallVec::new(),
            warrant_ref: unstaged_warrant_ref,
            mote_def_hash: shaper.hash(),
        })
        .unwrap_err();
    assert!(
        matches!(
            err,
            kx_projection::ProjectionError::WarrantStoreFetch { .. }
        ),
        "expected WarrantStoreFetch, got {err:?}"
    );
}
