//! Batching-hook unit tests (PR 8 DoD: batching hook present + tested
//! with a fake backend). Proves the seam: a backend can override
//! `batch_dispatch` and the override is the one that runs; the default
//! impl threads through `dispatch` per item.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use common::FakeBackend;
use kx_content::ContentRef;
use kx_inference::{BatchItem, InferenceBackend, InferenceInput, InferenceParams};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

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
            max_calls: 10,
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
fn batching_default_impl_dispatches_per_item() {
    // Use a backend that does NOT override batch_dispatch; we wrap our
    // FakeBackend in a transparent newtype that pins down the default
    // impl.
    struct DefaultBatchBackend(FakeBackend);
    impl kx_inference::InferenceBackend for DefaultBatchBackend {
        fn dispatch(
            &self,
            m: &ModelId,
            i: &InferenceInput,
            p: &InferenceParams,
            w: &WarrantSpec,
        ) -> Result<kx_inference::InferenceOutput, kx_inference::InferenceError> {
            self.0.dispatch(m, i, p, w)
        }
        fn supports(&self, m: &ModelId) -> bool {
            self.0.supports(m)
        }
        fn name(&self) -> &'static str {
            "default-batch"
        }
        // NO batch_dispatch override — default impl applies.
    }

    let id = ModelId("test-model".into());
    let inner = FakeBackend::new("fake").with_model(id.clone());
    let dispatch_counter = inner.dispatch_calls.clone();
    let backend = DefaultBatchBackend(inner);

    let warrant = dummy_warrant(id.clone());
    let prompt = InferenceInput::Text("hi".into());
    let params = InferenceParams::default();

    let items: Vec<BatchItem<'_>> = (0..3)
        .map(|_| BatchItem {
            model_id: &id,
            input: &prompt,
            params: &params,
            warrant: &warrant,
        })
        .collect();

    let results = backend.batch_dispatch(&items);
    assert_eq!(results.len(), 3);
    for r in results {
        let out = r.expect("dispatch should succeed");
        assert_eq!(out.bytes, b"FAKE OUTPUT");
    }
    assert_eq!(
        dispatch_counter.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "default batch_dispatch should call inner.dispatch once per item"
    );
}

#[test]
fn batching_override_runs_once_per_batch() {
    // FakeBackend overrides batch_dispatch and bumps a separate counter
    // each call. Per-item dispatch counter still ticks because the
    // override forwards.
    let id = ModelId("test-model".into());
    let backend = FakeBackend::new("fake").with_model(id.clone());
    let warrant = dummy_warrant(id.clone());
    let prompt = InferenceInput::Text("hi".into());
    let params = InferenceParams::default();

    let items: Vec<BatchItem<'_>> = (0..4)
        .map(|_| BatchItem {
            model_id: &id,
            input: &prompt,
            params: &params,
            warrant: &warrant,
        })
        .collect();

    let results = backend.batch_dispatch(&items);
    assert_eq!(results.len(), 4);
    assert_eq!(backend.batch_count(), 1, "override called exactly once");
    assert_eq!(
        backend.dispatch_count(),
        4,
        "per-item dispatch should still tick"
    );

    let _ = PathBuf::new(); // silence unused import elsewhere
}
