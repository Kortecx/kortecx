//! PR-D `GetRunInputs` + the `Invoke` args capture over a real tonic transport —
//! the gateway-core boundary proofs:
//!
//! - the `Invoke` args (+ handle + fingerprint) are captured into the seam keyed
//!   by the run's `instance_id`, and `GetRunInputs` reads them back verbatim;
//! - the capture is BEST-EFFORT: a failing store NEVER fails the `Invoke` (the
//!   args are pre-fill convenience, not part of run admission);
//! - the seam is `None` ⇒ `GetRunInputs` degrades to `Unimplemented`
//!   (old-host forward-compat);
//! - a run with nothing captured is an honest `NotFound` (not an empty lie).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::{
    build_run, sample_mote, sample_warrant, service_from, spawn, spawn_with_party, MockSubmitter,
    INSTANCE_ID, RECIPE_FP,
};
use kx_gateway_core::{
    BinderError, BoundRecipe, GatewayError, RecipeBinder, RunInputsEntry, RunInputsRecord,
    RunInputsStore,
};
use kx_proto::proto;
use tonic::Code;

/// A binder that admits a single plain (non-react) Mote — the happy `Invoke`
/// path so the capture fires.
struct PlainBinder;
#[tonic::async_trait]
impl RecipeBinder for PlainBinder {
    async fn bind(
        &self,
        _party: &str,
        _handle: &str,
        _args: &[u8],
        _context_bundles: &[String],
        _context_refs: &[String],
    ) -> Result<BoundRecipe, BinderError> {
        Ok(BoundRecipe {
            recipe_fingerprint: RECIPE_FP,
            motes: vec![(sample_mote(), sample_warrant())],
            terminal_mote_id: sample_mote().id,
            react_seed: false,
        })
    }
}

/// An in-memory [`RunInputsStore`] fake (the host's `run_inputs.db` stand-in).
#[derive(Default)]
struct MemRunInputs {
    rows: Mutex<HashMap<[u8; 16], RunInputsRecord>>,
}
impl RunInputsStore for MemRunInputs {
    fn record(&self, rec: RunInputsRecord) -> Result<(), GatewayError> {
        self.rows.lock().unwrap().insert(rec.instance_id, rec); // INSERT OR REPLACE
        Ok(())
    }
    fn get(&self, instance_id: &[u8; 16]) -> Result<Option<RunInputsEntry>, GatewayError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(instance_id)
            .map(|r| RunInputsEntry {
                instance_id: r.instance_id,
                recipe_fingerprint: r.recipe_fingerprint,
                handle: r.handle.clone(),
                args: r.args.clone(),
            }))
    }
}

/// A store whose write ALWAYS fails — proves the capture is best-effort.
struct FailingRunInputs;
impl RunInputsStore for FailingRunInputs {
    fn record(&self, _rec: RunInputsRecord) -> Result<(), GatewayError> {
        Err(GatewayError::Internal("disk full".into()))
    }
    fn get(&self, _instance_id: &[u8; 16]) -> Result<Option<RunInputsEntry>, GatewayError> {
        Ok(None)
    }
}

#[tokio::test]
async fn invoke_captures_args_then_get_run_inputs_returns_them() {
    let store = Arc::new(MemRunInputs::default());
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_recipe_binder(Arc::new(PlainBinder))
        .with_run_inputs_store(store.clone());
    let mut client = spawn_with_party(svc, "alice").await;

    client
        .invoke(proto::InvokeRequest {
            handle: "kx/recipes/echo".into(),
            args: br#"{"topic":"hi"}"#.to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke admitted");

    let got = client
        .get_run_inputs(proto::GetRunInputsRequest {
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .expect("captured")
        .into_inner();
    assert_eq!(got.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(got.recipe_fingerprint, RECIPE_FP.to_vec());
    assert_eq!(got.handle, "kx/recipes/echo");
    assert_eq!(got.args, br#"{"topic":"hi"}"#.to_vec());
}

#[tokio::test]
async fn invoke_succeeds_when_capture_fails() {
    // Best-effort capture: a failing sidecar must NOT fail the run admission.
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_recipe_binder(Arc::new(PlainBinder))
        .with_run_inputs_store(Arc::new(FailingRunInputs));
    let mut client = spawn_with_party(svc, "alice").await;
    client
        .invoke(proto::InvokeRequest {
            handle: "kx/recipes/echo".into(),
            args: b"{}".to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke still admitted despite a failing run-inputs capture");
}

#[tokio::test]
async fn get_run_inputs_unimplemented_without_store() {
    // No sidecar wired ⇒ forward-compat degrade (an old host).
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    let err = client
        .get_run_inputs(proto::GetRunInputsRequest {
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn get_run_inputs_not_found_for_uncaptured_run() {
    // A run with nothing captured (pre-PR-D / rebuilt-to-empty) is honest NotFound.
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_run_inputs_store(Arc::new(MemRunInputs::default()));
    let mut client = spawn(svc).await;
    let err = client
        .get_run_inputs(proto::GetRunInputsRequest {
            instance_id: [0x99; 16].to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn get_run_inputs_rejects_malformed_instance_id() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_run_inputs_store(Arc::new(MemRunInputs::default()));
    let mut client = spawn(svc).await;
    let err = client
        .get_run_inputs(proto::GetRunInputsRequest {
            instance_id: vec![0x01, 0x02], // not 16 bytes
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}
