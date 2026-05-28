//! Thesis-test witness (compile-level): `kx-worker` hosts the real `kx-executor`
//! (and, transitively, `kx-inference`) types verbatim — it runs Motes through the
//! exact `run_pure_mote` entry point + `MoteExecutor` / `ResourceManager` seams the
//! single-node `kx-runtime` uses, adding only the propose-don't-write glue.
//!
//! The *authoritative* proof that `kx-scheduler` / `kx-executor` / `kx-inference`
//! source is unchanged is the PR diff (`git diff <merge-base> -- crates/kx-scheduler
//! crates/kx-executor crates/kx-inference` is empty) — this test just pins the
//! dependency direction so the hosting cannot silently drift.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_executor::{
    run_pure_mote, LocalResourceManager, MoteExecutor, ResourceManager, TestMoteExecutor,
};
use kx_journal::InMemoryJournal;
use kx_mote::{GraphPosition, InputDataId, Mote, MoteDef, NdClass};

#[test]
fn worker_runs_motes_through_the_real_executor_entry_point() {
    // The exact stack the worker hosts: a throwaway journal, a real
    // ResourceManager, and a MoteExecutor — driven by run_pure_mote verbatim.
    let executor = TestMoteExecutor::deterministic();
    let rm = LocalResourceManager::dev_defaults();
    let scratch = InMemoryJournal::new();

    let def = MoteDef {
        nd_class: NdClass::Pure,
        ..sample_pure_def()
    };
    let mote = Mote::new(
        def,
        InputDataId::from_bytes([1u8; 32]),
        GraphPosition(vec![0]),
        smallvec_empty(),
    );
    let warrant = sample_warrant();

    // Hosted verbatim: this is the single-node executor entry point, called from a
    // worker that will PROPOSE the result rather than treat the journal as durable.
    let commit = run_pure_mote(&mote, &warrant, &scratch, &rm, &executor).expect("pure run");
    assert_eq!(commit.mote_id, mote.id);
    // And `MoteExecutor` / `ResourceManager` are consumed as the real traits.
    assert!(MoteExecutor::supports(&executor, warrant.executor_class));
    let slot = ResourceManager::acquire(&rm, &warrant.resource_ceiling).unwrap();
    ResourceManager::release(&rm, slot).unwrap();
}

// --- minimal fixtures (kept local so the witness has no cross-test deps) -------

fn smallvec_empty() -> smallvec::SmallVec<[kx_mote::ParentRef; 4]> {
    smallvec::SmallVec::new()
}

fn sample_pure_def() -> MoteDef {
    use kx_mote::{
        EffectPattern, InferenceParams, LogicRef, ModelId, PromptTemplateHash,
        MOTE_DEF_SCHEMA_VERSION,
    };
    MoteDef {
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: std::collections::BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: std::collections::BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn sample_warrant() -> kx_warrant::WarrantSpec {
    use kx_content::ContentRef;
    use kx_warrant::{
        ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
    };
    use std::collections::{BTreeMap, BTreeSet};
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: kx_mote::ModelId("m".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 3,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::MacOsSandbox,
    }
}
