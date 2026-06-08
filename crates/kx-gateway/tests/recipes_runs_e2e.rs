//! End-to-end witnesses for the UI-2 additive RPCs over a REAL bound port:
//!
//! - `ListRecipes` enumerates the server-provisioned invocable recipe handles;
//! - `GetRecipeForm` returns a recipe's typed free-param form (echo → `topic`
//!   STR); an unknown handle is `not_found` (a public discovery surface, NOT the
//!   uniform `permission_denied` of the Invoke execution surface);
//! - `ListRuns` enumerates the journal's registered runs newest-first with
//!   identity + recipe fingerprint + a wall-clock timestamp, and paginates by
//!   `before_seq`;
//! - all three are gated by the auth interceptor (deny-all refuses them).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, DEMO_RECIPE_HANDLE, FANOUT_RECIPE_HANDLE};
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

#[tokio::test]
async fn list_recipes_enumerates_the_provisioned_handles() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner()
        .recipes;
    let handles: Vec<&str> = recipes.iter().map(|r| r.handle.as_str()).collect();
    assert!(handles.contains(&DEMO_RECIPE_HANDLE));
    assert!(handles.contains(&FANOUT_RECIPE_HANDLE));

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn get_recipe_form_returns_the_typed_topic_field_and_not_found_for_unknown() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let form = c
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(form.handle, DEMO_RECIPE_HANDLE);
    assert_eq!(form.fields.len(), 1);
    assert_eq!(form.fields[0].name, "topic");
    assert_eq!(form.fields[0].r#type, proto::RecipeParamType::Str as i32);
    assert!(form.fields[0].required);
    assert_eq!(form.fields[0].max_len, Some(4096));

    // A public discovery surface: an unknown handle is `not_found` (honest), NOT
    // the uniform `permission_denied` collapse of the Invoke execution surface.
    let unknown = c
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: "kx/recipes/does-not-exist".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), tonic::Code::NotFound);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_runs_enumerates_the_durable_registered_run() {
    // GROUND TRUTH (single-node OSS): the coordinator registers ONE run per
    // journal (the `RunRegistered` fact is seq=1); every Invoke JOINS that run.
    // Distinct invocations are distinct TERMINAL MOTES within the one run
    // (distinguished by `terminal_mote_id`), NOT distinct instances. ListRuns
    // therefore enumerates the durable run INSTANCE(s) — the "re-open by
    // instance-id" primitive the journal could not expose before — which is 1
    // here. (Cloud multi-coordinator → multiple runs; that path is covered by the
    // gateway-core `runs::tests` pagination unit tests over synthetic journals.)
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Before any invocation: an empty journal lists no runs (not an error).
    let empty = c
        .list_runs(proto::ListRunsRequest {
            limit: None,
            before_seq: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(empty.runs.is_empty());

    // Two DISTINCT recipe invocations both JOIN the one registered run.
    let echo = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: br#"{"topic":"incidents"}"#.to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    let fan = c
        .invoke(proto::InvokeRequest {
            handle: FANOUT_RECIPE_HANDLE.to_string(),
            args: b"{}".to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        echo.instance_id, fan.instance_id,
        "single-node: every invocation joins the one run instance"
    );
    assert_ne!(
        echo.terminal_mote_id, fan.terminal_mote_id,
        "distinct invocations are distinct terminal Motes within the run"
    );

    // ListRuns enumerates the single durable run instance.
    let all = c
        .list_runs(proto::ListRunsRequest {
            limit: None,
            before_seq: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        all.runs.len(),
        1,
        "one registered run per journal (single-node)"
    );
    assert!(!all.has_more);
    let run = &all.runs[0];
    assert_eq!(run.instance_id, echo.instance_id);
    assert_eq!(
        run.recipe_fingerprint, echo.recipe_fingerprint,
        "the first registration (echo) is the run's recipe fingerprint"
    );
    assert_eq!(run.registered_seq, 1, "RunRegistered is the seq-1 fact");
    assert!(
        run.registered_unix_ms > 0,
        "a live registration stamps a wall-clock (audit-only)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn ui2_rpcs_are_gated_by_auth_under_deny_all() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // No credential under deny-all → every UI-2 RPC is refused by the interceptor.
    assert_eq!(
        c.list_runs(proto::ListRunsRequest {
            limit: None,
            before_seq: None
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        c.list_recipes(proto::ListRecipesRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        c.get_recipe_form(proto::GetRecipeFormRequest {
            handle: DEMO_RECIPE_HANDLE.to_string()
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::Unauthenticated
    );

    running.shutdown().await.unwrap();
}
