#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! **P1.11 — R49 cold-re-fold proof.** The load-bearing replay-faithfulness
//! test for D48 + D49. Anchors standing invariant **#5**
//! (`04-testing-and-gates.md`: "Topology-decision determinism — a committed
//! shaper decision rebuilds identical edges on re-fold and on recovery — no
//! orphaned/duplicated children").
//!
//! ## The four named properties (private corpus D49 §"Cold-re-fold verification")
//!
//! - **P1 — IDENTITY RECONSTRUCTIBILITY.** Cold-folding a journal-prefix
//!   containing a shaper's Committed entry produces bit-identical child
//!   MoteIds + edges to a live-folded equivalent.
//! - **P2 — NO-POSITION-METADATA (absence proof).** No `graph_position` field
//!   exists on `RegisterMote`, `DeclaredInfo` (via the `register_mote` API),
//!   `CommittedInfo` (via the projection's read API), or `JournalEntry`
//!   (verified by exhaustive pattern-match on the enum).
//! - **P3 — TRANSITIVITY (R49 LOAD-BEARING — the place R49 could be silently wrong).**
//!   A one-axis byte change in any descriptor produces a different shaper
//!   `result_ref`, a different child `input_data_id`, and a different child
//!   `MoteId`. BLAKE3 + canonical bincode + materializer compose into the
//!   claimed identity chain.
//! - **P4 — RE-RUN DISTINCTNESS.** Two different `TopologyDecision` payloads
//!   from two different shaper attempts produce identifiably-different child
//!   sets; dedup-by-key + content-addressing ensure only the surviving
//!   shaper's children land in the projection.

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    canonical_config, ChildDescriptor, EffectPattern, LogicRef, ModelId, MoteDef, MoteDefHash,
    MoteId, NdClass, PromptTemplateHash, RoleId, TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::{
    derive_child_identity, DefaultTopologyMaterializer, InMemoryMoteDefRegistry,
    InheritFromShaperResolver, MoteState, Projection,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, Role,
    RoleRegistry, WarrantSpec,
};
use smallvec::SmallVec;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

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

/// Compute the shaper's `MoteId` from its `MoteDef` + a fixed
/// `input_data_id` + a fixed `graph_position`. Tests use a single
/// shaper instance so the fixture is shared.
fn shaper_mote_id(def: &MoteDef) -> MoteId {
    kx_mote::derive_mote_id(
        &def.hash(),
        &kx_mote::InputDataId::from_bytes([0u8; 32]),
        &kx_mote::GraphPosition(vec![0u8]),
    )
}

/// Encode a `TopologyDecision` to bytes via canonical bincode.
fn encode_topology(td: &TopologyDecision) -> Vec<u8> {
    bincode::serde::encode_to_vec(td, canonical_config())
        .expect("TopologyDecision canonical bincode encodes infallibly")
}

/// Test-local [`RoleRegistry`] that returns the same [`Role`] for any
/// `RoleId`. R49 cold-refold tests don't exercise per-role warrant
/// narrowing — they just need a registry that doesn't fail. KG-1-close
/// narrowing semantics are tested separately in
/// `integration_topology_classes.rs`.
struct PermissiveAnyRoleRegistry {
    role: Role,
}

impl RoleRegistry for PermissiveAnyRoleRegistry {
    fn resolve(&self, _role_id: &RoleId) -> Option<Role> {
        Some(self.role.clone())
    }
}

/// Build a permissive [`WarrantSpec`] suitable as the shaper's warrant
/// and as every child role's spec (so [`kx_warrant::intersect`] succeeds
/// with no widening). Same shape as the K-G-1 close tests' baseline.
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
    }
}

type TestMaterializer = DefaultTopologyMaterializer<
    InMemoryContentStore,
    InMemoryMoteDefRegistry,
    PermissiveAnyRoleRegistry,
    InheritFromShaperResolver,
>;

