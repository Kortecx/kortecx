//! Batch B (PR-2) — `GetMoteDetail` over the in-process tonic round-trip, plus
//! the structured `kx-refusal-code` metadata on a coordinator-refused submit.
//!
//! The contract under test: ownership denies UNIFORMLY (no oracle); an unknown
//! mote in an OWNED run is an honest `NOT_FOUND`; an uncommitted mote and a
//! pre-Batch-B (blob-less) committed mote both answer `def_found = false`
//! (never an error); a wired seam resolves the full capped detail; a seamless
//! gateway degrades to `unimplemented`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{
    build_run, mote_def, sample_mote, sample_warrant, service_from, spawn, MockSubmitter,
    INSTANCE_ID, RECIPE_FP,
};
use kx_gateway_core::{
    GatewayError, MoteDefView, RunSubmitter, SubmitMoteOutcome, SubmitterError,
    REFUSAL_CODE_METADATA_KEY,
};
use kx_mote::{MoteDef, NdClass};
use kx_proto::proto;
use kx_warrant::WarrantSpec;
use tonic::Code;

/// A stub seam over an in-memory hash → def map (the host impl reads the same
/// mapping out of the content store).
#[derive(Default)]
struct StubDefView {
    defs: BTreeMap<[u8; 32], MoteDef>,
}

impl StubDefView {
    fn with(mut self, def: MoteDef) -> Self {
        self.defs.insert(*def.hash().as_bytes(), def);
        self
    }
}

impl MoteDefView for StubDefView {
    fn get_def(&self, mote_def_hash: &[u8; 32]) -> Result<Option<MoteDef>, GatewayError> {
        Ok(self.defs.get(mote_def_hash).cloned())
    }
}

fn detail_request(instance_id: &[u8], mote_id: &[u8]) -> proto::GetMoteDetailRequest {
    proto::GetMoteDetailRequest {
        instance_id: instance_id.to_vec(),
        mote_id: mote_id.to_vec(),
    }
}

#[tokio::test]
async fn full_detail_round_trips_for_a_committed_mote() {
    let run = build_run();
    let a = run.a;
    // Mote A committed with mote_def(0x01)'s hash (the build_run fixture).
    let def_a = mote_def(0x01, NdClass::Pure);
    let expected_hash = def_a.hash().as_bytes().to_vec();
    let svc = service_from(run, Arc::new(MockSubmitter::default()))
        .with_mote_def_view(Arc::new(StubDefView::default().with(def_a)));
    let mut client = spawn(svc).await;

    let detail = client
        .get_mote_detail(detail_request(&INSTANCE_ID, a.as_bytes()))
        .await
        .unwrap()
        .into_inner();
    assert!(detail.def_found);
    assert_eq!(detail.mote_id, a.as_bytes().to_vec());
    assert_eq!(detail.mote_def_hash, expected_hash);
    assert_eq!(detail.model_id, "test-model");
    assert_eq!(detail.step_kind, "pure");
    assert_eq!(detail.nd_class, proto::NdClass::Pure as i32);
    assert_eq!(detail.logic_ref, vec![7u8; 32]);
    // The fixture def carries one config entry ("tag"); no prompt.
    assert_eq!(detail.config_subset.len(), 1);
    assert_eq!(detail.config_subset[0].key, "tag");
    assert!(detail.prompt.is_empty());
    assert_eq!(
        detail.schema_version,
        u32::from(kx_mote::MOTE_DEF_SCHEMA_VERSION)
    );
}

#[tokio::test]
async fn wrong_instance_is_uniformly_denied() {
    let run = build_run();
    let a = run.a;
    let svc = service_from(run, Arc::new(MockSubmitter::default()))
        .with_mote_def_view(Arc::new(StubDefView::default()));
    let mut client = spawn(svc).await;

    let err = client
        .get_mote_detail(detail_request(&[0x99; 16], a.as_bytes()))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(err.message(), "not authorized", "uniform denial, no oracle");
}

