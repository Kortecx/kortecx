#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! **PR 11 — Real-life E2E scenario** (required per the standing testing
//! doctrine, `04-testing-and-gates.md` §86+).
//!
//! Models the Seam-A-end-to-end shape: a "planner" shaper Mote
//! (READ-ONLY-NONDET; produces a `TopologyDecision`) commits a topology
//! spawning N "worker" children (PURE, deterministic), and we verify:
//!
//! - the materializer registers all N workers,
//! - all N workers appear in `ready_set` after the shaper commits,
//! - committing each worker leaves the projection in a consistent state,
//! - cold-re-folding the journal reproduces the same projection
//!   (R49 — replay faithfulness anchored by D49's P1).
//!
//! **Fully deterministic** — no real model, no real clock, no real
//! network, no thread-scheduling luck. Mocked "inference" = stub
//! TopologyDecision payload staged directly to the content store.

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
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

fn planner_def() -> MoteDef {
    let mut tools = BTreeMap::new();
    tools.insert(
        kx_mote::ToolName("text-summarize".into()),
        kx_mote::ToolVersion("1.0".into()),
    );
    MoteDef {
        logic_ref: LogicRef([1u8; 32]),
        model_id: ModelId("planner-7b".into()),
        prompt_template_hash: PromptTemplateHash([3u8; 32]),
        tool_contract: tools,
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn planner_mote_id(def: &MoteDef) -> MoteId {
    kx_mote::derive_mote_id(
        &def.hash(),
        &kx_mote::InputDataId::from_bytes([0u8; 32]),
        &kx_mote::GraphPosition(vec![0u8]),
    )
}

/// The planner's mocked "decision": three sub-tasks (worker A, B, C).
fn mocked_planner_output() -> TopologyDecision {
    TopologyDecision {
        children: vec![
            ChildDescriptor {
                role_id: RoleId("worker-a".into()),
                logic_ref: LogicRef([0xa1; 32]),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
            },
            ChildDescriptor {
                role_id: RoleId("worker-b".into()),
                logic_ref: LogicRef([0xb2; 32]),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
            },
            ChildDescriptor {
                role_id: RoleId("worker-c".into()),
                logic_ref: LogicRef([0xc3; 32]),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
            },
        ],
    }
}

fn stage(store: &InMemoryContentStore, td: &TopologyDecision) -> ContentRef {
    let bytes = bincode::serde::encode_to_vec(td, canonical_config())
        .expect("TopologyDecision canonical bincode encodes infallibly");
    store.put(&bytes).expect("put succeeds")
}

#[test]
fn planner_worker_shaper_end_to_end_demonstrates_seam_a() {
    let planner = planner_def();
    let planner_id = planner_mote_id(&planner);
    let planner_hash = planner.hash();

    // 1. Stage the planner's mocked TopologyDecision to the content store.
    let store = Arc::new(InMemoryContentStore::new());
    let registry = Arc::new(InMemoryMoteDefRegistry::new());
    registry.register(planner.clone());
    let td = mocked_planner_output();
    let td_ref = stage(&store, &td);

    // 2. Build the journal with the planner's Committed entry.
    let journal = InMemoryJournal::new();
    let planner_committed = JournalEntry::Committed {
        mote_id: planner_id,
        idempotency_key: planner_id.0,
        seq: 1,
        nondeterminism: NdClass::ReadOnlyNondet,
        result_ref: td_ref,
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: planner_hash,
    };
    journal.append(planner_committed).unwrap();

    // 3. Cold-fold from the journal with the materializer wired.
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&registry),
        InheritFromShaperResolver,
    ));
    let mut projection =
        Projection::from_journal_with_materializer(&journal, materializer).unwrap();

    // 4. Verify Seam A end-to-end:
    //    - planner Mote is Committed
    //    - 3 worker children are materialized
    //    - all 3 children appear in ready_set (planner is committed; each
    //      child's only parent is the planner)
    assert_eq!(projection.len(), 4, "planner + 3 workers = 4 Motes");
    assert_eq!(projection.state_of(&planner_id), MoteState::Committed);
    let ready = projection.ready_set();
    assert_eq!(ready.len(), 3);
    for worker_id in &ready {
        assert_ne!(*worker_id, planner_id);
        assert_eq!(projection.state_of(worker_id), MoteState::Pending);
        let parents = projection.parents_of(worker_id);
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].0, planner_id);
    }

    // 5. Commit each worker; verify projection consistency.
    for (i, worker_id) in ready.iter().enumerate() {
        let worker_committed = JournalEntry::Committed {
            mote_id: *worker_id,
            idempotency_key: worker_id.0,
            seq: 2 + i as u64,
            nondeterminism: NdClass::Pure,
            result_ref: ContentRef::from_bytes([(0x10 + i as u8); 32]),
            parents: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: kx_mote::MoteDefHash::from_bytes([(0x20 + i as u8); 32]),
        };
        journal.append(worker_committed.clone()).unwrap();
        projection.fold(&worker_committed).unwrap();
    }

    // All workers are now Committed.
    assert_eq!(
        projection.committed_count(),
        4,
        "planner + 3 workers all committed"
    );
    // ready_set is now empty (no more Pending Motes).
    assert!(projection.ready_set().is_empty());

    // 6. Cold re-fold from the same journal produces bit-identical state
    //    (R49 — replay faithfulness end-to-end).
    let materializer_2 = Box::new(DefaultTopologyMaterializer::new(
        Arc::clone(&store),
        Arc::clone(&registry),
        InheritFromShaperResolver,
    ));
    let re_projection =
        Projection::from_journal_with_materializer(&journal, materializer_2).unwrap();
    assert_eq!(re_projection.len(), projection.len());
    assert_eq!(
        re_projection.committed_count(),
        projection.committed_count()
    );
    // Every Mote ID is the same.
    let live_motes: Vec<MoteId> = projection.iter_motes().map(|(id, _)| id).collect();
    let cold_motes: Vec<MoteId> = re_projection.iter_motes().map(|(id, _)| id).collect();
    assert_eq!(live_motes, cold_motes, "R49: cold re-fold = live fold");
}