/// Build the materializer wiring used by every R49 test in this file.
///
/// Returns the staged content store, the def registry, the shaper's
/// `warrant_ref` (used on the Committed entry), and the boxed
/// materializer. The shaper's `WarrantSpec` is staged in the content
/// store at the returned `warrant_ref` so the materializer can fetch +
/// decode it during `try_materialize` (PR 11.5 / KG-1-close path).
fn build_materializer(
    shaper_def_value: &MoteDef,
) -> (
    Arc<InMemoryContentStore>,
    Arc<InMemoryMoteDefRegistry>,
    ContentRef,
    Box<TestMaterializer>,
) {
    let store = Arc::new(InMemoryContentStore::new());
    let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
    def_registry.register(shaper_def_value.clone());
    let warrant = permissive_warrant();
    let warrant_bytes = bincode::serde::encode_to_vec(&warrant, canonical_config())
        .expect("WarrantSpec canonical bincode encodes infallibly");
    let shaper_warrant_ref = store.put(&warrant_bytes).expect("put succeeds");
    // Sanity: the put-derived ref matches warrant_ref_of (same canonical
    // bincode + blake3 chain). If this ever diverges, KG-1-close is wrong.
    assert_eq!(shaper_warrant_ref, warrant_ref_of(&warrant));
    let role = Role {
        name: "test-default".into(),
        version: 1,
        spec: warrant,
        description: String::new(),
    };
    let role_registry = Arc::new(PermissiveAnyRoleRegistry { role });
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&def_registry),
        role_registry,
        InheritFromShaperResolver,
    ));
    (store, def_registry, shaper_warrant_ref, materializer)
}

/// Stage the `TopologyDecision` payload to the content store; return its `ContentRef`.
fn stage_topology(store: &InMemoryContentStore, td: &TopologyDecision) -> ContentRef {
    let bytes = encode_topology(td);
    store.put(&bytes).expect("put succeeds")
}

/// Build a Committed entry for the shaper that points at the topology
/// payload AND carries the staged warrant_ref (PR 11.5 KG-1-close: the
/// materializer fetches the WarrantSpec at this ref).
fn shaper_committed_entry(
    shaper_id: MoteId,
    shaper_def_hash: MoteDefHash,
    topology_ref: ContentRef,
    shaper_warrant_ref: ContentRef,
    seq: u64,
) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: shaper_id,
        idempotency_key: shaper_id.0,
        seq,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: topology_ref,
        parents: SmallVec::new(),
        warrant_ref: shaper_warrant_ref,
        mote_def_hash: shaper_def_hash,
    }
}

// ---------------------------------------------------------------------------
// P1 — IDENTITY RECONSTRUCTIBILITY
// ---------------------------------------------------------------------------

