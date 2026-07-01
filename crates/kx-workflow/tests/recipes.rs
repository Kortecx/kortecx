//! The Morphic recipe library compiles to deterministic, well-formed Mote DAGs:
//! each recipe is reproducible (compile twice → byte-identical `MoteId`s), its
//! structure is what the recipe promises (fan-out width, edges, roles), and
//! identity shifts with the workflow seed. Recipes are static single-level
//! compositions of the existing builders — no topology shaper, no core change.
//!
//! Integration tests compile as their own crate; this file carries its own lint
//! exemptions (the workspace deny on unwrap/expect applies to library code).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::pedantic
)]

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_critic_types::{CheckSpec, SchemaSpec, SchemaTag};
use kx_mote::{ConfigKey, LogicRef, ModelId, NdClass, ToolName, ToolVersion};
use kx_workflow::{
    compile, fan_out_gather, image_batch_describe_reduce, map_reduce, rag_pipeline,
    rag_pipeline_hybrid, react_tool_loop, retry_until_critic, CompileError, WorkerKind,
    IMAGE_REF_KEY,
};

fn model() -> ModelId {
    ModelId("local".into())
}
fn cap() -> ToolName {
    ToolName("demo".into())
}
fn logic(seed: u8) -> LogicRef {
    LogicRef::from_bytes([seed; 32])
}
fn json_check() -> CheckSpec {
    CheckSpec::Schema(SchemaSpec {
        expected: SchemaTag::Json,
    })
}

/// The compiled `MoteId` vector — the determinism fingerprint.
fn ids(wf: &kx_workflow::WorkflowDef) -> Vec<kx_mote::MoteId> {
    compile(wf)
        .unwrap()
        .motes
        .iter()
        .map(|m| m.mote.id)
        .collect()
}

// ── map_reduce ──────────────────────────────────────────────────────────────

#[test]
fn map_reduce_compiles_deterministically_with_n_plus_one_motes() {
    let mappers = [logic(1), logic(2), logic(3)];
    let wf = map_reduce(7, model(), cap(), WorkerKind::Transform, &mappers, logic(9)).unwrap();
    assert_eq!(ids(&wf), ids(&wf), "recipe compile must be deterministic");
    let out = compile(&wf).unwrap();
    assert_eq!(out.motes.len(), mappers.len() + 1, "N mappers + 1 reduce");
}

#[test]
fn map_reduce_reduce_consumes_every_mapper() {
    let mappers = [logic(1), logic(2), logic(3), logic(4)];
    let out =
        compile(&map_reduce(0, model(), cap(), WorkerKind::Transform, &mappers, logic(9)).unwrap())
            .unwrap();
    // The reduce step is the one carrying reduce_logic; it must parent every mapper.
    let reduce = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(9))
        .expect("reduce present");
    assert_eq!(
        reduce.mote.parents.len(),
        mappers.len(),
        "reduce has one Data parent per mapper"
    );
    // Parents precede the reduce in topological order.
    let pos: BTreeMap<_, _> = out
        .motes
        .iter()
        .enumerate()
        .map(|(i, m)| (m.mote.id, i))
        .collect();
    let reduce_pos = pos[&reduce.mote.id];
    for p in &reduce.mote.parents {
        assert!(pos[&p.parent_id] < reduce_pos, "mapper precedes reduce");
    }
}

#[test]
fn map_reduce_worker_kind_sets_nd_class() {
    let mappers = [logic(1), logic(2)];
    let pure =
        compile(&map_reduce(0, model(), cap(), WorkerKind::Transform, &mappers, logic(9)).unwrap())
            .unwrap();
    for m in pure
        .motes
        .iter()
        .filter(|m| m.mote.def.logic_ref != logic(9))
    {
        assert_eq!(
            m.mote.def.nd_class,
            NdClass::Pure,
            "Transform mappers are PURE"
        );
    }
    let nd =
        compile(&map_reduce(0, model(), cap(), WorkerKind::Generator, &mappers, logic(9)).unwrap())
            .unwrap();
    for m in nd.motes.iter().filter(|m| m.mote.def.logic_ref != logic(9)) {
        assert_eq!(
            m.mote.def.nd_class,
            NdClass::ReadOnlyNondet,
            "Generator mappers are READ-ONLY-NONDET"
        );
    }
}

