//! Durable-Workflow catalog end-to-end over a REAL bound tonic port. Drives
//! `SaveWorkflow` / `ListWorkflows` / `GetWorkflow` / `DeleteWorkflow` and the
//! ADOPTED branch point-in-time history (`ListBranchVersions` /
//! `RestoreBranch` at the workflow handle) through the live gateway + the
//! `workflows.db` host store, proving both halves of each seam
//! deterministically (cross-component seam — never rely on the live model to
//! cover a cross-component seam):
//!
//! - **save → list → get round trip**: canonical bytes stored + read back
//!   byte-identically; `workflow_ref` server-derived; identical re-save
//!   dedups; a lifecycle FLIP on identical bytes is a real write.
//! - **definition history rides the entity-agnostic branch sidecar**: every
//!   non-dedup save appends a version; a restore re-syncs the catalog row so
//!   `GetWorkflow` serves the RESTORED definition (`workflow_resynced`).
//! - **cross-catalog handle refusal**: a handle that names an App is refused
//!   at `SaveWorkflow` (branches/locks/history are handle-keyed).
//! - **cross-party isolation** + **bad/App-tagged envelope ⇒ InvalidArgument**.
//! - **delete cascade**: row first, branch binding unbound, HISTORY retained
//!   (delete + restore is the recreate path) — with server-side coverage from
//!   day one (the DeleteApp zero-server-tests lesson).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::{Code, Request};

use kx_gateway::start;

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

/// A valid canonical `kortecx.workflow/v1` envelope authored via the kx-app
/// type crate — a two-step pure chain (no model needed anywhere in this file).
fn wf_envelope(name: &str) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "pure", "prompt": "" },
            { "kind": "pure", "prompt": "" }
        ],
        "edges": [ { "parent": 0, "child": 1, "data": true } ]
    });
    let mut env = kx_app::WorkflowEnvelope::new(name, blueprint);
    env.description = "demo workflow".to_string();
    env.to_canonical_json().unwrap()
}

fn save_req(handle: &str, envelope: Vec<u8>, lifecycle: &str) -> proto::SaveWorkflowRequest {
    proto::SaveWorkflowRequest {
        handle: handle.into(),
        envelope_json: envelope,
        source_digest: Vec::new(),
        lifecycle: lifecycle.into(),
    }
}

