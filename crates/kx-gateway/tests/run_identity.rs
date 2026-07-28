//! The replay-signature witness behind the bench's pass^k phase (model-free, always-on).
//!
//! The phase re-runs its flagship tasks K times and calls each run an independent
//! trial. That claim rests on run identity: an identical Invoke on the SAME serve
//! resolves to the same bound Mote ids, so the journal serves the committed result
//! back — one `instance_id`, K "trials", zero re-executions. A FRESH state dir is what
//! actually re-executes: its journal mints a new nonce `instance_id` under the same
//! input-addressed bind identity.
//!
//! This test proves the detector the phase uses reads DIFFERENTLY in the two regimes —
//! the L-196 question — without spending a second of model time: same-serve re-invoke
//! ⇒ SAME instance id (the replay signature the phase must never see across trials);
//! fresh-serve re-invoke ⇒ DIFFERENT instance id under an EQUAL terminal Mote id (real
//! re-execution of the same work). A pass^k phase whose trials shared an instance id
//! would be replaying one trial, and the phase's disjointness assert is exactly this
//! check run in anger.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, DEMO_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

async fn invoke_and_settle(
    c: &mut KxGatewayClient<Channel>,
) -> (Vec<u8>, Vec<u8>) {
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: b"{\"topic\":\"pass-k-trial\"}".to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke the demo recipe")
        .into_inner();
    // Settle: the identity claim below is about the POST-COMMIT regime (a cache hit is
    // only possible once the first run committed).
    for _ in 0..100 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: resp.instance_id.clone(),
                at_seq: None,
            })
            .await
            .expect("read the projection")
            .into_inner();
        let committed = view.motes.iter().any(|m| {
            m.mote_id == resp.terminal_mote_id
                && m.state == proto::MoteSnapshotState::Committed as i32
        });
        if committed {
            return (resp.instance_id, resp.terminal_mote_id);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the invoked recipe never committed");
}

#[tokio::test]
async fn a_replay_and_a_fresh_execution_read_differently() {
    // Serve A: the same Invoke twice. The second lands AFTER the first committed, so
    // the journal has the result — the runs must share ONE identity.
    let dir_a = tempfile::TempDir::new().unwrap();
    let running_a = start(common::gateway_config(&dir_a, true, HashMap::new()))
        .await
        .unwrap();
    let mut ca = client(running_a.local_addr()).await;
    let (instance_1, terminal_1) = invoke_and_settle(&mut ca).await;
    let (instance_2, terminal_2) = invoke_and_settle(&mut ca).await;
    assert_eq!(
        instance_1, instance_2,
        "an identical re-invoke on one serve joins the SAME run — this shared id is \
         the replay signature the pass^k phase's disjointness assert exists to catch"
    );
    assert_eq!(terminal_1, terminal_2, "same bind ⇒ same terminal Mote");
    running_a.shutdown().await.unwrap();

    // Serve B: the same Invoke on a FRESH state dir. Same input-addressed bind
    // identity — a genuinely new execution under it.
    let dir_b = tempfile::TempDir::new().unwrap();
    let running_b = start(common::gateway_config(&dir_b, true, HashMap::new()))
        .await
        .unwrap();
    let mut cb = client(running_b.local_addr()).await;
    let (instance_3, terminal_3) = invoke_and_settle(&mut cb).await;
    assert_ne!(
        instance_1, instance_3,
        "a fresh state dir mints a fresh nonce instance id — real re-execution, the \
         regime every pass^k trial must be in"
    );
    assert_eq!(
        terminal_1, terminal_3,
        "the bind identity is input-addressed and serve-independent — identity lives \
         in the recipe, isolation lives in the state dir"
    );
    running_b.shutdown().await.unwrap();
}