#[test]
fn map_reduce_seed_changes_identity() {
    let mappers = [logic(1), logic(2), logic(3)];
    let a = ids(&map_reduce(1, model(), cap(), WorkerKind::Transform, &mappers, logic(9)).unwrap());
    let b = ids(&map_reduce(2, model(), cap(), WorkerKind::Transform, &mappers, logic(9)).unwrap());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_ne!(x, y, "every MoteId shifts with the seed");
    }
}

#[test]
fn map_reduce_empty_is_empty_recipe() {
    let err = map_reduce(0, model(), cap(), WorkerKind::Transform, &[], logic(9)).unwrap_err();
    assert_eq!(err, CompileError::EmptyRecipe);
}

// ── fan_out_gather ──────────────────────────────────────────────────────────

#[test]
fn fan_out_gather_workers_are_nondet_gather_is_pure() {
    let workers = [logic(1), logic(2), logic(3)];
    let wf = fan_out_gather(5, model(), cap(), &workers, logic(9)).unwrap();
    assert_eq!(ids(&wf), ids(&wf), "deterministic compile");
    let out = compile(&wf).unwrap();
    assert_eq!(out.motes.len(), workers.len() + 1);
    let gather = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(9))
        .unwrap();
    assert_eq!(gather.mote.def.nd_class, NdClass::Pure, "gather is PURE");
    assert_eq!(gather.mote.parents.len(), workers.len());
    for m in out
        .motes
        .iter()
        .filter(|m| m.mote.def.logic_ref != logic(9))
    {
        assert_eq!(
            m.mote.def.nd_class,
            NdClass::ReadOnlyNondet,
            "workers sample"
        );
    }
}

#[test]
fn fan_out_gather_empty_is_empty_recipe() {
    assert_eq!(
        fan_out_gather(0, model(), cap(), &[], logic(9)).unwrap_err(),
        CompileError::EmptyRecipe
    );
}

// ── retry_until_critic (bounded best-of-N) ──────────────────────────────────

#[test]
fn retry_until_critic_gates_each_attempt_with_a_native_critic() {
    let attempts = [logic(1), logic(2), logic(3)];
    let wf = retry_until_critic(
        7,
        model(),
        cap(),
        &attempts,
        &json_check(),
        logic(8),
        logic(9),
    )
    .unwrap();
    assert_eq!(ids(&wf), ids(&wf), "deterministic compile");
    let out = compile(&wf).unwrap();
    // N attempts + N critics + 1 selector.
    assert_eq!(out.motes.len(), 2 * attempts.len() + 1);

    let critics: Vec<_> = out
        .motes
        .iter()
        .filter(|m| m.mote.def.critic_check.is_some())
        .collect();
    assert_eq!(critics.len(), attempts.len(), "one critic per attempt");
    let attempt_ids: std::collections::BTreeSet<_> = out.motes.iter().map(|m| m.mote.id).collect();
    for c in &critics {
        assert_eq!(c.mote.def.nd_class, NdClass::Pure, "native critic is PURE");
        assert!(!c.mote.def.is_topology_shaper, "critic is not a shaper");
        let producer = c.mote.def.critic_for.expect("critic_for resolves");
        assert!(
            attempt_ids.contains(&producer),
            "critic gates a real attempt"
        );
    }
    // The selector consumes every attempt + every verdict: 2N parents.
    let select = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(9))
        .unwrap();
    assert_eq!(select.mote.parents.len(), 2 * attempts.len());
}

#[test]
fn retry_until_critic_empty_is_empty_recipe() {
    assert_eq!(
        retry_until_critic(0, model(), cap(), &[], &json_check(), logic(8), logic(9)).unwrap_err(),
        CompileError::EmptyRecipe
    );
}

// ── react_tool_loop (single turn) ───────────────────────────────────────────