#[tokio::test]
async fn save_list_get_round_trip_dedup_and_lifecycle_flip() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let envelope = wf_envelope("triage");
    let saved = c
        .save_workflow(with_bearer(
            save_req("team/wf/triage", envelope.clone(), "draft"),
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!saved.deduplicated);
    assert_eq!(saved.handle, "team/wf/triage");
    assert_eq!(saved.workflow_ref.len(), 16, "16B server-derived id");

    // Identical bytes + identical lifecycle ⇒ dedup.
    let again = c
        .save_workflow(with_bearer(
            save_req("team/wf/triage", envelope.clone(), "draft"),
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(again.deduplicated);
    assert_eq!(again.workflow_ref, saved.workflow_ref);

    // A lifecycle FLIP on identical bytes (finishing the draft) is a REAL write.
    let finished = c
        .save_workflow(with_bearer(
            save_req("team/wf/triage", envelope.clone(), ""),
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!finished.deduplicated, "finishing a draft is not a no-op");

    // List surfaces the summary with the CURRENT lifecycle.
    let listed = c
        .list_workflows(with_bearer(
            proto::ListWorkflowsRequest {
                limit: 0,
                after_handle: String::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.workflows.len(), 1);
    let summary = &listed.workflows[0];
    assert_eq!(summary.handle, "team/wf/triage");
    assert_eq!(summary.name, "triage");
    assert_eq!(summary.step_count, 2);
    assert_eq!(summary.lifecycle, "");

    // Get returns the canonical bytes + the 32B handle-free digest.
    let got = c
        .get_workflow(with_bearer(
            proto::GetWorkflowRequest {
                handle: "team/wf/triage".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(got.found);
    assert_eq!(got.envelope_json, envelope);
    assert_eq!(got.workflow_digest.len(), 32);
    assert_eq!(got.summary.unwrap().lifecycle, "");
}

#[tokio::test]
async fn definition_history_records_saves_and_restore_resyncs_the_catalog() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let v1 = wf_envelope("triage-v1");
    let v2 = wf_envelope("triage-v2");
    for env in [&v1, &v2] {
        c.save_workflow(with_bearer(
            save_req("team/wf/hist", env.clone(), ""),
            "tok-alice",
        ))
        .await
        .unwrap();
    }
    // Both definitions were recorded as branch versions at the WORKFLOW handle
    // (create-baseline + two advances ⇒ at least two versions; the sidecar is
    // entity-agnostic, so no workflow-specific history RPC exists or is needed).
    let versions = c
        .list_branch_versions(with_bearer(
            proto::ListBranchVersionsRequest {
                handle: "team/wf/hist".into(),
                limit: 0,
                after_version: 0,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(versions.found, "the definition branch has history");
    assert!(
        versions.versions.len() >= 2,
        "every non-dedup save records a version, got {}",
        versions.versions.len()
    );

    // The CURRENT definition is v2. Restore to the OLDEST recorded version
    // whose manifest carries v1 — then GetWorkflow must serve v1 (the resync).
    let oldest = versions.versions.last().unwrap().version;
    // Find the version whose restore yields the v1 definition: walk from oldest
    // upward, restoring and checking — the FIRST restore that changes the
    // catalog row proves the mechanism; asserting on the exact v1 bytes proves
    // the CONTENT (not just that something happened).
    let mut resynced_to_v1 = false;
    for candidate in (oldest..=versions.versions.first().unwrap().version).rev() {
        let restored = c
            .restore_branch(with_bearer(
                proto::RestoreBranchRequest {
                    handle: "team/wf/hist".into(),
                    version: candidate,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        if restored.deduplicated {
            continue;
        }
        assert!(
            restored.workflow_resynced,
            "a non-dedup restore at a workflow handle must resync the catalog row"
        );
        let got = c
            .get_workflow(with_bearer(
                proto::GetWorkflowRequest {
                    handle: "team/wf/hist".into(),
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(got.found);
        if got.envelope_json == v1 {
            resynced_to_v1 = true;
            break;
        }
    }
    assert!(
        resynced_to_v1,
        "restoring a recorded version must bring the v1 definition back \
         through GetWorkflow (the catalog follows the history)"
    );
}

#[tokio::test]
async fn a_handle_naming_an_app_is_refused_and_vice_versa_shapes() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Save an APP at the handle.
    let app_env = {
        let env = kx_app::AppEnvelope::new("occupant", serde_json::json!({ "steps": [] }));
        env.to_canonical_json().unwrap()
    };
    c.save_app(with_bearer(
        proto::SaveAppRequest {
            handle: "team/shared/handle".into(),
            envelope_json: app_env.clone(),
            source_digest: Vec::new(),
        },
        "tok-alice",
    ))
    .await
    .unwrap();

    // A WORKFLOW at the same handle is refused: branches, locks and history are
    // (principal, handle)-keyed with no entity axis — sharing would silently
    // share all three.
    let err = c
        .save_workflow(with_bearer(
            save_req("team/shared/handle", wf_envelope("squatter"), ""),
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(
        err.message().contains("names an App"),
        "the refusal names the cause, got: {}",
        err.message()
    );

    // An App-TAGGED envelope at the workflow seam is refused (mutual exclusion),
    // and a workflow-tagged envelope at the App seam likewise.
    let err = c
        .save_workflow(with_bearer(
            save_req("team/wf/apptag", app_env, ""),
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    let err = c
        .save_app(with_bearer(
            proto::SaveAppRequest {
                handle: "team/apps/wftag".into(),
                envelope_json: wf_envelope("wf"),
                source_digest: Vec::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    // Lifecycle vocabulary is closed.
    let err = c
        .save_workflow(with_bearer(
            save_req("team/wf/badlife", wf_envelope("x"), "archived"),
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("lifecycle"));

    // Not JSON at all.
    let err = c
        .save_workflow(with_bearer(
            save_req("team/wf/bad", b"{not json".to_vec(), ""),
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn cross_party_isolation_is_uniform_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.save_workflow(with_bearer(
        save_req("team/wf/secret", wf_envelope("secret"), ""),
        "tok-alice",
    ))
    .await
    .unwrap();

    // Bob cannot see Alice's workflow (uniform not-found / empty list).
    let got = c
        .get_workflow(with_bearer(
            proto::GetWorkflowRequest {
                handle: "team/wf/secret".into(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!got.found);
    let listed = c
        .list_workflows(with_bearer(
            proto::ListWorkflowsRequest {
                limit: 0,
                after_handle: String::new(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(listed.workflows.is_empty());

    // Bob deleting Alice's workflow: uniform removed=false, and Alice's row
    // survives untouched.
    let del = c
        .delete_workflow(with_bearer(
            proto::DeleteWorkflowRequest {
                handle: "team/wf/secret".into(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!del.removed);
    let got = c
        .get_workflow(with_bearer(
            proto::GetWorkflowRequest {
                handle: "team/wf/secret".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(got.found, "the owner's row survives a foreign delete");
}

#[tokio::test]
async fn delete_cascades_row_first_and_history_survives_for_recreate() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.save_workflow(with_bearer(
        save_req("team/wf/gone", wf_envelope("gone"), ""),
        "tok-alice",
    ))
    .await
    .unwrap();

    let del = c
        .delete_workflow(with_bearer(
            proto::DeleteWorkflowRequest {
                handle: "team/wf/gone".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(del.removed);
    assert!(
        del.branch_unbound,
        "the definition branch BINDING is dropped by the cascade"
    );
    assert_eq!(
        del.triggers_removed, 0,
        "no workflow triggers can exist yet"
    );

    // The row is gone…
    let got = c
        .get_workflow(with_bearer(
            proto::GetWorkflowRequest {
                handle: "team/wf/gone".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!got.found);
    // …but the definition HISTORY survives the delete (D-branch posture:
    // delete + restore is the "recreate without losing state" path).
    let versions = c
        .list_branch_versions(with_bearer(
            proto::ListBranchVersionsRequest {
                handle: "team/wf/gone".into(),
                limit: 0,
                after_version: 0,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        versions.found && !versions.versions.is_empty(),
        "history survives the delete so the definition is recoverable"
    );

    // A second delete is a uniform no-op.
    let del2 = c
        .delete_workflow(with_bearer(
            proto::DeleteWorkflowRequest {
                handle: "team/wf/gone".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!del2.removed);
}

/// Poll `GetProjection` until `mote_id` is `Committed` (the terminal-anchor
/// discipline: assert on THE mote the RunHandle named, never on counts).
async fn await_mote_committed(
    c: &mut KxGatewayClient<Channel>,
    token: &str,
    instance_id: &[u8],
    mote_id: &[u8],
) {
    for _ in 0..200 {
        let view = c
            .get_projection(with_bearer(
                proto::GetProjectionRequest {
                    instance_id: instance_id.to_vec(),
                    at_seq: None,
                },
                token,
            ))
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            assert!(
                m.result_ref.is_some(),
                "a committed terminal carries a result_ref"
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the workflow's terminal mote never reached Committed");
}

#[tokio::test]
async fn run_workflow_runs_a_pure_chain_to_its_terminal_anchor() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.save_workflow(with_bearer(
        save_req("team/wf/chain", wf_envelope("chain"), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/chain".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    // The D239 anchor contract: terminal_mote_id populated for EVERY shape;
    // react_chain_salt only for the exactly-one-agentic-step shape (this pure
    // chain has none, so the salt is EMPTY by construction).
    assert_eq!(run.instance_id.len(), 16);
    assert_eq!(
        run.terminal_mote_id.len(),
        32,
        "the run anchor is populated"
    );
    assert!(
        run.react_chain_salt.is_empty(),
        "a pure DAG has no agentic chain key"
    );
    // The run settles: THE terminal the handle named commits (never a count).
    await_mote_committed(&mut c, "tok-alice", &run.instance_id, &run.terminal_mote_id).await;

    // A draft is RUNNABLE (lifecycle is advisory, never run enforcement) — the
    // refusal surface for drafts is trigger REGISTRATION, not the run path.
    c.save_workflow(with_bearer(
        save_req("team/wf/draftrun", wf_envelope("draftrun"), "draft"),
        "tok-alice",
    ))
    .await
    .unwrap();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/draftrun".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(run.terminal_mote_id.len(), 32);
}

#[tokio::test]
async fn run_workflow_error_shapes_are_honest() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Absent handle (and not-owned alike): uniform permission_denied — the
    // NotAuthorized posture, no existence oracle.
    let err = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/ghost".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    // An unserved model route REFUSES at submit — never silently degrades to a
    // different model (the wish ∩ served-catalog contract).
    let mut env = kx_app::WorkflowEnvelope::new(
        "routed",
        serde_json::json!({
            "seed": 0,
            "steps": [ { "kind": "model", "prompt": "hello" } ]
        }),
    );
    env.steering_config.model.model_route = "no-such-model".into();
    c.save_workflow(with_bearer(
        save_req("team/wf/routed", env.to_canonical_json().unwrap(), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let err = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/routed".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(
        err.message().contains("model route"),
        "names the route problem, got: {}",
        err.message()
    );

    // A step composing an UNDECLARED app is refused with the workflow-worded
    // composition error (the one axis that never degrades gracefully — a
    // silently-dropped sub-graph would look like the workflow working).
    let undeclared = kx_app::WorkflowEnvelope::new(
        "composer",
        serde_json::json!({
            "seed": 0,
            "steps": [ { "kind": "model", "prompt": "use it", "apps": ["team/apps/ghost"] } ]
        }),
    );
    c.save_workflow(with_bearer(
        save_req(
            "team/wf/composer",
            undeclared.to_canonical_json().unwrap(),
            "",
        ),
        "tok-alice",
    ))
    .await
    .unwrap();
    let err = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/composer".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(
        err.message().contains("workflow step composes app"),
        "the refusal speaks workflow, got: {}",
        err.message()
    );
}

/// A workflow whose terminal is a DURABLE WAIT: `pure → wait(delay_ms)`. The
/// wait is a pure step carrying the identity-bearing delay key — the
/// coordinator parks it, journals a `TimerArmed` once the parent commits, and
/// fires by synthesizing the wait's own `Committed` (pass-through of the
/// parent's result).
fn wait_envelope(name: &str, delay_ms: u64) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "pure", "prompt": "" },
            { "kind": "pure", "params": { "kx.wait.delay_ms": delay_ms.to_string() } }
        ],
        "edges": [ { "parent": 0, "child": 1, "data": true } ]
    });
    kx_app::WorkflowEnvelope::new(name, blueprint)
        .to_canonical_json()
        .unwrap()
}

/// Poll until the mote is committed; return the instant it was FIRST observed
/// committed (None on timeout).
async fn first_committed_at(
    c: &mut KxGatewayClient<Channel>,
    token: &str,
    instance_id: &[u8],
    mote_id: &[u8],
    timeout_ms: u64,
) -> Option<std::time::Instant> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        let view = c
            .get_projection(with_bearer(
                proto::GetProjectionRequest {
                    instance_id: instance_id.to_vec(),
                    at_seq: None,
                },
                token,
            ))
            .await
            .unwrap()
            .into_inner();
        if view
            .motes
            .iter()
            .any(|m| m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            return Some(std::time::Instant::now());
        }
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }
    None
}

#[tokio::test]
async fn a_durable_wait_holds_the_run_then_fires_once() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.save_workflow(with_bearer(
        save_req("team/wf/hold", wait_envelope("hold", 1500), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let started = std::time::Instant::now();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/hold".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    // The wait HOLDS: the terminal must not commit before the delay elapses.
    assert!(
        first_committed_at(
            &mut c,
            "tok-alice",
            &run.instance_id,
            &run.terminal_mote_id,
            600
        )
        .await
        .is_none(),
        "the wait committed before its delay — it did not hold"
    );
    // …and FIRES: committed within a generous settle budget, after ≥ delay.
    let fired = first_committed_at(
        &mut c,
        "tok-alice",
        &run.instance_id,
        &run.terminal_mote_id,
        15_000,
    )
    .await
    .expect("the wait must fire after its delay");
    let held = fired.duration_since(started);
    assert!(
        held >= std::time::Duration::from_millis(1500),
        "fired after {held:?} — before the declared delay"
    );
}

#[tokio::test]
async fn a_restart_re_arms_the_journaled_timer_and_never_re_fires() {
    let dir = tempfile::TempDir::new().unwrap();
    let delay_ms: u64 = 4_000;

    // Serve A: save + run; the timer arms; the serve dies BEFORE it fires.
    let started;
    let (instance_id, terminal_mote_id);
    {
        let running = start(common::gateway_config(&dir, false, two_party_tokens()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        c.save_workflow(with_bearer(
            save_req("team/wf/survive", wait_envelope("survive", delay_ms), ""),
            "tok-alice",
        ))
        .await
        .unwrap();
        started = std::time::Instant::now();
        let run = c
            .run_workflow(with_bearer(
                proto::RunWorkflowRequest {
                    handle: "team/wf/survive".into(),
                    args: Vec::new(),
                    require_approval: false,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        instance_id = run.instance_id.clone();
        terminal_mote_id = run.terminal_mote_id.clone();
        // Give the arm a moment (the parent pure step commits + the settle pass
        // journals TimerArmed), and pin that the wait has NOT fired.
        assert!(
            first_committed_at(&mut c, "tok-alice", &instance_id, &terminal_mote_id, 900)
                .await
                .is_none(),
            "the wait fired before the kill — the restart proof is vacuous"
        );
        running.shutdown().await.unwrap();
    }

    // Serve B, SAME state dir (that reuse IS the proof): the journal carries the
    // armed timer; the run rehydrates through the idempotent re-submit (the same
    // RunWorkflow yields byte-identical MoteIds — identical re-invoke is a replay);
    // the settle pass re-arms IN MEMORY from the folded fact at the JOURNALED
    // instant and appends nothing.
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let rerun = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/survive".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        rerun.terminal_mote_id, terminal_mote_id,
        "the re-submit resolves to the SAME run (byte-identical MoteIds)"
    );
    let fired = first_committed_at(
        &mut c,
        "tok-alice",
        &rerun.instance_id,
        &terminal_mote_id,
        20_000,
    )
    .await
    .expect("the re-armed timer must fire after the restart");
    let held = fired.duration_since(started);
    // Fires at (roughly) the ORIGINAL journaled instant: at least the declared
    // delay from the FIRST run's start…
    assert!(
        held >= std::time::Duration::from_millis(delay_ms),
        "fired after only {held:?} — before the journaled instant"
    );
    // …and well under a DOUBLE hold: a re-arm that restarted the clock (or a
    // second fire re-holding) would land past 2×delay. The margin below 2×
    // covers serve boot + polling slack while still refuting a re-hold.
    assert!(
        held < std::time::Duration::from_millis(2 * delay_ms),
        "fired after {held:?} — the restart re-held the delay instead of \
         re-arming at the journaled instant"
    );
}

/// A minimal local HTTP fixture: one background thread, answers every request
/// with a fixed JSON body carrying a fixture-only token, and records whether an
/// `authorization` header arrived. Plain std — hermetic, no model, no weather.
fn spawn_http_fixture(expect_bearer: Option<&'static str>) -> (u16, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        // Serve a bounded number of requests then exit (test-scoped).
        for stream in listener.incoming().take(4) {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let authed = match expect_bearer {
                None => true,
                Some(token) => req
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {token}").to_ascii_lowercase()),
            };
            let body = if authed {
                r#"{"vessel":"kestrel","officer":"FIXTURE-TOKEN-77"}"#
            } else {
                r#"{"error":"missing bearer"}"#
            };
            let status = if authed { "200 OK" } else { "401 Unauthorized" };
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    (port, handle)
}

#[tokio::test]
async fn an_http_step_dials_with_its_named_secret_and_carries_the_answer() {
    // The credential the step names — resolved by NAME at dispatch through the
    // env arm of the secret chain; the VALUE never appears in the envelope.
    std::env::set_var("KX_E2E_HTTP_TOKEN", "fixture-secret-42");
    let (port, _fixture) = spawn_http_fixture(Some("fixture-secret-42"));
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // http (credentialed fetch) → pure (the carry) — the sequential-carry spine.
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "http", "args": {
                "url": format!("http://127.0.0.1:{port}/record"),
                "secret_name": "KX_E2E_HTTP_TOKEN"
            } },
            { "kind": "pure", "prompt": "" }
        ],
        "edges": [ { "parent": 0, "child": 1, "data": true } ]
    });
    let env = kx_app::WorkflowEnvelope::new("dial", blueprint);
    c.save_workflow(with_bearer(
        save_req("team/wf/dial", env.to_canonical_json().unwrap(), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/dial".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    await_mote_committed(&mut c, "tok-alice", &run.instance_id, &run.terminal_mote_id).await;

    // The FIXTURE-ONLY token reached the run: read the http observation via the
    // projection's ancestor set (the terminal's parent is the http step) and
    // fetch its committed content — the oracle is underivable without the dial
    // AND the credential (the fixture 401s a bearer-less request).
    let view = c
        .get_projection(with_bearer(
            proto::GetProjectionRequest {
                instance_id: run.instance_id.clone(),
                at_seq: None,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    let mut carried = false;
    for m in &view.motes {
        if m.state != proto::MoteSnapshotState::Committed as i32 {
            continue;
        }
        let Some(r) = m.result_ref.clone() else {
            continue;
        };
        let content = c
            .get_content(with_bearer(
                proto::GetContentRequest {
                    instance_id: run.instance_id.clone(),
                    content_ref: r,
                },
                "tok-alice",
            ))
            .await;
        if let Ok(resp) = content {
            let body = String::from_utf8_lossy(&resp.into_inner().payload).to_string();
            if body.contains("FIXTURE-TOKEN-77") {
                assert!(
                    body.contains("\"status\":200"),
                    "an ANSWERED dial commits its status: {body}"
                );
                carried = true;
                break;
            }
        }
    }
    assert!(
        carried,
        "the http observation carrying the fixture-only token must be committed in this run"
    );
}

#[tokio::test]
async fn an_http_step_naming_a_private_host_by_name_is_refused() {
    // The SSRF A/B: `127.0.0.1` is a LITERAL the operator declared (allowed —
    // the previous test); `localhost` is a NAME that resolves to a private
    // address, which the egress kernel refuses (rebind defense) — the dial
    // never happens and the step fails honestly rather than reach inside.
    let (port, _fixture) = spawn_http_fixture(None);
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "http", "args": { "url": format!("http://localhost:{port}/record") } }
        ]
    });
    let env = kx_app::WorkflowEnvelope::new("ssrf", blueprint);
    c.save_workflow(with_bearer(
        save_req("team/wf/ssrf", env.to_canonical_json().unwrap(), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/ssrf".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    // The step must FAIL (dead-letter), never commit: poll for a terminal
    // Failed state on the http mote and pin that it never commits.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut failed = false;
    while std::time::Instant::now() < deadline {
        let view = c
            .get_projection(with_bearer(
                proto::GetProjectionRequest {
                    instance_id: run.instance_id.clone(),
                    at_seq: None,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        if view.motes.iter().any(|m| {
            m.mote_id == run.terminal_mote_id
                && m.state == proto::MoteSnapshotState::Committed as i32
        }) {
            panic!("a name-to-private dial must never commit");
        }
        if view
            .motes
            .iter()
            .any(|m| m.state == proto::MoteSnapshotState::Failed as i32)
        {
            failed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        failed,
        "the refused dial must surface as an honest Failed state"
    );
}

/// A PATH-ROUTED http fixture for the conditional pair: pressure readings and
/// gate orders, each answer derivable only by dialing its route.
fn spawn_routed_fixture() -> (u16, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let first_key: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        for stream in listener.incoming().take(16) {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let (status, body) = if req.starts_with("GET /high") {
                ("200 OK", r#"{"reading":87}"#)
            } else if req.starts_with("GET /low") {
                ("200 OK", r#"{"reading":12}"#)
            } else if req.starts_with("GET /open") {
                ("200 OK", r#"{"order":"SLUICE-OPEN-9"}"#)
            } else if req.starts_with("GET /shut") {
                ("200 OK", r#"{"order":"SLUICE-SHUT-2"}"#)
            } else if req.starts_with("POST /flaky") {
                // IDENTITY-keyed flake: every dial carrying the FIRST-seen
                // Idempotency-Key is refused forever (so the worker's
                // same-identity at-least-once redispatch can never succeed);
                // any OTHER key answers. The token is therefore underivable
                // unless the retry ladder minted a FRESH attempt identity —
                // the fresh-token-fresh-attempt claim, proven by the fixture.
                let key = req
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("idempotency-key:"))
                    .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
                    .unwrap_or_default();
                let mut first = first_key.lock().unwrap();
                if first.is_none() {
                    *first = Some(key.clone());
                }
                if first.as_deref() == Some(key.as_str()) {
                    (
                        "503 Service Unavailable",
                        r#"{"error":"depot refuses this key"}"#,
                    )
                } else {
                    ("200 OK", r#"{"code":"EMBER-RELAY-19"}"#)
                }
            } else if req.starts_with("GET /dead") {
                ("503 Service Unavailable", r#"{"error":"permanently down"}"#)
            } else {
                ("200 OK", r#"{"error":"no such route"}"#)
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    (port, handle)
}

/// The sluice workflow: http(source) → conditional(":87"?) →Control→ two http
/// arms (open/shut, skip-guarded) → first_non_skip join. The PAIR is the
/// oracle: a predicate that never reads its parent emits the same token on
/// both runs and fails exactly one of them; a conditional that ran BOTH arms
/// fails the join outright (two non-skip parents).
fn sluice_envelope(name: &str, port: u16, source_path: &str) -> Vec<u8> {
    let base = format!("http://127.0.0.1:{port}");
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "http", "args": { "url": format!("{base}/{source_path}") } },
            { "kind": "conditional", "params": {
                "kx.cond.predicate": "{\"op\":\"contains\",\"value\":\":87\"}"
            } },
            { "kind": "http", "args": { "url": format!("{base}/open") },
              "params": { "kx.cond.skip_guard": "true", "kx.cond.arm": "then" } },
            { "kind": "http", "args": { "url": format!("{base}/shut") },
              "params": { "kx.cond.skip_guard": "true", "kx.cond.arm": "else" } },
            { "kind": "pure", "params": { "kx.cond.join": "first_non_skip" } }
        ],
        "edges": [
            { "parent": 0, "child": 1 },
            { "parent": 1, "child": 2, "edge": "control" },
            { "parent": 1, "child": 3, "edge": "control" },
            { "parent": 2, "child": 4 },
            { "parent": 3, "child": 4 }
        ]
    });
    kx_app::WorkflowEnvelope::new(name, blueprint)
        .to_canonical_json()
        .unwrap()
}

#[tokio::test]
async fn a_conditional_takes_the_right_arm_in_both_directions() {
    let (port, _fixture) = spawn_routed_fixture();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    for (handle, source, must, must_not) in [
        (
            "team/wf/sluice-high",
            "high",
            "SLUICE-OPEN-9",
            "SLUICE-SHUT-2",
        ),
        (
            "team/wf/sluice-low",
            "low",
            "SLUICE-SHUT-2",
            "SLUICE-OPEN-9",
        ),
    ] {
        c.save_workflow(with_bearer(
            save_req(handle, sluice_envelope(source, port, source), ""),
            "tok-alice",
        ))
        .await
        .unwrap();
        let run = c
            .run_workflow(with_bearer(
                proto::RunWorkflowRequest {
                    handle: handle.into(),
                    args: Vec::new(),
                    require_approval: false,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        await_mote_committed(&mut c, "tok-alice", &run.instance_id, &run.terminal_mote_id).await;
        // Read THE terminal's committed bytes (the join's verbatim carry of the
        // surviving arm's http observation).
        let view = c
            .get_projection(with_bearer(
                proto::GetProjectionRequest {
                    instance_id: run.instance_id.clone(),
                    at_seq: None,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        let terminal = view
            .motes
            .iter()
            .find(|m| m.mote_id == run.terminal_mote_id)
            .expect("terminal in projection");
        let r = terminal.result_ref.clone().expect("committed result");
        let body = String::from_utf8_lossy(
            &c.get_content(with_bearer(
                proto::GetContentRequest {
                    instance_id: run.instance_id.clone(),
                    content_ref: r,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner()
            .payload,
        )
        .into_owned();
        assert!(
            body.contains(must),
            "{source}: the join must carry the TAKEN arm's token, got: {body}"
        );
        assert!(
            !body.contains(must_not),
            "{source}: the UNTAKEN arm's token leaked into the join: {body}"
        );
    }
}

#[tokio::test]
async fn a_retry_policy_re_attempts_a_flaky_step_with_a_fresh_identity() {
    let (port, _fixture) = spawn_routed_fixture();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ONE http POST under retry policy — the terminal IS the policied launch,
    // whose commit the coordinator synthesizes from the first landed attempt.
    // The fixture refuses EVERY dial carrying the first-seen Idempotency-Key,
    // so the worker's same-identity at-least-once redispatch can never
    // succeed: the token is underivable unless the ladder minted a FRESH
    // attempt identity (fresh idempotency token — the design's core claim),
    // after the DURABLE backoff.
    let started = std::time::Instant::now();
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "http",
              "args": { "url": format!("http://127.0.0.1:{port}/flaky"),
                        "method": "POST", "body": "{}" },
              "params": {
                  "kx.step.failure_mode": "retry",
                  "kx.step.retry_max": "3",
                  "kx.step.retry_backoff_ms": "500"
              } }
        ]
    });
    let env = kx_app::WorkflowEnvelope::new("flaky", blueprint);
    c.save_workflow(with_bearer(
        save_req("team/wf/flaky", env.to_canonical_json().unwrap(), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/flaky".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    await_mote_committed(&mut c, "tok-alice", &run.instance_id, &run.terminal_mote_id).await;
    let view = c
        .get_projection(with_bearer(
            proto::GetProjectionRequest {
                instance_id: run.instance_id.clone(),
                at_seq: None,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    let terminal = view
        .motes
        .iter()
        .find(|m| m.mote_id == run.terminal_mote_id)
        .unwrap();
    let body = String::from_utf8_lossy(
        &c.get_content(with_bearer(
            proto::GetContentRequest {
                instance_id: run.instance_id.clone(),
                content_ref: terminal.result_ref.clone().unwrap(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner()
        .payload,
    )
    .into_owned();
    assert!(
        body.contains("EMBER-RELAY-19"),
        "the launch must commit the FRESH attempt's answer, got: {body}"
    );
    // The failed first attempt is an honest fact in the SAME run's journal.
    assert!(
        view.motes
            .iter()
            .any(|m| m.state == proto::MoteSnapshotState::Failed as i32),
        "attempt 1's terminal failure must be visible, never papered over"
    );
    // The DURABLE backoff actually held between the attempts.
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(500),
        "the retry must wait its backoff, elapsed {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_continue_policy_commits_the_failure_as_a_fact_and_the_quorum_proceeds() {
    let (port, _fixture) = spawn_routed_fixture();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // A parallel pair — one healthy dial, one PERMANENTLY dead endpoint under
    // continue policy — joined by a 1-of-2 quorum. The run completes; the
    // aggregate carries the survivor and says honestly that one of two made it.
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [
            { "kind": "http", "args": { "url": format!("http://127.0.0.1:{port}/open") } },
            { "kind": "http",
              "args": { "url": format!("http://127.0.0.1:{port}/dead") },
              "params": {
                  "kx.step.failure_mode": "continue",
                  "kx.step.retry_max": "1"
              } },
            { "kind": "pure", "params": { "kx.join.quorum": "1" } }
        ],
        "edges": [
            { "parent": 0, "child": 2 },
            { "parent": 1, "child": 2 }
        ]
    });
    let env = kx_app::WorkflowEnvelope::new("quorum", blueprint);
    c.save_workflow(with_bearer(
        save_req("team/wf/quorum", env.to_canonical_json().unwrap(), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    let run = c
        .run_workflow(with_bearer(
            proto::RunWorkflowRequest {
                handle: "team/wf/quorum".into(),
                args: Vec::new(),
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    await_mote_committed(&mut c, "tok-alice", &run.instance_id, &run.terminal_mote_id).await;
    let view = c
        .get_projection(with_bearer(
            proto::GetProjectionRequest {
                instance_id: run.instance_id.clone(),
                at_seq: None,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    let terminal = view
        .motes
        .iter()
        .find(|m| m.mote_id == run.terminal_mote_id)
        .unwrap();
    let body = String::from_utf8_lossy(
        &c.get_content(with_bearer(
            proto::GetContentRequest {
                instance_id: run.instance_id.clone(),
                content_ref: terminal.result_ref.clone().unwrap(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner()
        .payload,
    )
    .into_owned();
    assert!(
        body.contains("SLUICE-OPEN-9"),
        "the quorum aggregate carries the survivor, got: {body}"
    );
    assert!(
        body.contains("\"survivors\":1") && body.contains("\"of\":2"),
        "the aggregate says honestly that one of two made it, got: {body}"
    );
}

#[tokio::test]
async fn a_workflow_trigger_registers_fires_and_cascades_on_delete() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // A DRAFT workflow is refused at REGISTRATION (never a forever-dead-letter).
    c.save_workflow(with_bearer(
        save_req("team/wf/draft-t", wf_envelope("draft-t"), "draft"),
        "tok-alice",
    ))
    .await
    .unwrap();
    let err = c
        .register_trigger(with_bearer(
            proto::RegisterTriggerRequest {
                name: "draft-trigger".into(),
                kind: proto::TriggerKind::Grpc as i32,
                recipe_handle: String::new(),
                app_handle: String::new(),
                workflow_handle: "team/wf/draft-t".into(),
                auth: proto::TriggerAuth::None as i32,
                auth_secret_ref: String::new(),
                schedule_spec: String::new(),
                timezone: String::new(),
                enabled: true,
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert!(
        err.message().contains("draft"),
        "the refusal names the draft, got: {}",
        err.message()
    );

    // An UN-SAVED workflow is refused at registration too.
    let err = c
        .register_trigger(with_bearer(
            proto::RegisterTriggerRequest {
                name: "ghost-trigger".into(),
                kind: proto::TriggerKind::Grpc as i32,
                recipe_handle: String::new(),
                app_handle: String::new(),
                workflow_handle: "team/wf/ghost".into(),
                auth: proto::TriggerAuth::None as i32,
                auth_secret_ref: String::new(),
                schedule_spec: String::new(),
                timezone: String::new(),
                enabled: true,
                require_approval: false,
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert!(
        err.message().contains("not in your catalog"),
        "got: {}",
        err.message()
    );

    // A saved (active) workflow registers, and a gRPC SubmitTrigger fires it
    // through the SAME RunWorkflow resolver — the run settles to Committed.
    c.save_workflow(with_bearer(
        save_req("team/wf/fired", wf_envelope("fired"), ""),
        "tok-alice",
    ))
    .await
    .unwrap();
    c.register_trigger(with_bearer(
        proto::RegisterTriggerRequest {
            name: "wf-trigger".into(),
            kind: proto::TriggerKind::Grpc as i32,
            recipe_handle: String::new(),
            app_handle: String::new(),
            workflow_handle: "team/wf/fired".into(),
            auth: proto::TriggerAuth::None as i32,
            auth_secret_ref: String::new(),
            schedule_spec: String::new(),
            timezone: String::new(),
            enabled: true,
            require_approval: false,
        },
        "tok-alice",
    ))
    .await
    .expect("an active workflow is schedulable");
    let fired = c
        .submit_trigger(with_bearer(
            proto::SubmitTriggerRequest {
                name: "wf-trigger".into(),
                idempotency_key: "evt-1".into(),
                payload_json: String::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fired.instance_id.len(), 16);
    // The run settles (find ANY committed terminal-looking progress: poll the
    // projection until at least the two chain motes commit).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut committed = 0;
    while std::time::Instant::now() < deadline {
        let view = c
            .get_projection(with_bearer(
                proto::GetProjectionRequest {
                    instance_id: fired.instance_id.clone(),
                    at_seq: None,
                },
                "tok-alice",
            ))
            .await
            .unwrap()
            .into_inner();
        committed = view
            .motes
            .iter()
            .filter(|m| m.state == proto::MoteSnapshotState::Committed as i32)
            .count();
        if committed >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }
    assert!(committed >= 2, "the fired workflow run must settle");
    // A replayed event dedups onto the same run.
    let replay = c
        .submit_trigger(with_bearer(
            proto::SubmitTriggerRequest {
                name: "wf-trigger".into(),
                idempotency_key: "evt-1".into(),
                payload_json: String::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(replay.deduped);
    assert_eq!(replay.instance_id, fired.instance_id);

    // The governance row carries the workflow target.
    let listed = c
        .list_triggers(with_bearer(
            proto::ListTriggersRequest {
                limit: 0,
                after_name: String::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    let row = listed
        .triggers
        .iter()
        .find(|t| t.name == "wf-trigger")
        .expect("the trigger lists");
    assert_eq!(row.workflow_handle, "team/wf/fired");
    assert!(row.disabled_reason.is_empty());
    assert_eq!(row.consecutive_failures, 0);

    // DeleteWorkflow cascades the trigger (the no-FK orphan hazard, closed).
    let del = c
        .delete_workflow(with_bearer(
            proto::DeleteWorkflowRequest {
                handle: "team/wf/fired".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(del.removed);
    assert_eq!(del.triggers_removed, 1, "the workflow trigger is cascaded");
    let listed = c
        .list_triggers(with_bearer(
            proto::ListTriggersRequest {
                limit: 0,
                after_name: String::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !listed.triggers.iter().any(|t| t.name == "wf-trigger"),
        "the cascaded trigger is gone"
    );
}

#[tokio::test]
async fn oversized_envelope_is_refused_at_the_boundary() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // 1 MiB + 1 of bytes — refused BEFORE any parse/host touch.
    let oversized = vec![b'x'; (1 << 20) + 1];
    let err = c
        .save_workflow(with_bearer(
            save_req("team/wf/big", oversized, ""),
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("cap"));
}
