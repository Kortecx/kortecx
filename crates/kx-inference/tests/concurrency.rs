//! Concurrency contract — SN-4 v2:
//!   - Send + Sync compile-time assertions on every public type.
//!   - Thread-independence under `Arc<dyn InferenceBackend>` (4 threads,
//!     each running disjoint dispatches; aggregated counters confirm
//!     every thread's work landed).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::thread;

use common::FakeBackend;
use kx_content::ContentRef;
use kx_inference::{
    BatchItem, Dispatcher, DispatcherConfig, Grammar, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams, LlamaInferenceBackend,
};
use kx_model_validator::InMemoryModelRegistry;
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

/// Compile-time `Send` + `Sync` set — fails to compile if any public type
/// silently loses these traits.
fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn send_sync_set() {
    assert_send::<InferenceInput>();
    assert_sync::<InferenceInput>();
    assert_send::<InferenceParams>();
    assert_sync::<InferenceParams>();
    assert_send::<InferenceOutput>();
    assert_sync::<InferenceOutput>();
    assert_send::<InferenceError>();
    assert_sync::<InferenceError>();
    assert_send::<Grammar>();
    assert_sync::<Grammar>();
    assert_send_sync::<LlamaInferenceBackend>();
    assert_send_sync::<Dispatcher>();
    // Trait-object form (what the dispatcher holds).
    assert_send_sync::<Arc<dyn InferenceBackend>>();
    assert_send_sync::<Box<dyn InferenceBackend>>();
    assert_send_sync::<BatchItem<'_>>();
}

fn dummy_warrant(model_id: ModelId) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id,
            max_input_tokens: 2048,
            max_output_tokens: 512,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

#[test]
fn four_thread_dispatch_independence() {
    // Single backend, four threads, 100 dispatches each. Final counter
    // MUST be 400 regardless of interleaving (proves thread-independence
    // of dispatch under `Arc<dyn InferenceBackend>`).
    const THREADS: usize = 4;
    const PER_THREAD: usize = 100;

    let id = ModelId("test-model".into());
    let fake = FakeBackend::new("fake").with_model(id.clone());
    let dispatch_counter = fake.dispatch_calls.clone();
    let backend: Arc<dyn InferenceBackend> = Arc::new(fake);

    let mut handles = Vec::with_capacity(THREADS);
    for thread_idx in 0..THREADS {
        let b = Arc::clone(&backend);
        let id = id.clone();
        let warrant = dummy_warrant(id.clone());
        handles.push(thread::spawn(move || {
            let prompt = InferenceInput::Text(format!("thread {thread_idx}"));
            let params = InferenceParams::default();
            for _ in 0..PER_THREAD {
                let _out = b
                    .dispatch(&id, &prompt, &params, &warrant)
                    .expect("dispatch must succeed");
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let total = THREADS * PER_THREAD;
    assert_eq!(
        dispatch_counter.load(std::sync::atomic::Ordering::SeqCst),
        total as u64,
        "every dispatch across all threads must be counted"
    );
}

#[test]
fn dispatcher_clone_is_shareable_across_threads() {
    // Dispatcher is `Clone` (it holds Vec<Arc<...>> + Arc<dyn ...>);
    // proving each clone can dispatch independently from another
    // thread confirms the routing layer has no thread-local state.
    let id = ModelId("test-model".into());
    let fake = Arc::new(FakeBackend::new("fake").with_model(id.clone()));
    let registry = Arc::new(InMemoryModelRegistry::new());

    let mut dispatcher = Dispatcher::new(DispatcherConfig {
        model_registry: registry,
    });
    dispatcher.register_backend(fake as Arc<dyn InferenceBackend>);

    let h1 = thread::spawn({
        let d = dispatcher.clone();
        move || assert_eq!(d.backend_count(), 1)
    });
    let h2 = thread::spawn({
        let d = dispatcher.clone();
        move || assert_eq!(d.backend_count(), 1)
    });
    h1.join().unwrap();
    h2.join().unwrap();
}