#[tokio::test]
async fn unknown_mote_in_an_owned_run_is_not_found() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_mote_def_view(Arc::new(StubDefView::default()));
    let mut client = spawn(svc).await;

    let err = client
        .get_mote_detail(detail_request(&INSTANCE_ID, &[0xEE; 32]))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::NotFound,
        "the owner can already enumerate motes — honest, not an oracle"
    );
}

#[tokio::test]
async fn uncommitted_mote_answers_def_found_false_with_no_hash() {
    let run = build_run();
    let c = run.c; // Proposed-only (Scheduled) — no Committed fact, no def hash.
    let svc = service_from(run, Arc::new(MockSubmitter::default()))
        .with_mote_def_view(Arc::new(StubDefView::default()));
    let mut client = spawn(svc).await;

    let detail = client
        .get_mote_detail(detail_request(&INSTANCE_ID, c.as_bytes()))
        .await
        .unwrap()
        .into_inner();
    assert!(!detail.def_found);
    assert!(
        detail.mote_def_hash.is_empty(),
        "the hash only exists on a Committed fact"
    );
}

#[tokio::test]
async fn blobless_committed_mote_answers_def_found_false_with_the_hash() {
    // The pre-Batch-B back-compat shape: the journal names a def hash but the
    // store never held the blob (admitted by an old binary). Honest empty.
    let run = build_run();
    let a = run.a;
    let svc = service_from(run, Arc::new(MockSubmitter::default()))
        .with_mote_def_view(Arc::new(StubDefView::default())); // empty: no blobs
    let mut client = spawn(svc).await;

    let detail = client
        .get_mote_detail(detail_request(&INSTANCE_ID, a.as_bytes()))
        .await
        .unwrap()
        .into_inner();
    assert!(!detail.def_found);
    assert_eq!(
        detail.mote_def_hash,
        mote_def(0x01, NdClass::Pure).hash().as_bytes().to_vec(),
        "the committed hash still surfaces (the def bytes just are not retained)"
    );
}

#[tokio::test]
async fn no_seam_degrades_to_unimplemented() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    let a = mote_def(0x01, NdClass::Pure); // any id — the seam gate fires first
    let err = client
        .get_mote_detail(detail_request(&INSTANCE_ID, a.hash().as_bytes()))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn malformed_ids_are_invalid_argument() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_mote_def_view(Arc::new(StubDefView::default()));
    let mut client = spawn(svc).await;
    let err = client
        .get_mote_detail(detail_request(&[1, 2, 3], &[0xAA; 32]))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    let err = client
        .get_mote_detail(detail_request(&INSTANCE_ID, &[1, 2, 3]))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

// --- the structured refusal-code metadata (the coordinator-refused path) ---

/// A submitter that refuses every Mote with the STRUCTURED code (the shape
/// `TonicCoordinatorSubmitter` produces from `SubmitMoteResponse.refusal_code`).
struct RefusingSubmitter;

#[tonic::async_trait]
impl RunSubmitter for RefusingSubmitter {
    async fn register_run(
        &self,
        _recipe_fingerprint: [u8; 32],
    ) -> Result<[u8; 16], SubmitterError> {
        Ok(INSTANCE_ID)
    }

    async fn submit_mote(
        &self,
        _mote: kx_mote::Mote,
        _warrant: WarrantSpec,
        _accept_at_least_once: bool,
        _react_seed: bool,
    ) -> Result<SubmitMoteOutcome, SubmitterError> {
        Err(SubmitterError::Refused {
            code: "R-1".to_string(),
            detail: "R-1: refused for the metadata test".to_string(),
        })
    }
}

#[tokio::test]
async fn refused_submit_carries_the_refusal_code_metadata() {
    let svc = service_from(build_run(), Arc::new(RefusingSubmitter));
    let mut client = spawn(svc).await;
    let err = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: RECIPE_FP.to_vec(),
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(sample_mote().into()),
                warrant: Some(sample_warrant().into()),
                accept_at_least_once: false,
                react_seed: false,
            }],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(
        err.metadata()
            .get(REFUSAL_CODE_METADATA_KEY)
            .and_then(|v| v.to_str().ok()),
        Some("R-1"),
        "the structured code rides the trailers"
    );
    assert!(err.message().contains("R-1"), "the prose stays human");
}