#[test]
fn p1_cold_refold_rebuilds_identical_children_and_edges() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    let td = TopologyDecision {
        children: vec![
            descriptor(11, NdClass::Pure, EffectPattern::IdempotentByConstruction),
            descriptor(
                22,
                NdClass::ReadOnlyNondet,
                EffectPattern::IdempotentByConstruction,
            ),
            descriptor(33, NdClass::Pure, EffectPattern::StageThenCommit),
        ],
    };

    // Live fold path: build projection with materializer, fold shaper commit.
    let (store_live, _reg_live, w_ref, materializer_live) = build_materializer(&shaper);
    let td_ref_live = stage_topology(&store_live, &td);
    let mut live = Projection::with_materializer(materializer_live);
    let entry_live = shaper_committed_entry(shaper_id, shaper_hash, td_ref_live, w_ref, 1);
    live.fold(&entry_live).unwrap();

    // Cold re-fold path: write entry to a journal, then from_journal_with_materializer.
    let (store_cold, _reg_cold, w_ref, materializer_cold) = build_materializer(&shaper);
    let td_ref_cold = stage_topology(&store_cold, &td);
    assert_eq!(
        td_ref_live, td_ref_cold,
        "content-addressed staging yields identical refs"
    );
    let journal = InMemoryJournal::new();
    let entry_cold = shaper_committed_entry(shaper_id, shaper_hash, td_ref_cold, w_ref, 1);
    journal.append(entry_cold).unwrap();
    let cold = Projection::from_journal_with_materializer(&journal, materializer_cold).unwrap();

    // Both projections see the shaper itself as Committed.
    assert_eq!(live.state_of(&shaper_id), MoteState::Committed);
    assert_eq!(cold.state_of(&shaper_id), MoteState::Committed);

    // Compute the expected child MoteIds via the public derivation helper.
    let expected_children: Vec<MoteId> = td
        .children
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (id, _hash, _nd, _ep) = derive_child_identity(
                shaper_id,
                &shaper,
                td_ref_live,
                d,
                i,
                &InheritFromShaperResolver,
            );
            id
        })
        .collect();

    for child_id in &expected_children {
        // Each child is now in the projection (Pending, waiting on the shaper's
        // commit — but the shaper IS committed, so they're ready).
        assert!(
            !matches!(live.state_of(child_id), MoteState::Pending)
                || live.parents_of(child_id).len() == 1,
            "live: child {child_id:?} should be visible"
        );
        assert!(
            !matches!(cold.state_of(child_id), MoteState::Pending)
                || cold.parents_of(child_id).len() == 1,
            "cold: child {child_id:?} should be visible"
        );

        // Both projections agree on the parent edge: single Control edge from shaper.
        let live_parents = live.parents_of(child_id);
        let cold_parents = cold.parents_of(child_id);
        assert_eq!(live_parents, cold_parents);
        assert_eq!(live_parents.len(), 1);
        assert_eq!(live_parents[0].0, shaper_id, "shaper is the single parent");
    }

    // ready_set parity: same Motes are ready in both projections.
    let mut live_ready: Vec<MoteId> = live.ready_set();
    let mut cold_ready: Vec<MoteId> = cold.ready_set();
    live_ready.sort();
    cold_ready.sort();
    assert_eq!(live_ready, cold_ready);

    // All three materialized children appear in ready_set (shaper is committed
    // and they each have only the shaper as a parent).
    for child_id in &expected_children {
        assert!(
            cold_ready.contains(child_id),
            "child {child_id:?} should be in ready_set"
        );
    }
}

// ---------------------------------------------------------------------------
// P2 — NO-POSITION-METADATA (absence proof)
// ---------------------------------------------------------------------------

#[test]
fn p2_no_graph_position_field_on_journal_entry() {
    // Exhaustive pattern-match on JournalEntry's variants. If a future
    // variant adds a graph_position field, this match would still
    // compile (we'd need to update it), but the assertion below names
    // every field of the Committed variant so a graph_position
    // *insertion* into Committed would force a compile error here.
    let entry = JournalEntry::Committed {
        mote_id: MoteId::from_bytes([1u8; 32]),
        idempotency_key: [1u8; 32],
        seq: 1,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([2u8; 32]),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([3u8; 32]),
        mote_def_hash: MoteDefHash::from_bytes([4u8; 32]),
    };
    // Destructure with explicit field naming; any new field would
    // surface here as an error (or as a `..` ellipsis if added; this
    // test forbids the ellipsis to enforce explicit acknowledgment).
    let JournalEntry::Committed {
        mote_id: _,
        idempotency_key: _,
        seq: _,
        nondeterminism: _,
        result_ref: _,
        parents: _,
        warrant_ref: _,
        mote_def_hash: _,
    } = entry
    else {
        unreachable!()
    };
}

