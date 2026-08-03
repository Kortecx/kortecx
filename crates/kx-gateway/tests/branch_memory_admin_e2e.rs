//! W3 — the branch and memory admin RPCs, driven through a REAL bound gateway port.
//!
//! ## The scenario, named before the code (Rule 47)
//!
//! An operator keeps working state in branches: they snapshot files in from disk, page
//! through what they have, and delete the ones they are done with. Separately, an agent
//! remembers things, and the operator asks it to forget one. Every one of those actions
//! must do exactly what it says AND stop at the boundary of the person asking: another
//! party's branches are neither listed, nor deleted, nor readable.
//!
//! ## Why this file exists
//!
//! `SnapshotInto`, `DeleteBranch` and `ForgetMemory` each had exactly ONE non-generated
//! caller — the handler and the CLI verb — and no test above their store. The structural
//! proof is stronger than a grep: `with_branches_store` and `with_memory_view` are
//! referenced ONLY from `server.rs`, so no in-process gateway test could reach these
//! handlers' non-`unimplemented` paths at all.
//!
//! **`ListBranches` is the exception, and the difference matters.** `kx-cli`'s
//! `json_contract` DOES drive `kx branch list --json` against a real gateway — but it
//! asserts only that the process exits 0 and prints one JSON value. That is true of a
//! `ListBranches` that returns nothing, forever, for anyone. What was missing is an
//! ORACLE: pagination that actually pages, a cursor that actually advances, and a caller
//! scope that actually excludes. So this is not "the first test", it is the first test
//! whose result would differ if the RPC stopped working.
//!
//! Model-free by construction: branches are store operations, and the memory arm uses the
//! CLIENT-VECTOR path (`embedding` / `query_embedding`), so nothing here needs an embedder.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::{Code, Request};

mod common;

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

