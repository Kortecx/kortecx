//! End-to-end witnesses for the UI-3 additive RPCs over a REAL bound port:
//!
//! - `ListTeams` enumerates the bootstrap-seeded demo team (owner + members);
//! - `ListTeamMembers` shows each member's role + caps, with a Delegate for role
//!   variety; with an `asset_ref` it populates the resolved warrant (membership ∩
//!   grant, ⊆ the team) — and `not_found` for an unknown team (a public viewer);
//! - `ListAssetGrants` shows the demo-recipe grants + the demo TEAM grant on echo;
//! - all three are gated by the auth interceptor (deny-all refuses them);
//! - the seeded team is durable + idempotent across a restart.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, DEMO_RECIPE_HANDLE, DEMO_TEAM_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::Request;

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

fn with_bearer<T>(payload: T, token: &str) -> Request<T> {
    let mut req = Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

/// Two auth-token parties (alice, bob) → a richer seeded team (alice + bob +
/// local-dev). The client authenticates as alice via her bearer token.
fn two_party_tokens() -> HashMap<String, String> {
    HashMap::from([
        ("tok-alice".to_string(), "alice@acme".to_string()),
        ("tok-bob".to_string(), "bob@acme".to_string()),
    ])
}

#[tokio::test]
async fn list_teams_and_members_shows_the_seeded_demo_team() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let teams = c
        .list_teams(with_bearer(proto::ListTeamsRequest {}, "tok-alice"))
        .await
        .unwrap()
        .into_inner()
        .teams;
    assert_eq!(teams.len(), 1, "exactly one bootstrap demo team");
    assert_eq!(teams[0].team_id, DEMO_TEAM_HANDLE);
    assert_eq!(teams[0].owner, "kx-gateway");
    // alice + bob + local-dev.
    assert_eq!(teams[0].member_count, 3);

    let members = c
        .list_team_members(with_bearer(
            proto::ListTeamMembersRequest {
                team_id: DEMO_TEAM_HANDLE.to_string(),
                asset_ref: None,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(members.owner, "kx-gateway");
    assert_eq!(members.members.len(), 3);
    let delegates = members
        .members
        .iter()
        .filter(|m| m.action_caps.contains(&"Delegate".to_string()))
        .count();
    assert_eq!(delegates, 1, "exactly one Delegate (role variety)");
    // No asset_ref ⇒ no resolved warrant.
    assert!(members.members.iter().all(|m| m.resolved_warrant.is_none()));

    // An unknown team is `not_found` (a public viewer surface).
    let unknown = c
        .list_team_members(with_bearer(
            proto::ListTeamMembersRequest {
                team_id: "kx/teams/nope".to_string(),
                asset_ref: None,
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), tonic::Code::NotFound);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn resolve_member_warrant_with_asset_ref_is_bounded_by_the_team() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // With the echo asset: a member resolves a warrant THROUGH the team membership
    // (the demo team holds Use on echo); it is ⊆ the team warrant (max_calls ≤ 3, no
    // escalation). The membership ∩ grant composition — the kx-fleet thesis — surfaced.
    let members = c
        .list_team_members(proto::ListTeamMembersRequest {
            team_id: DEMO_TEAM_HANDLE.to_string(),
            asset_ref: Some(DEMO_RECIPE_HANDLE.to_string()),
        })
        .await
        .unwrap()
        .into_inner();
    let resolved = members
        .members
        .iter()
        .find_map(|m| m.resolved_warrant.as_ref())
        .expect("a member resolves a warrant on echo through the team");
    assert!(
        resolved.max_calls <= 3,
        "no escalation past the team warrant"
    );
    assert!(!resolved.executor_class.is_empty());

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_asset_grants_shows_recipe_and_team_grants() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let grants = c
        .list_asset_grants(proto::ListAssetGrantsRequest {
            asset_ref: DEMO_RECIPE_HANDLE.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(grants.owner, "kx-gateway");
    // The demo team is granted Use on echo (root, active) — the resolve path's source.
    let team_grant = grants
        .grants
        .iter()
        .find(|g| g.grantee == DEMO_TEAM_HANDLE)
        .expect("the demo team holds a grant on echo");
    assert!(team_grant.is_root);
    assert!(!team_grant.revoked);
    assert!(team_grant.actions.contains(&"Use".to_string()));
    // No revoked grants were seeded.
    assert!(grants.grants.iter().all(|g| !g.revoked));

    // An unknown asset is `not_found` (a public viewer surface, honest code).
    let unknown = c
        .list_asset_grants(proto::ListAssetGrantsRequest {
            asset_ref: "kx/recipes/does-not-exist".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), tonic::Code::NotFound);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn ui3_rpcs_are_gated_by_auth_under_deny_all() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // No credential under deny-all → every UI-3 RPC is refused by the interceptor
    // (the transport-level auth wraps every method, including the new ones).
    assert_eq!(
        c.list_teams(proto::ListTeamsRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        c.list_team_members(proto::ListTeamMembersRequest {
            team_id: DEMO_TEAM_HANDLE.to_string(),
            asset_ref: None,
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        c.list_asset_grants(proto::ListAssetGrantsRequest {
            asset_ref: DEMO_RECIPE_HANDLE.to_string(),
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::Unauthenticated
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_demo_team_is_durable_and_idempotent_across_restart() {
    // The membership ledger (members.db) lives in the catalog dir (alongside the
    // journal); re-seeding on every start is idempotent (content-addressed fact
    // dedup), so a restart keeps exactly one team with the same members.
    let dir = tempfile::TempDir::new().unwrap();

    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let first = c
        .list_teams(proto::ListTeamsRequest {})
        .await
        .unwrap()
        .into_inner()
        .teams;
    assert_eq!(first.len(), 1);
    let count_before = first[0].member_count;
    running.shutdown().await.unwrap();

    // Restart on the SAME dir (same durable ledgers).
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let again = c
        .list_teams(proto::ListTeamsRequest {})
        .await
        .unwrap()
        .into_inner()
        .teams;
    assert_eq!(again.len(), 1, "still exactly one team after a restart");
    assert_eq!(
        again[0].member_count, count_before,
        "idempotent re-seed never double-admits across a restart"
    );

    running.shutdown().await.unwrap();
}
