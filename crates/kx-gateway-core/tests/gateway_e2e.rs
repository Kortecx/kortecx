//! In-process `KxGateway` round-trips over a real tonic transport server. Three
//! enterprise scenarios (operator renders a run DAG; end-user fetches a committed
//! result; resumable event stream) plus the SubmitRun propose-proxy and the SN-8
//! "client never computes a MoteId" boundary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;

use common::{
    build_run, sample_mote, sample_warrant, service_from, spawn, MockSubmitter, INSTANCE_ID,
    RECIPE_FP,
};
use kx_gateway_core::RunSubmitter;
use kx_proto::proto;
use tonic::Code;

fn no_submitter() -> Arc<dyn RunSubmitter> {
    Arc::new(MockSubmitter::default())
}

// --- Scenario A — operator renders a live run DAG -------------------------

#[tokio::test]
async fn scenario_a_renders_run_dag_server_derived() {
    let run = build_run();
    let (a, b, c) = (run.a, run.b, run.c);
    let mut client = spawn(service_from(run, no_submitter())).await;

    let view = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: INSTANCE_ID.to_vec(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(view.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(view.recipe_fingerprint, RECIPE_FP.to_vec());
    assert_eq!(view.current_seq, 4);
    assert_eq!(view.motes.len(), 3);

    let snap = |id: kx_mote::MoteId| {
        view.motes
            .iter()
            .find(|m| m.mote_id == id.as_bytes().to_vec())
            .cloned()
            .unwrap_or_else(|| panic!("missing snapshot for {id:?}"))
    };

    // Every mote_id is server-derived (it came from the journal fold, not a request).
    let sa = snap(a);
    assert_eq!(sa.state, proto::MoteSnapshotState::Committed as i32);
    assert!(sa.result_ref.is_some());
    assert_eq!(
        sa.mote_def_hash.len(),
        32,
        "committed snapshot carries its def hash"
    );
    assert!(sa.warrant_ref.is_some());
    assert!(sa.verdict.is_none(), "verdict opaque/absent at freeze");

    let sb = snap(b);
    assert_eq!(sb.state, proto::MoteSnapshotState::Committed as i32);
    assert_eq!(sb.parents.len(), 1, "B is a data-child of A");
    assert_eq!(sb.parents[0].parent_id, a.as_bytes().to_vec());

    let sc = snap(c);
    assert_eq!(sc.state, proto::MoteSnapshotState::Scheduled as i32);
    assert!(sc.committed_seq.is_none());
}

#[tokio::test]
async fn scenario_a_at_seq_is_clamped_to_head() {
    let run = build_run();
    let mut client = spawn(service_from(run, no_submitter())).await;
    // A far-future at_seq must yield the head, never an error that leaks the head.
    let view = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: INSTANCE_ID.to_vec(),
            at_seq: Some(9_999),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(view.current_seq, 4);
}

#[tokio::test]
async fn scenario_a_wrong_instance_is_uniform_denied() {
    let run = build_run();
    let mut client = spawn(service_from(run, no_submitter())).await;
    let err = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: [0x99; 16].to_vec(),
            at_seq: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(err.message(), "not authorized");
}

// --- Scenario B — end-user fetches a committed result (no oracle) ----------

#[tokio::test]
async fn scenario_b_get_content_owned_then_uniform_denials() {
    let run = build_run();
    let a_ref = run.a_ref;
    let mut client = spawn(service_from(run, no_submitter())).await;

    // Owned committed ref → bytes.
    let blob = client
        .get_content(proto::GetContentRequest {
            content_ref: a_ref.0.to_vec(),
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(blob.payload, b"result-of-A");

    // Wrong instance and unknown ref must BOTH return the identical denial — no
    // existence oracle (a caller cannot tell "not yours" from "doesn't exist").
    let wrong_instance = client
        .get_content(proto::GetContentRequest {
            content_ref: a_ref.0.to_vec(),
            instance_id: [0x99; 16].to_vec(),
        })
        .await
        .unwrap_err();
    let unknown_ref = client
        .get_content(proto::GetContentRequest {
            content_ref: [0xEE; 32].to_vec(),
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(wrong_instance.code(), Code::PermissionDenied);
    assert_eq!(unknown_ref.code(), Code::PermissionDenied);
    assert_eq!(wrong_instance.message(), unknown_ref.message());
    assert_eq!(wrong_instance.message(), "not authorized");
}

// --- Scenario C — resumable event stream -----------------------------------

async fn collect_frames(
    client: &mut kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    since_seq: u64,
) -> Vec<proto::EventFrame> {
    let mut stream = client
        .stream_events(proto::StreamEventsRequest {
            instance_id: INSTANCE_ID.to_vec(),
            since_seq,
        })
        .await
        .unwrap()
        .into_inner();
    let mut frames = Vec::new();
    while let Some(frame) = stream.message().await.unwrap() {
        frames.push(frame);
    }
    frames
}

fn committed_mote_ids(frames: &[proto::EventFrame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .flat_map(|f| &f.deltas)
        .filter_map(|d| match &d.kind {
            Some(proto::event_delta::Kind::Committed(c)) => Some(c.mote_id.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn scenario_c_stream_from_start_then_resume() {
    let run = build_run();
    let (a, b) = (run.a, run.b);
    let mut client = spawn(service_from(run, no_submitter())).await;

    // From the start: A + B committed deltas; the proposed C is NOT surfaced.
    let frames = collect_frames(&mut client, 0).await;
    let committed = committed_mote_ids(&frames);
    assert!(committed.contains(&a.as_bytes().to_vec()));
    assert!(committed.contains(&b.as_bytes().to_vec()));
    let last = frames.last().unwrap();
    assert!(last.journal_boundary);
    assert_eq!(last.next_seq, 4, "next_seq never exceeds head");

    // Resume at A's seq (2): only B's committed delta (seq 3) is newer.
    let resumed = collect_frames(&mut client, 2).await;
    let committed_after = committed_mote_ids(&resumed);
    assert_eq!(committed_after, vec![b.as_bytes().to_vec()]);
    assert!(resumed.iter().all(|f| f.next_seq <= 4));
}

// --- SubmitRun — propose-proxy ordering ------------------------------------

#[tokio::test]
async fn submit_run_registers_first_then_submits() {
    let mock = MockSubmitter::default();
    let svc = service_from(build_run(), Arc::new(mock.clone()));
    let mut client = spawn(svc).await;

    let handle = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: RECIPE_FP.to_vec(),
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(sample_mote().into()),
                warrant: Some(sample_warrant().into()),
                accept_at_least_once: false,
            }],
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(handle.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(handle.recipe_fingerprint, RECIPE_FP.to_vec());
    // register_run is proxied BEFORE any submit_mote (never ack ahead of the run).
    let calls = mock.calls();
    assert_eq!(calls.first().map(String::as_str), Some("register_run(34)")); // 0x22 == 34
    assert!(calls.iter().any(|c| c == "submit_mote"));
    assert!(
        calls.iter().position(|c| c.starts_with("register_run"))
            < calls.iter().position(|c| c == "submit_mote")
    );
}

#[tokio::test]
async fn submit_run_rejects_malformed_recipe_fingerprint() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    let err = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: vec![0x22; 4], // not 32 bytes
            motes: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

// --- SN-8 — the client never computes a MoteId -----------------------------

#[test]
fn submit_boundary_rederives_mote_id_discarding_wire_advisory() {
    // A genuine Mote and its wire form.
    let mote = sample_mote();
    let expected = *mote.id.as_bytes();
    let mut wire: proto::Mote = mote.into();
    // Tamper with the advisory wire mote_id.
    wire.mote_id = vec![0xFF; 32];
    // The boundary re-derives identity Rust-side (D53/SN-8): the tampered
    // advisory id is discarded; the re-derived id matches the genuine one.
    let rebuilt: kx_mote::Mote = wire.try_into().unwrap();
    assert_eq!(*rebuilt.id.as_bytes(), expected);
    assert_ne!(*rebuilt.id.as_bytes(), [0xFF; 32]);
}

// --- Stubbed signature RPCs ------------------------------------------------

#[tokio::test]
async fn signature_rpcs_are_unimplemented_at_freeze() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    let err = client
        .list_signatures(proto::ListSignaturesRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}
