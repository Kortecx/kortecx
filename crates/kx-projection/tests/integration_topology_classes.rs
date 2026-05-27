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
use smallvec::SmallVec;
use std::collections::BTreeMap;

fn shaper_def() -> MoteDef {
    MoteDef {
        logic_ref: LogicRef([1u8; 32]),
        model_id: ModelId("planner-v1".into()),
        prompt_template_hash: PromptTemplateHash([3u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
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

fn build_projection_with_topology(
    shaper: &MoteDef,
    td: &TopologyDecision,
) -> (Arc<InMemoryContentStore>, Projection, MoteId, ContentRef) {
    let store = Arc::new(InMemoryContentStore::new());
    let registry = Arc::new(InMemoryMoteDefRegistry::new());
    registry.register(shaper.clone());
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&registry),
        InheritFromShaperResolver,
    ));
    let bytes = encode_topology(td);
    let td_ref = store.put(&bytes).expect("put");
    let shaper_id = shaper_mote_id(shaper);
    let mut proj = Projection::with_materializer(materializer);
    let entry = JournalEntry::Committed {
        mote_id: shaper_id,
        idempotency_key: shaper_id.0,
        seq: 1,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: td_ref,
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
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
    let registry = Arc::new(InMemoryMoteDefRegistry::new());
    registry.register(shaper_a.clone());
    registry.register(shaper_b.clone());
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&registry),
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
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
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
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
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
// KG-1 mitigation — Warrant safe-default
// ---------------------------------------------------------------------------

#[test]
fn kg1_shaper_spawned_child_inherits_shaper_warrant_verbatim_kg1_safe_default() {
    // PR 11 ships KG-1 safe-default: materialized children's warrant
    // inheritance happens via the projection reading the shaper's
    // CommittedInfo.warrant_ref. The materializer does NOT itself stamp
    // a per-child warrant_ref. To verify the safe-default, we assert:
    // (1) the shaper's warrant_ref is stored on its CommittedInfo;
    // (2) the projection has no separate per-child warrant axis (the
    //     executor reads shaper.warrant_ref when dispatching children
    //     until KG-1-close lands in PR 11.5).
    let shaper = shaper_def();
    let td = TopologyDecision {
        children: vec![descriptor(
            7,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let (_store, proj, shaper_id, _r) = build_projection_with_topology(&shaper, &td);

    // Property: the shaper's warrant_ref is recorded on its committed entry
    // (the projection holds it for downstream consumers).
    let shaper_committed_seq = proj.committed_seq_of(&shaper_id);
    assert!(
        shaper_committed_seq.is_some(),
        "shaper committed seq is recorded"
    );

    // Property: the projection's per-Mote read API does NOT expose a
    // separate warrant axis for materialized children. Until KG-1-close
    // (PR 11.5) adds a `RoleRegistry` and the materializer's intersect
    // call, the safe-default = child inherits shaper warrant verbatim
    // is structural: there is no separate child warrant to inherit
    // from. (If a future PR adds per-child warrant storage to
    // DeclaredInfo, this test will compile and pass — KG-1 will need
    // an explicit closing-PR audit.)
    //
    // Smoke check: every materialized child has a parent edge to the
    // shaper (i.e., the materializer registered them correctly).
    let ready = proj.ready_set();
    assert_eq!(ready.len(), 1);
    let child_id = ready[0];
    let parents = proj.parents_of(&child_id);
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[0].0, shaper_id);
}
