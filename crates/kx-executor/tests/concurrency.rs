//! Compile-time `Send + Sync` over the kx-executor public surface +
//! 4-thread thread-independence under `Arc<>`. SN-4 v2 mandate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::sync::Arc;
use std::thread;

use kx_executor::{
    default_executor, executor_for_class, profile_from_warrant, run_pure_mote,
    seed_idempotency_key, seed_mote_id, validate_submission, write_fact_zero, BwrapExecutor,
    CloudMicroVmExecutor, FactZeroError, LifecycleCommit, LifecycleError, LocalResourceManager,
    MacOsSandboxExecutor, MoteExecutionResult, MoteExecutor, MoteExecutorError, OciDaemonExecutor,
    ResourceError, ResourceManager, Rootfs, SbplProfile, SeedPayload, Slot, SubmissionRefusal,
    TestMoteExecutor, WorkflowSubmission,
};

// ---------------------------------------------------------------------------
// Compile-time Send + Sync over the public-type set. Any regression here is a
// type-system regression — Send/Sync must be inferable for every public type
// to admit safe sharing across worker threads.
// ---------------------------------------------------------------------------

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_sync() {
    // Trait objects — the load-bearing seam (callers hold Arc<dyn ...> per
    // 02-crate-specs.md's MoteExecutor / ResourceManager surfaces).
    assert_send_sync::<Arc<dyn MoteExecutor>>();
    assert_send_sync::<Arc<dyn ResourceManager>>();

    // Concrete backends.
    assert_send_sync::<BwrapExecutor>();
    assert_send_sync::<MacOsSandboxExecutor>();
    assert_send_sync::<OciDaemonExecutor>();
    assert_send_sync::<CloudMicroVmExecutor>();
    assert_send_sync::<LocalResourceManager>();
    assert_send_sync::<TestMoteExecutor>();

    // Value / payload types.
    assert_send_sync::<MoteExecutionResult>();
    assert_send_sync::<MoteExecutorError>();
    assert_send_sync::<Rootfs>();
    assert_send_sync::<Slot>();
    assert_send_sync::<ResourceError>();
    assert_send_sync::<SbplProfile>();
    assert_send_sync::<SeedPayload>();
    // FactZeroError holds a `Box<dyn Error + Send + Sync>` via thiserror — confirm.
    assert_send::<FactZeroError>();
    assert_sync::<FactZeroError>();

    assert_send_sync::<SubmissionRefusal>();
    assert_send_sync::<WorkflowSubmission>();
    assert_send_sync::<LifecycleCommit>();
    assert_send::<LifecycleError>();
    assert_sync::<LifecycleError>();
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence: PURE-Mote lifecycle under Arc<dyn MoteExecutor>.
// Spawns 4 threads each running a PURE Mote through `run_pure_mote`; asserts
// no thread observes another thread's commits + the final journal carries
// 4 Committed entries (or fewer, if the dedup-by-key triggers due to identical
// MoteIds — but we make them distinct via graph_position).
// ---------------------------------------------------------------------------

#[test]
fn four_thread_pure_mote_thread_independence() {
    use kx_content::InMemoryContentStore;
    use kx_journal::{InMemoryJournal, Journal};
    use kx_mote::{
        ConfigKey, ConfigVal, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
        MoteDef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use smallvec::SmallVec;
    use std::collections::BTreeMap;

    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let rm: Arc<dyn ResourceManager> = Arc::new(LocalResourceManager::dev_defaults());
    let executor: Arc<dyn MoteExecutor> = Arc::new(TestMoteExecutor::deterministic());

    let warrant = example_warrant();
    let mut handles = Vec::new();
    for thread_index in 0..4u8 {
        let journal = Arc::clone(&journal);
        let rm = Arc::clone(&rm);
        let executor = Arc::clone(&executor);
        let warrant = warrant.clone();
        let _store = Arc::clone(&store);
        let handle = thread::spawn(move || {
            let def = MoteDef {
                critic_check: None,
                logic_ref: LogicRef::from_bytes([1; 32]),
                model_id: ModelId("local".into()),
                prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
                tool_contract: BTreeMap::<_, _>::new(),
                nd_class: NdClass::Pure,
                config_subset: BTreeMap::<ConfigKey, ConfigVal>::new(),
                effect_pattern: EffectPattern::IdempotentByConstruction,
                critic_for: None,
                is_topology_shaper: false,
                inference_params: kx_mote::InferenceParams::default(),
                schema_version: MOTE_DEF_SCHEMA_VERSION,
            };
            let mote = Mote::new(
                def,
                InputDataId::from_bytes([0; 32]),
                GraphPosition(vec![thread_index]),
                SmallVec::new(),
            );
            run_pure_mote(&mote, &warrant, &*journal, &*rm, &*executor)
        });
        handles.push(handle);
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    for r in &results {
        r.as_ref()
            .expect("each thread's PURE-Mote run should succeed");
    }
    // 4 threads × (1 Proposed + 1 Committed) = 8 journal entries.
    assert_eq!(journal.count_entries().unwrap(), 8);
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence: validate_submission + write_fact_zero + the
// pure helpers. Proves the helpers are pure / total / deterministic across
// threads.
// ---------------------------------------------------------------------------

#[test]
fn four_thread_pure_helpers_are_thread_independent() {
    let mut handles = Vec::new();
    for thread_index in 0..4u8 {
        let handle = thread::spawn(move || {
            // Each thread derives the same seed_mote_id for the same run_id.
            let run_id = [thread_index; 16];
            let mid = seed_mote_id(&run_id);
            let idk = seed_idempotency_key(&run_id);
            (mid, idk)
        });
        handles.push(handle);
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    for (i, (mid, idk)) in results.iter().enumerate() {
        // Run-id-determined → idempotent across threads.
        let expected_mid = seed_mote_id(&[i as u8; 16]);
        let expected_idk = seed_idempotency_key(&[i as u8; 16]);
        assert_eq!(*mid, expected_mid);
        assert_eq!(*idk, expected_idk);
    }
}

#[test]
fn four_thread_validate_submission_is_thread_independent() {
    use std::collections::BTreeMap;
    let warrant = example_warrant();
    let submission = Arc::new(WorkflowSubmission {
        run_id: [0u8; 32],
        master_warrant: warrant,
        motes: BTreeMap::new(),
        accept_at_least_once: BTreeMap::new(),
    });
    let mut handles = Vec::new();
    for _ in 0..4 {
        let sub = Arc::clone(&submission);
        handles.push(thread::spawn(move || validate_submission(&sub)));
    }
    for h in handles {
        let r = h.join().unwrap();
        assert!(r.is_ok(), "empty submission has no R-* triggers");
    }
}

#[test]
fn four_thread_profile_from_warrant_is_thread_independent() {
    let warrant = Arc::new(example_warrant());
    let mut handles = Vec::new();
    for _ in 0..4 {
        let w = Arc::clone(&warrant);
        handles.push(thread::spawn(move || profile_from_warrant(&w)));
    }
    let profiles: Vec<SbplProfile> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // All four threads produce byte-identical profiles (purity).
    let first = profiles[0].as_bytes().to_vec();
    for p in &profiles[1..] {
        assert_eq!(p.as_bytes(), first.as_slice());
    }
}

#[test]
fn fact_zero_write_is_idempotent_across_threads() {
    use kx_content::InMemoryContentStore;
    use kx_journal::{InMemoryJournal, Journal};

    let store = Arc::new(InMemoryContentStore::new());
    let journal = Arc::new(InMemoryJournal::new());
    let warrant = Arc::new(example_warrant());
    let seed = Arc::new(SeedPayload {
        run_id: [7u8; 16],
        task: "shared".into(),
        system_prompt: None,
        workflow_def_ref: kx_content::ContentRef::from_bytes([0; 32]),
        submitted_at_ms: 0,
    });

    let mut handles = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let journal = Arc::clone(&journal);
        let warrant = Arc::clone(&warrant);
        let seed = Arc::clone(&seed);
        handles.push(thread::spawn(move || {
            write_fact_zero(&*store, &*journal, &seed, &warrant)
        }));
    }
    let mote_ids: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .collect();
    // All threads must produce the same mote_id (D34 §3.4).
    for m in &mote_ids[1..] {
        assert_eq!(*m, mote_ids[0]);
    }
    // Journal must have exactly ONE Committed entry (dedup-by-key per the
    // shared idempotency_key).
    assert_eq!(journal.count_entries().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------------

fn example_warrant() -> kx_warrant::WarrantSpec {
    use std::collections::BTreeSet;
    kx_warrant::WarrantSpec {
        mote_class: kx_warrant::MoteClass::Pure,
        nd_class: kx_warrant::MoteClass::Pure,
        fs_scope: kx_warrant::FsScope::empty(),
        net_scope: kx_warrant::NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: kx_warrant::ModelRoute {
            model_id: kx_mote::ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: kx_warrant::ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: kx_warrant::ExecutorClass::Bwrap,
    }
}

// ---------------------------------------------------------------------------
// Smoke checks on the default factory.
// ---------------------------------------------------------------------------

#[test]
fn default_executor_is_constructible_and_is_send_sync() {
    let exec: Box<dyn MoteExecutor> = default_executor();
    let _ = exec;
}

#[test]
fn executor_for_class_returns_a_backend_for_each_variant() {
    use kx_warrant::ExecutorClass;
    let _ = executor_for_class(ExecutorClass::Bwrap);
    let _ = executor_for_class(ExecutorClass::MacOsSandbox);
    let _ = executor_for_class(ExecutorClass::OciDaemon);
    let _ = executor_for_class(ExecutorClass::CloudMicroVm);
}
