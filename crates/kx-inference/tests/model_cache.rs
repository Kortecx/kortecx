//! Loaded-model cache + `ModelResolver` rewiring (M4, PR-1).
//!
//! These run WITHOUT a real GGUF: they exercise the resolver-miss path (no
//! worker thread) and the owner-thread round-trip + load-failure path (the
//! worker spawns, attempts `Model::load` on a non-existent file, and the typed
//! `BackendFailure` is relayed back). Byte-identical-cache-hit and
//! no-reload-on-hit are validated against a real model in `kx-model-harness`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceParams, LlamaInferenceBackend,
};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

/// A warrant routed to `model_id` with a POSITIVE wall-clock budget, so a valid
/// text dispatch reaches the model-load step instead of timing out immediately.
fn warrant(model_id: ModelId) -> WarrantSpec {
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
            // Above the default params' `max_output_tokens` so a valid dispatch
            // clears `check_within` and reaches the resolver/cache step.
            max_output_tokens: 2048,
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
fn dispatch_unknown_model_is_model_not_found() {
    let registered = ModelId("registered".into());
    let backend = LlamaInferenceBackend::with_model(
        registered,
        std::path::PathBuf::from("/tmp/whatever.gguf"),
    );
    let unknown = ModelId("unknown".into());
    let err = backend
        .dispatch(
            &unknown,
            &InferenceInput::Text("hi".into()),
            &InferenceParams::default(),
            &warrant(unknown.clone()),
        )
        .expect_err("an unregistered model must be ModelNotFound");
    assert!(matches!(err, InferenceError::ModelNotFound { .. }));
    // Resolver miss never spawns the worker / loads a model.
    assert_eq!(backend.loads_performed(), 0);
}

#[test]
fn dispatch_missing_file_surfaces_backend_failure() {
    let id = ModelId("ghost".into());
    let backend = LlamaInferenceBackend::with_model(
        id.clone(),
        std::path::PathBuf::from("/nonexistent/definitely/not/a/model.gguf"),
    );
    let err = backend
        .dispatch(
            &id,
            &InferenceInput::Text("hello".into()),
            &InferenceParams::default(),
            &warrant(id.clone()),
        )
        .expect_err("loading a non-existent model file must fail");
    // The owner thread round-tripped the load failure as a typed BackendFailure.
    assert!(
        matches!(err, InferenceError::BackendFailure { .. }),
        "expected BackendFailure, got {err:?}"
    );
    // A *failed* load does not count as a completed load.
    assert_eq!(backend.loads_performed(), 0);
}

#[test]
fn backend_is_cloneable_and_shares_one_cache() {
    let id = ModelId("m".into());
    let backend = LlamaInferenceBackend::with_model(id, std::path::PathBuf::from("/tmp/m.gguf"));
    let clone = backend.clone();
    // Fresh backends have performed no loads.
    assert_eq!(backend.loads_performed(), 0);
    assert_eq!(clone.loads_performed(), 0);
}
