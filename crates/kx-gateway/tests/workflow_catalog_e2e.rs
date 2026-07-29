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
    assert_eq!(del.triggers_removed, 0, "no workflow triggers can exist yet");

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
        if let Some(m) = view.motes.iter().find(|m| {
            m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32
        }) {
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
    assert_eq!(run.terminal_mote_id.len(), 32, "the run anchor is populated");
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
        if view.motes.iter().any(|m| {
            m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32
        }) {
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
        first_committed_at(&mut c, "tok-alice", &run.instance_id, &run.terminal_mote_id, 600)
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
    // RunWorkflow yields byte-identical MoteIds — the L-207 replay semantics);
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