#[test]
fn react_tool_loop_wires_reason_act_observe_with_tool_contract() {
    let mut tools = BTreeMap::new();
    tools.insert(ToolName("search".into()), ToolVersion("1".into()));
    let wf = react_tool_loop(
        3,
        model(),
        cap(),
        logic(1),
        logic(2),
        logic(3),
        tools.clone(),
    )
    .unwrap();
    assert_eq!(ids(&wf), ids(&wf), "deterministic compile");
    let out = compile(&wf).unwrap();
    assert_eq!(out.motes.len(), 3, "reason + act + observe");

    let reason = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(1))
        .unwrap();
    let act = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(2))
        .unwrap();
    let observe = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(3))
        .unwrap();

    assert_eq!(reason.mote.def.nd_class, NdClass::ReadOnlyNondet);
    assert_eq!(act.mote.def.nd_class, NdClass::ReadOnlyNondet);
    assert_eq!(
        observe.mote.def.nd_class,
        NdClass::Pure,
        "observe folds deterministically"
    );
    assert_eq!(
        act.mote.def.tool_contract, tools,
        "act carries the closed tool contract"
    );
    assert!(reason.mote.parents.is_empty(), "reason is the turn root");
    assert_eq!(act.mote.parents.len(), 1, "act depends on reason");
    assert_eq!(observe.mote.parents.len(), 1, "observe depends on act");
}

// ── rag_pipeline (DP2 — retrieval-augmented generation) ─────────────────────

#[test]
fn rag_pipeline_wires_n_ingest_then_query_then_assemble() {
    let docs = [logic(1), logic(2), logic(3)];
    let query = logic(50);
    let assemble = logic(60);
    let wf = rag_pipeline(7, model(), cap(), &docs, query, assemble).unwrap();
    assert_eq!(ids(&wf), ids(&wf), "recipe compile must be deterministic");
    let out = compile(&wf).unwrap();
    assert_eq!(
        out.motes.len(),
        docs.len() + 2,
        "N ingest + 1 query + 1 assemble"
    );

    // The ingest steps are READ-ONLY-NONDET (they embed + populate the index).
    for d in &docs {
        let ingest = out
            .motes
            .iter()
            .find(|m| m.mote.def.logic_ref == *d)
            .expect("ingest present");
        assert_eq!(
            ingest.mote.def.nd_class,
            NdClass::ReadOnlyNondet,
            "ingest/embed steps sample"
        );
        assert!(ingest.mote.parents.is_empty(), "ingest is a corpus root");
    }

    // The query (retrieval) step is ROND and parents EVERY ingest.
    let q = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == query)
        .expect("query present");
    assert_eq!(
        q.mote.def.nd_class,
        NdClass::ReadOnlyNondet,
        "retrieval is a nondet read of the index"
    );
    assert_eq!(
        q.mote.parents.len(),
        docs.len(),
        "the query reads an index populated by every ingest"
    );

    // The assemble step is PURE and depends only on the query's top-k fact.
    let a = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == assemble)
        .expect("assemble present");
    assert_eq!(
        a.mote.def.nd_class,
        NdClass::Pure,
        "assemble folds the retrieved refs deterministically"
    );
    assert_eq!(
        a.mote.parents.len(),
        1,
        "assemble depends on the query fact"
    );
    assert_eq!(
        a.mote.parents[0].parent_id, q.mote.id,
        "assemble's parent is the query step"
    );
}

#[test]
fn rag_pipeline_single_doc_is_three_motes() {
    let out = compile(&rag_pipeline(0, model(), cap(), &[logic(1)], logic(50), logic(60)).unwrap())
        .unwrap();
    assert_eq!(out.motes.len(), 3, "1 ingest + query + assemble");
}

#[test]
fn rag_pipeline_seed_changes_identity() {
    let docs = [logic(1), logic(2)];
    let a = ids(&rag_pipeline(1, model(), cap(), &docs, logic(50), logic(60)).unwrap());
    let b = ids(&rag_pipeline(2, model(), cap(), &docs, logic(50), logic(60)).unwrap());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_ne!(x, y, "every MoteId shifts with the seed");
    }
}