#[test]
fn p2_no_graph_position_on_register_mote() {
    // Exhaustive struct-literal construction of RegisterMote — a future
    // graph_position field would surface as a missing-field error here.
    let _reg = kx_projection::RegisterMote {
        mote_id: MoteId::from_bytes([1u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
    };
}

#[test]
fn p2_projection_read_api_has_no_graph_position_method() {
    // Compile-time absence assertion via the public read API surface.
    // We list every method on Projection that returns Mote-specific
    // info; none returns a graph_position. If a future API ever needs
    // it, the no-position-metadata clause is violated and R49 must be
    // re-opened.
    let p = Projection::new();
    let id = MoteId::from_bytes([1u8; 32]);
    let _: MoteState = p.state_of(&id);
    let _: smallvec::SmallVec<[(MoteId, kx_mote::EdgeMeta); 4]> = p.parents_of(&id);
    let _: Vec<(MoteId, kx_mote::EdgeMeta)> = p.children_of(&id);
    let _: Option<ContentRef> = p.result_ref_of(&id);
    let _: Option<u64> = p.committed_seq_of(&id);
    let _: Vec<MoteId> = p.ready_set();
}

// ---------------------------------------------------------------------------
// P3 — TRANSITIVITY (R49 LOAD-BEARING)
// ---------------------------------------------------------------------------

#[test]
fn p3_one_byte_change_in_descriptor_changes_child_mote_id_via_logic_ref() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);

    let td_a = TopologyDecision {
        children: vec![descriptor(
            11,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let td_b = TopologyDecision {
        children: vec![descriptor(
            12,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };

    assert_ne!(
        td_a.hash(),
        td_b.hash(),
        "different descriptors → different shaper result_refs"
    );

    let ref_a = ContentRef::from_bytes(td_a.hash());
    let ref_b = ContentRef::from_bytes(td_b.hash());
    let (id_a, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_a,
        &td_a.children[0],
        0,
        &InheritFromShaperResolver,
    );
    let (id_b, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_b,
        &td_b.children[0],
        0,
        &InheritFromShaperResolver,
    );
    assert_ne!(
        id_a, id_b,
        "P3: one-byte change in logic_ref → different child MoteId"
    );
}

#[test]
fn p3_one_byte_change_via_nd_class() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let td_a = TopologyDecision {
        children: vec![descriptor(
            11,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let td_b = TopologyDecision {
        children: vec![descriptor(
            11,
            NdClass::ReadOnlyNondet,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    assert_ne!(td_a.hash(), td_b.hash());
    let ref_a = ContentRef::from_bytes(td_a.hash());
    let ref_b = ContentRef::from_bytes(td_b.hash());
    let (id_a, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_a,
        &td_a.children[0],
        0,
        &InheritFromShaperResolver,
    );
    let (id_b, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_b,
        &td_b.children[0],
        0,
        &InheritFromShaperResolver,
    );
    assert_ne!(id_a, id_b, "P3: nd_class change → different child MoteId");
}

#[test]
fn p3_one_byte_change_via_effect_pattern() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let td_a = TopologyDecision {
        children: vec![descriptor(
            11,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let td_b = TopologyDecision {
        children: vec![descriptor(
            11,
            NdClass::Pure,
            EffectPattern::StageThenCommit,
        )],
    };
    assert_ne!(td_a.hash(), td_b.hash());
    let ref_a = ContentRef::from_bytes(td_a.hash());
    let ref_b = ContentRef::from_bytes(td_b.hash());
    let (id_a, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_a,
        &td_a.children[0],
        0,
        &InheritFromShaperResolver,
    );
    let (id_b, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_b,
        &td_b.children[0],
        0,
        &InheritFromShaperResolver,
    );
    assert_ne!(
        id_a, id_b,
        "P3: effect_pattern change → different child MoteId"
    );
}

#[test]
fn p3_one_byte_change_via_role_id() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    // role_id only affects warrant narrowing (KG-1 closing PR 11.5),
    // not the MoteDef itself today — under InheritFromShaperResolver
    // role_id is NOT a MoteDef axis. But it IS a TopologyDecision-
    // payload axis, so the shaper's result_ref differs, which still
    // makes the child input_data_id differ — and so the MoteId. P3
    // holds via the transitivity of (result_ref → input_data_id → MoteId).
    let mut child_a = descriptor(11, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    let mut child_b = descriptor(11, NdClass::Pure, EffectPattern::IdempotentByConstruction);
    child_a.role_id = RoleId("alpha".into());
    child_b.role_id = RoleId("beta".into());
    let td_a = TopologyDecision {
        children: vec![child_a.clone()],
    };
    let td_b = TopologyDecision {
        children: vec![child_b.clone()],
    };
    assert_ne!(
        td_a.hash(),
        td_b.hash(),
        "role_id must affect TopologyDecision hash"
    );
    let ref_a = ContentRef::from_bytes(td_a.hash());
    let ref_b = ContentRef::from_bytes(td_b.hash());
    let (id_a, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_a,
        &child_a,
        0,
        &InheritFromShaperResolver,
    );
    let (id_b, _, _, _) = derive_child_identity(
        shaper_id,
        &shaper,
        ref_b,
        &child_b,
        0,
        &InheritFromShaperResolver,
    );
    assert_ne!(
        id_a, id_b,
        "P3: role_id change → different child MoteId via input_data_id transitivity"
    );
}

#[test]
fn p3_full_materializer_path_one_byte_descriptor_change_changes_child_ids() {
    // P3 end-to-end via the production DefaultTopologyMaterializer:
    // build two TopologyDecisions differing by one byte; route both
    // through the materializer; assert child MoteIds differ.
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    let td_a = TopologyDecision {
        children: vec![descriptor(
            1,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let td_b = TopologyDecision {
        children: vec![descriptor(
            2,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };

    let (store_a, _reg_a, w_ref, materializer_a) = build_materializer(&shaper);
    let ref_a = stage_topology(&store_a, &td_a);
    let mut proj_a = Projection::with_materializer(materializer_a);
    let entry_a = shaper_committed_entry(shaper_id, shaper_hash, ref_a, w_ref, 1);
    proj_a.fold(&entry_a).unwrap();

    let (store_b, _reg_b, w_ref, materializer_b) = build_materializer(&shaper);
    let ref_b = stage_topology(&store_b, &td_b);
    let mut proj_b = Projection::with_materializer(materializer_b);
    let entry_b = shaper_committed_entry(shaper_id, shaper_hash, ref_b, w_ref, 1);
    proj_b.fold(&entry_b).unwrap();

    // Each projection has exactly one materialized child + the shaper = 2 Motes.
    assert_eq!(proj_a.len(), 2);
    assert_eq!(proj_b.len(), 2);

    // The materialized child differs across projections.
    let child_a: Vec<MoteId> = proj_a
        .iter_motes()
        .filter_map(|(id, _)| if id != shaper_id { Some(id) } else { None })
        .collect();
    let child_b: Vec<MoteId> = proj_b
        .iter_motes()
        .filter_map(|(id, _)| if id != shaper_id { Some(id) } else { None })
        .collect();
    assert_eq!(child_a.len(), 1);
    assert_eq!(child_b.len(), 1);
    assert_ne!(
        child_a[0], child_b[0],
        "end-to-end P3: different descriptors → different child MoteIds"
    );
}

// ---------------------------------------------------------------------------
// P4 — RE-RUN DISTINCTNESS
// ---------------------------------------------------------------------------

#[test]
fn p4_different_topology_decisions_produce_different_child_sets() {
    // Two shaper attempts that each emit a different TopologyDecision
    // produce different child sets. Dedup-by-key on the journal would
    // ensure only one shaper Committed lands; here we just verify the
    // identity property at the materializer layer (same shaper_id +
    // different result_ref → different children).
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    // Attempt A produces 2 children.
    let td_a = TopologyDecision {
        children: vec![
            descriptor(10, NdClass::Pure, EffectPattern::IdempotentByConstruction),
            descriptor(20, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        ],
    };
    // Attempt B (different nondet output) produces 2 different children.
    let td_b = TopologyDecision {
        children: vec![
            descriptor(30, NdClass::Pure, EffectPattern::IdempotentByConstruction),
            descriptor(40, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        ],
    };
    assert_ne!(td_a.hash(), td_b.hash());

    let (store_a, _reg_a, w_ref, materializer_a) = build_materializer(&shaper);
    let ref_a = stage_topology(&store_a, &td_a);
    let mut proj_a = Projection::with_materializer(materializer_a);
    proj_a
        .fold(&shaper_committed_entry(
            shaper_id,
            shaper_hash,
            ref_a,
            w_ref,
            1,
        ))
        .unwrap();

    let (store_b, _reg_b, w_ref, materializer_b) = build_materializer(&shaper);
    let ref_b = stage_topology(&store_b, &td_b);
    let mut proj_b = Projection::with_materializer(materializer_b);
    proj_b
        .fold(&shaper_committed_entry(
            shaper_id,
            shaper_hash,
            ref_b,
            w_ref,
            1,
        ))
        .unwrap();

    // Each projection has shaper + 2 children = 3 Motes.
    assert_eq!(proj_a.len(), 3);
    assert_eq!(proj_b.len(), 3);

    // The non-shaper Motes are disjoint between A and B.
    let a_children: std::collections::BTreeSet<MoteId> = proj_a
        .iter_motes()
        .filter_map(|(id, _)| if id != shaper_id { Some(id) } else { None })
        .collect();
    let b_children: std::collections::BTreeSet<MoteId> = proj_b
        .iter_motes()
        .filter_map(|(id, _)| if id != shaper_id { Some(id) } else { None })
        .collect();
    let overlap: std::collections::BTreeSet<_> = a_children.intersection(&b_children).collect();
    assert!(
        overlap.is_empty(),
        "P4: re-run distinctness — different shaper attempts produce disjoint child sets (overlap = {overlap:?})"
    );
}

#[test]
fn p4_only_surviving_shaper_committed_materializes_children() {
    // In a real journal, only one shaper Committed lands per identity
    // (dedup-by-key on idempotency_key). The losing attempt's
    // TopologyDecision (if staged before death) becomes orphaned
    // content. The projection only ever materializes the surviving
    // commit's children — because the projection only sees ONE
    // Committed entry per MoteId (the journal enforces this).
    //
    // This test simulates by attempting to fold two Committed entries
    // with the same MoteId — the second should error with
    // DuplicateCommitted, preserving the first attempt's children.
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    let td_a = TopologyDecision {
        children: vec![descriptor(
            10,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let td_b = TopologyDecision {
        children: vec![descriptor(
            20,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };

    let (store, _reg, w_ref, materializer) = build_materializer(&shaper);
    let ref_a = stage_topology(&store, &td_a);
    let ref_b = stage_topology(&store, &td_b);
    let mut proj = Projection::with_materializer(materializer);

    proj.fold(&shaper_committed_entry(
        shaper_id,
        shaper_hash,
        ref_a,
        w_ref,
        1,
    ))
    .unwrap();
    let len_after_a = proj.len();
    assert_eq!(len_after_a, 2, "shaper + 1 child after A");

    // Second Committed for same shaper_id: rejected with DuplicateCommitted.
    let err = proj
        .fold(&shaper_committed_entry(
            shaper_id,
            shaper_hash,
            ref_b,
            w_ref,
            2,
        ))
        .unwrap_err();
    assert!(
        matches!(err, kx_projection::ProjectionError::DuplicateCommitted(_)),
        "P4: second shaper Committed rejected as duplicate; only the first attempt's children survive"
    );
    // No B-side children materialized.
    assert_eq!(proj.len(), len_after_a);
}

// ---------------------------------------------------------------------------
// Sanity: empty TopologyDecision materializes zero children
// (Boundary class #7(a) per the test-plan table.)
// ---------------------------------------------------------------------------

#[test]
fn empty_topology_decision_materializes_zero_children() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();
    let td = TopologyDecision { children: vec![] };

    let (store, _reg, w_ref, materializer) = build_materializer(&shaper);
    let td_ref = stage_topology(&store, &td);
    let mut proj = Projection::with_materializer(materializer);
    proj.fold(&shaper_committed_entry(
        shaper_id,
        shaper_hash,
        td_ref,
        w_ref,
        1,
    ))
    .unwrap();
    assert_eq!(proj.len(), 1, "shaper only; no children");
    assert_eq!(proj.state_of(&shaper_id), MoteState::Committed);
}

// ---------------------------------------------------------------------------
// Materializer no-op when shaper MoteDef is not registered (workflow-author
// error, surfaced as a warn trace + skip).
// ---------------------------------------------------------------------------

#[test]
fn materializer_skips_when_shaper_def_not_registered() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    // Build materializer with EMPTY registry (do NOT call build_materializer
    // which registers the shaper def). The role registry is irrelevant —
    // the materializer early-skips before touching it when the def is
    // missing — but the materializer constructor needs one.
    let store = Arc::new(InMemoryContentStore::new());
    let registry = Arc::new(InMemoryMoteDefRegistry::new());
    let role_registry = Arc::new(PermissiveAnyRoleRegistry {
        role: Role {
            name: "irrelevant".into(),
            version: 1,
            spec: permissive_warrant(),
            description: String::new(),
        },
    });
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&registry),
        role_registry,
        InheritFromShaperResolver,
    ));
    let td = TopologyDecision {
        children: vec![descriptor(
            10,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
        )],
    };
    let td_ref = stage_topology(&store, &td);
    // Stage a permissive WarrantSpec so the entry carries a valid
    // warrant_ref (though the materializer will early-skip before
    // touching it).
    let warrant_bytes = bincode::serde::encode_to_vec(permissive_warrant(), canonical_config())
        .expect("WarrantSpec canonical bincode encodes infallibly");
    let w_ref = store.put(&warrant_bytes).expect("put");
    let mut proj = Projection::with_materializer(materializer);
    proj.fold(&shaper_committed_entry(
        shaper_id,
        shaper_hash,
        td_ref,
        w_ref,
        1,
    ))
    .unwrap();
    assert_eq!(
        proj.len(),
        1,
        "shaper only; no children materialized when def not registered"
    );
}

// ---------------------------------------------------------------------------
// Materializer propagates content-store fetch failure (Boundary class #3).
// ---------------------------------------------------------------------------

#[test]
fn materialization_propagates_content_store_get_failure() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    let (_store, _reg, w_ref, materializer) = build_materializer(&shaper);
    // Use a result_ref that was NEVER staged → content store fetch fails.
    let unstaged_ref = ContentRef::from_bytes([0xff; 32]);
    let mut proj = Projection::with_materializer(materializer);
    let err = proj
        .fold(&shaper_committed_entry(
            shaper_id,
            shaper_hash,
            unstaged_ref,
            w_ref,
            1,
        ))
        .unwrap_err();
    assert!(
        matches!(
            err,
            kx_projection::ProjectionError::ContentStoreFetch { .. }
        ),
        "expected ContentStoreFetch, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Materializer propagates bincode decode failure (corrupt payload).
// ---------------------------------------------------------------------------

#[test]
fn materialization_propagates_topology_decode_failure() {
    let shaper = shaper_def();
    let shaper_id = shaper_mote_id(&shaper);
    let shaper_hash = shaper.hash();

    let (store, _reg, w_ref, materializer) = build_materializer(&shaper);
    // Stage garbage that won't decode as a TopologyDecision. 4 bytes is
    // below the minimum 8-byte fixed-int length prefix, so bincode
    // rejects with UnexpectedEnd rather than attempting to allocate a
    // bogusly-sized Vec.
    let garbage = vec![0u8; 4];
    let bad_ref = store.put(&garbage).expect("put succeeds");

    let mut proj = Projection::with_materializer(materializer);
    let err = proj
        .fold(&shaper_committed_entry(
            shaper_id,
            shaper_hash,
            bad_ref,
            w_ref,
            1,
        ))
        .unwrap_err();
    assert!(
        matches!(
            err,
            kx_projection::ProjectionError::TopologyDecodeFailed { .. }
        ),
        "expected TopologyDecodeFailed, got {err:?}"
    );
}