fn with_bearer<T>(payload: T, token: &str) -> Request<T> {
    let mut req = Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

fn two_party_tokens() -> HashMap<String, String> {
    HashMap::from([
        ("tok-alice".to_string(), "alice@acme".to_string()),
        ("tok-bob".to_string(), "bob@acme".to_string()),
    ])
}

async fn create(c: &mut KxGatewayClient<Channel>, token: &str, handle: &str) {
    c.create_branch(with_bearer(
        proto::CreateBranchRequest {
            handle: handle.to_string(),
            description: "w3 fixture".into(),
            parent_handle: String::new(),
        },
        token,
    ))
    .await
    .unwrap_or_else(|e| panic!("create {handle}: {e}"));
}

async fn del(c: &mut KxGatewayClient<Channel>, token: &str, handle: &str) -> bool {
    c.delete_branch(with_bearer(
        proto::DeleteBranchRequest {
            handle: handle.to_string(),
        },
        token,
    ))
    .await
    .unwrap_or_else(|e| panic!("delete {handle}: {e}"))
    .into_inner()
    .removed
}

async fn handles(c: &mut KxGatewayClient<Channel>, token: &str, limit: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut after = String::new();
    // Walk the cursor to exhaustion rather than trusting one page — a cursor that never
    // advances would otherwise read as "one page, done".
    for _ in 0..32 {
        let page = c
            .list_branches(with_bearer(
                proto::ListBranchesRequest {
                    limit,
                    after_handle: after.clone(),
                },
                token,
            ))
            .await
            .unwrap()
            .into_inner();
        let got: Vec<String> = page.branches.iter().map(|b| b.handle.clone()).collect();
        assert!(
            got.len() <= limit as usize,
            "a page returned {} rows for limit {limit}",
            got.len()
        );
        out.extend(got);
        if !page.has_more {
            return out;
        }
        after = out
            .last()
            .cloned()
            .expect("has_more with an empty page would never terminate");
    }
    panic!("the cursor never exhausted — it is not advancing");
}

/// `ListBranches` pages, its cursor advances, and it never leaks another party's rows.
///
/// The pre-existing smoke test would pass over an empty list; each assertion here fails if
/// the RPC returns nothing, returns everything, or returns the same page forever.
#[tokio::test]
async fn list_branches_pages_through_a_moving_cursor_and_is_caller_scoped() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let mine: Vec<String> = (0..5).map(|i| format!("team/w3/br-{i}")).collect();
    for h in &mine {
        create(&mut c, "tok-alice", h).await;
    }
    // Bob's own branch, created in the same store, is the control that makes the scope
    // assertion mean something: without it "alice sees only hers" is true of an empty DB.
    create(&mut c, "tok-bob", "team/w3/bobs-own").await;

    // A page size SMALLER than the row count, so paging is exercised rather than assumed.
    let seen = handles(&mut c, "tok-alice", 2).await;
    assert_eq!(seen, mine, "every branch, in handle order, exactly once");

    let bobs = handles(&mut c, "tok-bob", 100).await;
    assert_eq!(
        bobs,
        vec!["team/w3/bobs-own".to_string()],
        "bob sees his own branch and none of alice's"
    );

    // The cursor is EXCLUSIVE: asking after the last handle yields nothing more.
    let tail = c
        .list_branches(with_bearer(
            proto::ListBranchesRequest {
                limit: 100,
                after_handle: mine.last().unwrap().clone(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(tail.branches.is_empty() && !tail.has_more);
}

/// `DeleteBranch` removes exactly one branch, for exactly its owner.
///
/// The bool alone is a weak witness — a handler that always answered `true` would pass a
/// test that only reads it. Every arm here also asserts what SURVIVED.
#[tokio::test]
async fn delete_branch_removes_one_branch_and_only_for_its_owner() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    for i in 0..3 {
        create(&mut c, "tok-alice", &format!("team/w3/del-{i}")).await;
    }

    assert!(del(&mut c, "tok-alice", "team/w3/del-1").await);
    assert_eq!(
        handles(&mut c, "tok-alice", 100).await,
        vec!["team/w3/del-0".to_string(), "team/w3/del-2".to_string()],
        "exactly the deleted branch is gone; its siblings survive"
    );

    // Idempotent: deleting it again is `false`, not an error.
    assert!(!del(&mut c, "tok-alice", "team/w3/del-1").await);

    // Bob cannot delete Alice's branch — and the branch is STILL THERE afterwards, which
    // is the assertion that would catch a delete that succeeded and mis-reported.
    assert!(!del(&mut c, "tok-bob", "team/w3/del-0").await);
    assert!(handles(&mut c, "tok-alice", 100)
        .await
        .contains(&"team/w3/del-0".to_string()));

    // A malformed handle is a harmless no-op, NOT an error. `DeleteBranch` deliberately
    // does not run the handle validator that `SnapshotInto` and `CreateBranch` run: those
    // WRITE the handle as a key, so a bad one must never be stored, while a delete matches
    // no row through parameterised SQL. Pinned here so the asymmetry reads as designed
    // rather than as an oversight someone later "fixes" into an error.
    assert!(!del(&mut c, "tok-alice", "not a valid handle!!").await);
    assert_eq!(handles(&mut c, "tok-alice", 100).await.len(), 2);
}

/// `SnapshotInto` reads confined host files into CAS, and refuses to leave the mount.
///
/// The escape arm is paired with an ACCEPTING control that differs by ONE variable — the
/// path — so a refusal that fired for any other reason (no root, bad handle, empty list)
/// cannot be mistaken for confinement working.
#[tokio::test]
async fn snapshot_into_ingests_confined_files_and_refuses_an_escape() {
    let root = tempfile::TempDir::new().unwrap();
    let secret = tempfile::TempDir::new().unwrap();
    std::fs::write(root.path().join("notes.md"), b"# w3\nconfined body\n").unwrap();
    std::fs::write(secret.path().join("outside.txt"), b"must not be readable").unwrap();
    // Read at HostBranchStore construction, so it must be set before `start`.
    std::env::set_var("KX_SERVE_FS_ROOT", root.path());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let handle = "team/w3/snap".to_string();
    let inside = root.path().join("notes.md").to_string_lossy().into_owned();

    // The ACCEPTING control, run FIRST: the same RPC, same handle, an in-root path.
    let ok = c
        .snapshot_into(with_bearer(
            proto::SnapshotIntoRequest {
                handle: handle.clone(),
                paths: vec![inside.clone()],
                description: "confined".into(),
                parent_handle: String::new(),
            },
            "tok-alice",
        ))
        .await
        .expect("an in-root path snapshots")
        .into_inner();
    assert_eq!(ok.ingested, 1);
    assert!(ok.items.iter().any(|i| i.path.ends_with("notes.md")));

    // Content IDENTITY, not just presence: the bytes in CAS are the bytes on disk.
    let body = c
        .get_branch_content(with_bearer(
            proto::GetBranchContentRequest {
                handle: handle.clone(),
                path: ok
                    .items
                    .iter()
                    .find(|i| i.path.ends_with("notes.md"))
                    .unwrap()
                    .path
                    .clone(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(body.found);
    assert_eq!(body.payload, b"# w3\nconfined body\n");

    // ONE variable changes: a path outside the mount.
    let escaped = c
        .snapshot_into(with_bearer(
            proto::SnapshotIntoRequest {
                handle: handle.clone(),
                paths: vec![secret
                    .path()
                    .join("outside.txt")
                    .to_string_lossy()
                    .into_owned()],
                description: "confined".into(),
                parent_handle: String::new(),
            },
            "tok-alice",
        ))
        .await;
    let err = escaped.expect_err("a path outside KX_SERVE_FS_ROOT is refused");
    // Assert the REASON, not merely that something failed — a negative arm passes on any
    // error, including one that means the feature is switched off.
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(
        err.message().contains("KX_SERVE_FS_ROOT"),
        "the refusal must name confinement, got: {}",
        err.message()
    );

    // And the escaped body never entered CAS under any path.
    let after = c
        .get_branch(with_bearer(
            proto::GetBranchRequest {
                handle: handle.clone(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(after
        .branch
        .unwrap()
        .items
        .iter()
        .all(|i| !i.path.contains("outside")));

    std::env::remove_var("KX_SERVE_FS_ROOT");
}

/// `ForgetMemory` removes one memory from BOTH the listing and recall.
///
/// Uses the client-vector path, so it needs no embedder. The sibling memory is the control:
/// a `forget` that wiped the namespace would pass an assertion that only checked the target
/// was gone.
#[cfg(feature = "hnsw")]
#[tokio::test]
async fn forget_memory_removes_it_from_both_the_listing_and_recall() {
    std::env::set_var("KX_SERVE_MEMORY", "1");
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let target = store_memory(&mut c, b"the roast tier is dark", vec![1.0, 0.0, 0.0, 0.0]).await;
    let keeper = store_memory(
        &mut c,
        b"the kettle is descaled monthly",
        vec![0.0, 1.0, 0.0, 0.0],
    )
    .await;
    assert_ne!(target, keeper, "two distinct memories");

    // PRECONDITION, asserted and not skipped: it is there, and it is recallable. Without
    // this, "it is gone afterwards" is also true of a memory that was never stored.
    assert!(
        list_memory_ids(&mut c).await.contains(&target),
        "stored and listed"
    );
    assert!(
        recall_ids(&mut c, vec![1.0, 0.0, 0.0, 0.0])
            .await
            .contains(&target),
        "recallable before the forget"
    );

    let forgotten = c
        .forget_memory(proto::ForgetMemoryRequest {
            memory_id: target.clone(),
            namespace: String::new(),
        })
        .await
        .expect("forget memory")
        .into_inner()
        .forgotten;
    assert!(forgotten);

    let after = list_memory_ids(&mut c).await;
    assert!(!after.contains(&target), "gone from the listing");
    assert!(after.contains(&keeper), "the sibling memory survives");
    assert!(
        !recall_ids(&mut c, vec![1.0, 0.0, 0.0, 0.0])
            .await
            .contains(&target),
        "gone from RECALL too — a forget that only hid it from the listing would still \
         feed it back into an agent's context, which is the whole point of forgetting"
    );

    // Idempotent: forgetting it twice is `false`, never an error.
    assert!(
        !c.forget_memory(proto::ForgetMemoryRequest {
            memory_id: target,
            namespace: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .forgotten
    );

    std::env::remove_var("KX_SERVE_MEMORY");
}

#[cfg(feature = "hnsw")]
async fn store_memory(
    c: &mut KxGatewayClient<Channel>,
    body: &[u8],
    embedding: Vec<f32>,
) -> Vec<u8> {
    c.store_memory(proto::StoreMemoryRequest {
        content: body.to_vec(),
        embedding,
        kind: proto::MemoryKind::Semantic as i32,
        namespace: String::new(),
    })
    .await
    .expect("store memory (client-vector path)")
    .into_inner()
    .memory_id
}

#[cfg(feature = "hnsw")]
async fn list_memory_ids(c: &mut KxGatewayClient<Channel>) -> Vec<Vec<u8>> {
    c.list_memories(proto::ListMemoriesRequest {
        limit: Some(50),
        instance_id: None,
        namespace: String::new(),
        include_tombstoned: false,
    })
    .await
    .unwrap()
    .into_inner()
    .memories
    .into_iter()
    .map(|m| m.memory_id)
    .collect()
}

#[cfg(feature = "hnsw")]
async fn recall_ids(c: &mut KxGatewayClient<Channel>, query: Vec<f32>) -> Vec<Vec<u8>> {
    c.recall_memory(proto::RecallMemoryRequest {
        query_text: String::new(),
        query_embedding: query,
        k: 5,
        namespace: String::new(),
    })
    .await
    .unwrap()
    .into_inner()
    .hits
    .into_iter()
    .map(|h| h.memory_id)
    .collect()
}