#[test]
fn rag_pipeline_empty_corpus_is_empty_recipe() {
    assert_eq!(
        rag_pipeline(0, model(), cap(), &[], logic(50), logic(60)).unwrap_err(),
        CompileError::EmptyRecipe
    );
}

// ── rag_pipeline_hybrid (RC4c — hybrid + rewrite + optional LLM rerank) ──────

#[test]
fn rag_pipeline_hybrid_wires_rewrite_then_query_then_assemble() {
    let docs = [logic(1), logic(2)];
    // rewrite(10) → query(50) → assemble(60); no rerank.
    let wf = rag_pipeline_hybrid(
        7,
        model(),
        cap(),
        &docs,
        logic(10),
        logic(50),
        None,
        logic(60),
    )
    .unwrap();
    let out = compile(&wf).unwrap();
    // N ingest + rewrite + query + assemble.
    assert_eq!(out.motes.len(), docs.len() + 3);

    let rewrite = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(10))
        .expect("rewrite present");
    assert_eq!(rewrite.mote.def.nd_class, NdClass::ReadOnlyNondet);

    let q = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(50))
        .expect("query present");
    // the query reads EVERY ingest + the rewrite.
    assert_eq!(q.mote.parents.len(), docs.len() + 1);
    // and bakes the hybrid retrieval-mode marker.
    assert_eq!(
        q.mote
            .def
            .config_subset
            .get(&ConfigKey("kx.retrieval.mode".into())),
        Some(&kx_mote::ConfigVal(b"hybrid".to_vec())),
        "the hybrid query step carries the retrieval-mode marker"
    );
}

#[test]
fn rag_pipeline_hybrid_query_moteid_differs_from_dense_rag_pipeline() {
    let docs = [logic(1), logic(2)];
    // The dense `rag_pipeline` query step (no rewrite, no marker).
    let dense =
        compile(&rag_pipeline(7, model(), cap(), &docs, logic(50), logic(60)).unwrap()).unwrap();
    let dense_q = dense
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(50))
        .unwrap();
    // The hybrid query step (same logic ref + seed) — the retrieval-mode marker MUST
    // shift its MoteId (a different retrieval is a different fact); the dense recipe's
    // golden identities are untouched.
    let hybrid = compile(
        &rag_pipeline_hybrid(
            7,
            model(),
            cap(),
            &docs,
            logic(10),
            logic(50),
            None,
            logic(60),
        )
        .unwrap(),
    )
    .unwrap();
    let hybrid_q = hybrid
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(50))
        .unwrap();
    assert_ne!(
        dense_q.mote.id, hybrid_q.mote.id,
        "the hybrid retrieval-mode marker yields a distinct query MoteId"
    );
}

#[test]
fn rag_pipeline_hybrid_with_rerank_adds_a_step_before_assemble() {
    let docs = [logic(1)];
    let wf = rag_pipeline_hybrid(
        0,
        model(),
        cap(),
        &docs,
        logic(10),
        logic(50),
        Some(logic(55)),
        logic(60),
    )
    .unwrap();
    let out = compile(&wf).unwrap();
    // 1 ingest + rewrite + query + rerank + assemble.
    assert_eq!(out.motes.len(), 5);
    let rerank = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(55))
        .expect("rerank present");
    assert_eq!(rerank.mote.def.nd_class, NdClass::ReadOnlyNondet);
    // assemble's sole parent is the rerank step (it grounds on the reranked order).
    let a = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(60))
        .unwrap();
    assert_eq!(a.mote.parents.len(), 1);
    assert_eq!(a.mote.parents[0].parent_id, rerank.mote.id);
}

#[test]
fn rag_pipeline_hybrid_empty_corpus_is_empty_recipe() {
    assert_eq!(
        rag_pipeline_hybrid(
            0,
            model(),
            cap(),
            &[],
            logic(10),
            logic(50),
            None,
            logic(60)
        )
        .unwrap_err(),
        CompileError::EmptyRecipe
    );
}

