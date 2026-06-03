//! Trait-seam test (per D35 + 02-crate-specs.md DoD):
//!
//! Proves a backend with an OUT-OF-PROCESS-SHAPED internal pattern
//! (here: a queue + worker-style indirection) compiles against the
//! same `InferenceBackend` trait the in-process `LlamaInferenceBackend`
//! uses. The point is to demonstrate that future cloud backends
//! (vLLM, Triton, remote APIs) fit without trait change.
//!
//! The "out-of-process shape" is simulated by performing the dispatch
//! through an internal channel + worker thread; from the caller's
//! perspective, it's still a synchronous `dispatch()` call. This is
//! the exact shape a Triton client would have if it blocked on its
//! gRPC call internally.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use kx_content::ContentRef;
use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

/// Out-of-process-shaped backend: every dispatch routes through an
/// mpsc channel to a worker thread. The worker computes the canned
/// output and returns it via a oneshot-style response channel. This
/// is the typical shape an RPC client would take if it serialised
/// blocking calls.
struct ProxiedBackend {
    sender: mpsc::SyncSender<(
        ModelId,
        mpsc::SyncSender<Result<InferenceOutput, InferenceError>>,
    )>,
    _worker: Arc<JoinHandle<()>>,
}

impl ProxiedBackend {
    fn new(model_id: ModelId) -> Self {
        let (tx, rx) = mpsc::sync_channel::<(
            ModelId,
            mpsc::SyncSender<Result<InferenceOutput, InferenceError>>,
        )>(4);
        let target_id = model_id.clone();
        let worker = thread::spawn(move || {
            while let Ok((req_id, resp)) = rx.recv() {
                let out = if req_id == target_id {
                    Ok(InferenceOutput {
                        bytes: b"PROXY OUTPUT".to_vec(),
                        output_tokens: 2,
                        backend_name: "proxy-backend",
                        model_id: req_id,
                        elapsed: Duration::from_millis(2),
                    })
                } else {
                    Err(InferenceError::ModelNotFound {
                        model_id: req_id.0.clone(),
                    })
                };
                let _ = resp.send(out);
            }
        });
        Self {
            sender: tx,
            _worker: Arc::new(worker),
        }
    }
}

impl InferenceBackend for ProxiedBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.sender
            .send((model_id.clone(), tx))
            .map_err(|e| InferenceError::BackendFailure {
                backend: "proxy-backend",
                message: format!("worker channel disconnected: {e}"),
            })?;
        rx.recv().map_err(|e| InferenceError::BackendFailure {
            backend: "proxy-backend",
            message: format!("response channel disconnected: {e}"),
        })?
    }

    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "proxy-backend"
    }
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
fn proxied_backend_satisfies_trait() {
    let id = ModelId("proxy-model".into());
    let backend: Arc<dyn InferenceBackend> = Arc::new(ProxiedBackend::new(id.clone()));
    let warrant = dummy_warrant(id.clone());
    let input = InferenceInput::Text("hello".into());
    let params = InferenceParams::default();

    let out = backend
        .dispatch(&id, &input, &params, &warrant)
        .expect("proxy dispatch should succeed");
    assert_eq!(out.bytes, b"PROXY OUTPUT");
    assert_eq!(out.backend_name, "proxy-backend");
}

#[test]
fn proxied_backend_returns_model_not_found_on_unknown() {
    let id = ModelId("proxy-model".into());
    let other = ModelId("other-model".into());
    let backend: Arc<dyn InferenceBackend> = Arc::new(ProxiedBackend::new(id.clone()));
    // Warrant authorises 'other' but the proxy was constructed with 'proxy-model'.
    let warrant = dummy_warrant(other.clone());
    let input = InferenceInput::Text("hello".into());
    let params = InferenceParams::default();
    let err = backend
        .dispatch(&other, &input, &params, &warrant)
        .expect_err("unknown model must fail");
    assert!(matches!(err, InferenceError::ModelNotFound { .. }));
}
