//! End-to-end witnesses for PR-4 Batch D over a REAL bound port:
//!
//! - `ListRecipes` now carries ADVISORY metadata (description / tags / version)
//!   for each provisioned recipe;
//! - `SearchRecipes` ranks the recipes against an intent (display-only basis
//!   points — an exact handle is `10000`, a tag/name/description match lower; a
//!   non-match is dropped), is best-first + deterministic, and respects `limit`;
//! - both are advisory: a score SURFACES a recipe, never invokes one.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, DEMO_RECIPE_HANDLE, PASSTHROUGH_DAG_HANDLE};
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
async fn list_recipes_carries_advisory_metadata() {
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
    let echo = recipes
        .iter()
        .find(|r| r.handle == DEMO_RECIPE_HANDLE)
        .expect("echo is provisioned");
    assert!(
        echo.description.contains("Echo"),
        "advisory description present"
    );
    assert!(
        echo.tags.iter().any(|t| t == "passthrough"),
        "advisory tags present"
    );
    // Content-addressed published version pin (12 hex), never a faked semver.
    assert_eq!(echo.version.len(), 12);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn search_recipes_ranks_exact_first_filters_and_caps() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Exact handle ⇒ rank 0 is that handle at 10000 bp; only positive matches.
    let ranked = c
        .search_recipes(proto::SearchRecipesRequest {
            intent: DEMO_RECIPE_HANDLE.to_string(),
            keywords: vec![],
            limit: None,
        })
        .await
        .unwrap()
        .into_inner()
        .ranked;
    let top = ranked.first().expect("at least one hit");
    assert_eq!(top.recipe.as_ref().unwrap().handle, DEMO_RECIPE_HANDLE);
    assert_eq!(top.score_bp, 10_000);
    assert!(ranked
        .iter()
        .all(|r| r.score_bp > 0 && r.score_bp <= 10_000));

    // A tag query surfaces every recipe carrying it (echo + passthrough-dag).
    let by_tag = c
        .search_recipes(proto::SearchRecipesRequest {
            intent: String::new(),
            keywords: vec!["passthrough".to_string()],
            limit: None,
        })
        .await
        .unwrap()
        .into_inner()
        .ranked;
    let hits: Vec<&str> = by_tag
        .iter()
        .map(|r| r.recipe.as_ref().unwrap().handle.as_str())
        .collect();
    assert!(hits.contains(&DEMO_RECIPE_HANDLE));
    assert!(hits.contains(&PASSTHROUGH_DAG_HANDLE));

    // limit caps the result set.
    let capped = c
        .search_recipes(proto::SearchRecipesRequest {
            intent: String::new(),
            keywords: vec![],
            limit: Some(1),
        })
        .await
        .unwrap()
        .into_inner()
        .ranked;
    assert_eq!(capped.len(), 1);

    running.shutdown().await.unwrap();
}
