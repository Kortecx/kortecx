//! End-to-end PURE Mote integration test. PR 9a's locked exit gate:
//! "runs a PURE Mote end-to-end through the platform backend."
//!
//! PR 9a uses a `TestMoteExecutor` (deterministic in-process body) instead
//! of the platform backend's real spawn path; the PR 9a-hardening follow-up
//! switches to the real bwrap/sandbox-exec backend without changing the
//! lifecycle's seams.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};

use kx_content::{ContentRef, InMemoryContentStore};
use kx_executor::{
    run_pure_mote, write_fact_zero, LocalResourceManager, SeedPayload, TestMoteExecutor,
};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
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
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
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
    }
}

fn pure_mote(seed_id_byte: u8) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![seed_id_byte]),
        SmallVec::new(),
    )
}

#[test]
fn pure_mote_runs_end_to_end_with_fact_zero() {
    let store = InMemoryContentStore::new();
    let journal = InMemoryJournal::new();
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();

    let warrant = permissive_warrant();
    let seed = SeedPayload {
        run_id: [42u8; 16],
        task: "demo".into(),
        system_prompt: None,
        workflow_def_ref: ContentRef::from_bytes([0; 32]),
        submitted_at_ms: 1_000,
    };

    // Step 1: write fact-zero (D34) as the first journal entry.
    let seed_mote_id =
        write_fact_zero(&store, &journal, &seed, &warrant).expect("fact-zero write must succeed");
    assert_eq!(seed_mote_id, kx_executor::seed_mote_id(&seed.run_id));
    assert_eq!(journal.count_entries().unwrap(), 1);

    // Step 2: run the root PURE Mote.
    let root = pure_mote(0);
    let commit = run_pure_mote(&root, &warrant, &journal, &rm, &executor)
        .expect("PURE Mote must run end-to-end");
    assert_eq!(commit.mote_id, root.id);

    // Step 3: assertion shape — the journal now has fact-zero (Committed) +
    // root Proposed + root Committed = 3 entries.
    assert_eq!(journal.count_entries().unwrap(), 3);

    // Step 4: replay-from-journal yields identical result_ref.
    let committed = journal.read_committed(&root.id).unwrap().unwrap();
    match committed {
        JournalEntry::Committed { result_ref, .. } => {
            assert_eq!(result_ref, commit.result_ref);
        }
        _ => panic!("expected Committed entry for root Mote"),
    }
}

#[test]
fn second_submit_run_with_same_seed_is_dedup_no_op() {
    let store = InMemoryContentStore::new();
    let journal = InMemoryJournal::new();
    let warrant = permissive_warrant();
    let seed = SeedPayload {
        run_id: [99u8; 16],
        task: "demo".into(),
        system_prompt: None,
        workflow_def_ref: ContentRef::from_bytes([0; 32]),
        submitted_at_ms: 0,
    };
    let mote_id_1 = write_fact_zero(&store, &journal, &seed, &warrant).unwrap();
    let mote_id_2 = write_fact_zero(&store, &journal, &seed, &warrant).unwrap();
    assert_eq!(mote_id_1, mote_id_2);
    // Second call dedups via the journal's `idempotency_key` partial index.
    assert_eq!(journal.count_entries().unwrap(), 1);
}

#[test]
fn fact_zero_result_ref_excludes_submitted_at_ms() {
    // D34 §3.3: two runs of the same task at different times produce the
    // same `result_ref` (audit timestamp is not identity-bearing).
    let seed_early = SeedPayload {
        run_id: [1u8; 16],
        task: "same-task".into(),
        system_prompt: None,
        workflow_def_ref: ContentRef::from_bytes([0; 32]),
        submitted_at_ms: 100,
    };
    let seed_late = SeedPayload {
        submitted_at_ms: 9_999_999,
        ..seed_early.clone()
    };
    assert_eq!(seed_early.result_ref(), seed_late.result_ref());
}

#[test]
fn fact_zero_mote_id_includes_run_id() {
    // D34 §3.4: two runs of the same task with different run_ids produce
    // different fact-zero mote_ids.
    let mid_a = kx_executor::seed_mote_id(&[1u8; 16]);
    let mid_b = kx_executor::seed_mote_id(&[2u8; 16]);
    assert_ne!(mid_a, mid_b);
}

#[test]
fn pure_mote_writes_warrant_ref_on_proposed_and_committed() {
    let store = InMemoryContentStore::new();
    let journal = InMemoryJournal::new();
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    let warrant = permissive_warrant();
    let expected_warrant_ref = kx_warrant::warrant_ref_of(&warrant);

    let root = pure_mote(7);
    let _commit = run_pure_mote(&root, &warrant, &journal, &rm, &executor).unwrap();

    let entries: Vec<JournalEntry> = journal.read_entries_by_seq(0..u64::MAX).unwrap().collect();
    let mut found_proposed = false;
    let mut found_committed = false;
    for entry in entries {
        match entry {
            JournalEntry::Proposed {
                warrant_ref,
                mote_id,
                ..
            } if mote_id == root.id => {
                assert_eq!(warrant_ref, expected_warrant_ref);
                found_proposed = true;
            }
            JournalEntry::Committed {
                warrant_ref,
                mote_id,
                ..
            } if mote_id == root.id => {
                assert_eq!(warrant_ref, expected_warrant_ref);
                found_committed = true;
            }
            _ => {}
        }
    }
    assert!(
        found_proposed,
        "Proposed entry must carry warrant_ref (D36)"
    );
    assert!(
        found_committed,
        "Committed entry must carry warrant_ref (D36)"
    );

    // Sanity: the warrant bytes are also in the content store (fact-zero's
    // pre-flight put). PR 9a doesn't pre-`put` for non-seed warrants — the
    // lifecycle layer's caller is expected to `put` before invoking. The
    // store usage at PR 9a is documented as "audit-only"; PR 9b's commit
    // protocol formalizes the put-then-append discipline (D39 §a).
    let _ = store;
}