// ── image_batch_describe_reduce (multi-modal capstone) ──────────────────────

#[test]
fn image_batch_bakes_a_distinct_image_into_each_describe_step() {
    let images = [
        ContentRef([1; 32]),
        ContentRef([2; 32]),
        ContentRef([3; 32]),
    ];
    let wf = image_batch_describe_reduce(11, model(), cap(), logic(7), &images, logic(9)).unwrap();
    assert_eq!(ids(&wf), ids(&wf), "deterministic compile");
    let out = compile(&wf).unwrap();
    assert_eq!(out.motes.len(), images.len() + 1, "N describes + 1 reduce");

    let image_key = ConfigKey(IMAGE_REF_KEY.to_string());
    // Every describe step carries its OWN image ref (not an empty placeholder).
    let baked: std::collections::BTreeSet<Vec<u8>> = out
        .motes
        .iter()
        .filter(|m| m.mote.def.logic_ref == logic(7))
        .map(|m| {
            assert_eq!(
                m.mote.def.nd_class,
                NdClass::ReadOnlyNondet,
                "describe samples"
            );
            m.mote
                .def
                .config_subset
                .get(&image_key)
                .expect("describe carries its image ref")
                .0
                .clone()
        })
        .collect();
    assert_eq!(
        baked.len(),
        images.len(),
        "each describe step carries a DISTINCT image ref"
    );
    for img in &images {
        assert!(
            baked.contains(&img.0[..]),
            "the exact image ref is baked in"
        );
    }

    let reduce = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(9))
        .unwrap();
    assert_eq!(reduce.mote.parents.len(), images.len());
}

#[test]
fn image_batch_distinct_images_yield_distinct_describe_identities() {
    // The capstone promise: N distinct images ⇒ N distinct describe Motes.
    let a = compile(
        &image_batch_describe_reduce(
            0,
            model(),
            cap(),
            logic(7),
            &[ContentRef([1; 32])],
            logic(9),
        )
        .unwrap(),
    )
    .unwrap();
    let b = compile(
        &image_batch_describe_reduce(
            0,
            model(),
            cap(),
            logic(7),
            &[ContentRef([2; 32])],
            logic(9),
        )
        .unwrap(),
    )
    .unwrap();
    // Same describe_logic + same position, only the image differs → different identity.
    assert_ne!(
        a.motes[0].mote.id, b.motes[0].mote.id,
        "a different image is a different describe Mote (image ref folds into identity)"
    );
}

#[test]
fn image_batch_empty_is_empty_recipe() {
    assert_eq!(
        image_batch_describe_reduce(0, model(), cap(), logic(7), &[], logic(9)).unwrap_err(),
        CompileError::EmptyRecipe
    );
}

// ── property: arbitrary static-N map_reduce compiles deterministically ───────

use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_map_reduce_any_n_is_deterministic_and_well_formed(
        seed in any::<u32>(),
        n in 1usize..16,
    ) {
        let mappers: Vec<LogicRef> = (0..n).map(|i| logic(i as u8)).collect();
        let wf = map_reduce(seed, model(), cap(), WorkerKind::Transform, &mappers, logic(200)).unwrap();
        let a = compile(&wf).unwrap();
        let b = compile(&wf).unwrap();
        let ids_a: Vec<_> = a.motes.iter().map(|m| m.mote.id).collect();
        let ids_b: Vec<_> = b.motes.iter().map(|m| m.mote.id).collect();
        prop_assert_eq!(ids_a, ids_b);
        prop_assert_eq!(a.motes.len(), n + 1);

        // Well-formed: every parent exists and precedes its child.
        let pos: BTreeMap<_, _> = a.motes.iter().enumerate().map(|(i, m)| (m.mote.id, i)).collect();
        for (i, m) in a.motes.iter().enumerate() {
            for parent in &m.mote.parents {
                prop_assert!(a.graph.get(&parent.parent_id).is_some());
                prop_assert!(pos[&parent.parent_id] < i);
            }
        }
    }
}
