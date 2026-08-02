//! A trigger is validated before it is armed, and firing the same event twice starts ONE
//! run.
//!
//! ## The scenario, named before the code (Rule 47)
//!
//! An operator wires an inbound event to work the runtime should do. Before arming it
//! they want to know it will bind — without setting anything off, because a dry run that
//! fires is worse than no dry run at all. Once it is armed, the upstream system will
//! eventually deliver the same event twice (every webhook source does), and that must
//! produce ONE run, not two.
//!
//! ## What was actually uncovered, and why the existing dedup test does not cover it
//!
//! `TestTrigger` had **no test caller anywhere** — the only non-generated references were
//! the handler, the CLI verb, and a header comment in a sibling test file claiming
//! coverage the file does not have.
//!
//! Dedup looks covered: `workflow_catalog_e2e` fires the same key twice and asserts
//! `replay.deduped` with the same returned `instance_id`. Both are properties of the
//! RESPONSE, and the response is upheld by a mechanism other than the one a reader would
//! assume — see the note below, which is the finding this file produced.
//!
//! ## The witness, and the one this file tried first
//!
//! The obvious counter is `ListRuns`, and it is the WRONG instrument here — it was used
//! for three revisions of this file before the measurement showed why. A coordinator
//! hosts exactly one run: `register_run` requires the registration to be journal seq 1
//! and returns the EXISTING identity for every later call, fingerprint ignored. So in an
//! embedded serve every trigger fire after the first returns the first run's id, and
//! `ListRuns` reports one row no matter how many events arrive. "No second run appeared"
//! is then true unconditionally — a pass that cannot fail for the reason it is being
//! asked, which is this project's dominant defect class.
//!
//! What actually separates a fire from a replay is the MOTES. A fire binds its target and
//! submits one Mote per step; a deduplicated delivery returns before binding and submits
//! none. So the arms below count admitted Motes through `GetProjection`, and the one arm
//! that legitimately uses `ListRuns` uses it for the question it can answer: whether a
//! dry run registered a run AT ALL.
//!
//! ## ⚠ WHAT THIS FILE DOES NOT GUARD, established by mutation
//!
//! `submit` deduplicates TWICE. A pre-check reads the fire record and returns before
//! binding; then, after the run is registered, `record_fire` does an `INSERT OR IGNORE`
//! and reports whether the key was already there, returning the prior id either way.
//!
//! Defeating the PRE-CHECK entirely — replacing its lookup with `None` so it never fires
//! — leaves **every arm in this file green**. Two things absorb it. The post-write guard
//! still returns `deduped: true` with the first run's id, so the response is unchanged.
//! And the re-bound Motes are content-addressed, so re-submitting identical work adds no
//! rows to the projection. The pre-check's only observable effect is the work it skips.
//!
//! So: the arms below prove that a replay produces **no additional work in the run**, and
//! that property is genuine and is what an operator cares about. They do NOT prove the
//! pre-check functions, and nothing else does either. What they WOULD catch is a replay
//! that did DIFFERENT work — the shape the lost-race comment in `submit` describes — since
//! different work compiles to different Motes. Constructing that race deterministically is
//! not possible from here, which is a coverage gap and is recorded as one rather than
//! papered over.
//!
//! ## The defect this file was written RED against
//!
//! `submit` routes three ways — workflow target, App target, else recipe. `test` branches
//! on the App handle alone and has no workflow arm at all, so a workflow-target trigger
//! dry-runs through the RECIPE binder with an empty handle and reports a misleading
//! failure. It is the only validation verb a trigger has, and nothing called it.
//!
//! ```text
//!   cargo test -p kx-gateway --test trigger_fire_e2e
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tonic::transport::Channel;

/// The workflow every trigger here targets. A saved, active, caller-owned workflow is
/// the path an operator actually uses, and it is the one target kind proven to fire in
/// the ordinary suite.
const WORKFLOW: &str = "team/wf/fires";
/// A SECOND, genuinely different workflow — the control for "different work is a
/// different run". It has a third step, so its compiled Motes differ from the first's.
const OTHER_WORKFLOW: &str = "team/wf/other";

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// Whether this runtime has registered a run at all. Sound for exactly one question —
/// "did anything start" — because a serve hosts a single run registration.
async fn any_run_started(c: &mut KxGatewayClient<Channel>) -> bool {
    !c.list_runs(proto::ListRunsRequest {
        limit: None,
        before_seq: None,
    })
    .await
    .expect("ListRuns reaches the gateway")
    .into_inner()
    .runs
    .is_empty()
}

/// Poll until the run has admitted at least `want` Motes, then return the count.
///
/// Submission is asynchronous, so an immediate read is a race that reports zero. Panics
/// on the deadline rather than returning a short count, because a short count silently
/// satisfies every "did not grow" assertion below.
async fn motes_reach(c: &mut KxGatewayClient<Channel>, instance_id: &[u8], want: usize) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let n = mote_count(c, instance_id).await;
        if n >= want {
            return n;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the run admitted {n} Motes, expected at least {want} — the fire this test \
             is about did not happen, so nothing after it means anything"
        );
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
}

/// How many Motes the run has admitted. A trigger fire submits one per step of its
/// target; a deduplicated delivery submits none, because it returns before binding. This
/// is the witness that can tell those apart.
async fn mote_count(c: &mut KxGatewayClient<Channel>, instance_id: &[u8]) -> usize {
    c.get_projection(proto::GetProjectionRequest {
        instance_id: instance_id.to_vec(),
        at_seq: None,
    })
    .await
    .expect("GetProjection reaches the gateway")
    .into_inner()
    .motes
    .len()
}

/// Save a pure-step workflow of `steps` steps under `handle`. Pure steps need no model,
/// so the fired run settles on its own and this whole file stays in the ordinary suite.
///
/// Every step carries a prompt naming its handle and index. Motes are content-addressed,
/// so two workflows built from identical empty-prompt steps compile to the SAME Mote ids
/// and the second one appears to admit almost nothing — which is how the Mote count below
/// first read 3 where 5 was owed. Distinct prompts make the two targets genuinely
/// distinct work.
async fn save_workflow(c: &mut KxGatewayClient<Channel>, handle: &str, steps: usize) {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": (0..steps)
            .map(|i| serde_json::json!({
                "kind": "pure",
                "params": { "tag": format!("{handle}#{i}") }
            }))
            .collect::<Vec<_>>(),
        "edges": (1..steps)
            .map(|i| serde_json::json!({ "parent": i - 1, "child": i, "data": true }))
            .collect::<Vec<_>>()
    });
    let mut env = kx_app::WorkflowEnvelope::new(handle, blueprint);
    env.description = "a workflow a trigger targets".to_string();
    c.save_workflow(proto::SaveWorkflowRequest {
        handle: handle.to_string(),
        envelope_json: env.to_canonical_json().unwrap(),
        source_digest: Vec::new(),
        lifecycle: String::new(),
    })
    .await
    .expect("save the workflow");
}

fn wf_trigger(name: &str, workflow: &str) -> proto::RegisterTriggerRequest {
    proto::RegisterTriggerRequest {
        name: name.to_string(),
        kind: proto::TriggerKind::Grpc as i32,
        recipe_handle: String::new(),
        app_handle: String::new(),
        workflow_handle: workflow.to_string(),
        auth: proto::TriggerAuth::None as i32,
        auth_secret_ref: String::new(),
        schedule_spec: String::new(),
        timezone: String::new(),
        enabled: true,
        require_approval: false,
    }
}

async fn submit(
    c: &mut KxGatewayClient<Channel>,
    name: &str,
    key: &str,
    payload: &str,
) -> proto::SubmitTriggerResponse {
    c.submit_trigger(proto::SubmitTriggerRequest {
        name: name.to_string(),
        idempotency_key: key.to_string(),
        payload_json: payload.to_string(),
    })
    .await
    .expect("SubmitTrigger reaches the gateway")
    .into_inner()
}

async fn dry_run(
    c: &mut KxGatewayClient<Channel>,
    name: &str,
) -> Result<proto::TestTriggerResponse, tonic::Status> {
    c.test_trigger(proto::TestTriggerRequest {
        name: name.to_string(),
        payload_json: String::new(),
    })
    .await
    .map(tonic::Response::into_inner)
}

/// ★ The dry run validates and starts NOTHING; the fire starts the run.
///
/// The accepting control is the SAME trigger with the SAME payload, submitted instead of
/// tested. Without it, "no run appeared" would pass just as well on a trigger that could
/// not bind at all — which is exactly what a broken dry run looks like.
#[tokio::test]
async fn a_dry_run_validates_the_trigger_without_starting_anything() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    save_workflow(&mut c, WORKFLOW, 2).await;
    c.register_trigger(wf_trigger("nightly", WORKFLOW))
        .await
        .expect("register the trigger");

    // The precondition, asserted rather than assumed: nothing has run yet. Without this
    // the assertion below would be about a run some earlier step had already started.
    assert!(
        !any_run_started(&mut c).await,
        "no run exists before the trigger is exercised"
    );

    let dry = dry_run(&mut c, "nightly")
        .await
        .expect("the dry run answers");
    assert!(
        dry.ok,
        "the dry run must report that the trigger binds — if it cannot, the assertion \
         below is about a trigger that was never viable: {}",
        dry.detail
    );
    assert!(
        !dry.detail.is_empty(),
        "a dry run that says only `ok` tells an operator nothing about what it would do"
    );
    assert!(
        !any_run_started(&mut c).await,
        "A DRY RUN MUST START NOTHING. This is the whole point of the verb: an operator \
         validating a trigger before arming it has not asked for the work to happen. A \
         dry run that fired would have registered the run, which is what this reads"
    );

    // THE ACCEPTING CONTROL, one variable changed: submit instead of test.
    submit(&mut c, "nightly", "evt-control", "{}").await;
    assert!(
        any_run_started(&mut c).await,
        "the same trigger, the same payload, SUBMITTED, starts the run — without this \
         the assertion above would pass on a trigger that cannot start anything"
    );
}

/// ★ The dry run on a name that does not exist refuses, naming it.
#[tokio::test]
async fn a_dry_run_on_an_unknown_trigger_refuses_by_name() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let err = dry_run(&mut c, "no-such-trigger")
        .await
        .expect_err("an unknown trigger is refused");
    assert_eq!(err.code(), tonic::Code::NotFound, "got {err:?}");
    assert!(
        err.message().contains("no-such-trigger"),
        "the refusal names the trigger the operator asked about; got {:?}",
        err.message()
    );

    // The ACCEPTING control: the identical call against a trigger that DOES exist
    // answers. Without it this would pass on any failure at all — an unwired seam, a
    // broken store, a bad party.
    save_workflow(&mut c, WORKFLOW, 2).await;
    c.register_trigger(wf_trigger("real", WORKFLOW))
        .await
        .expect("register");
    assert!(
        dry_run(&mut c, "real").await.expect("answers").ok,
        "the same verb against a registered trigger works"
    );
}

/// ★ A replayed event adds NO WORK to the run — counted in Motes, not inferred from a
/// flag.
///
/// Read the module header's mutation note before extending this: it establishes that the
/// assertion holds structurally as well as through the dedup pre-check, so it is a guard
/// on the PROPERTY (no extra work appears) and not on the mechanism.
#[tokio::test]
async fn a_replayed_event_adds_no_work_to_the_run() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    save_workflow(&mut c, WORKFLOW, 2).await;
    c.register_trigger(wf_trigger("alerts", WORKFLOW))
        .await
        .expect("register");

    let first = submit(&mut c, "alerts", "evt-1", "{}").await;
    assert!(
        !first.deduped,
        "the first delivery of an event is not a replay"
    );
    let after_first = motes_reach(&mut c, &first.instance_id, 2).await;
    assert_eq!(
        after_first, 2,
        "the first delivery admits one Mote per step of the two-step workflow"
    );

    let replay = submit(&mut c, "alerts", "evt-1", "{}").await;
    assert!(replay.deduped, "the replay is recognised as one");
    assert_eq!(
        replay.instance_id, first.instance_id,
        "and it reports the run the first delivery started"
    );

    // THE ACCEPTING CONTROL, and also the synchronisation barrier. Reading the count
    // straight after the replay would be a race — an unfolded submission also reads as
    // "no work". So fire genuinely DIFFERENT work afterwards and wait for ITS Motes to
    // appear: once they have, anything the replay submitted would have arrived too.
    save_workflow(&mut c, OTHER_WORKFLOW, 3).await;
    c.register_trigger(wf_trigger("other-alerts", OTHER_WORKFLOW))
        .await
        .expect("register the second trigger");
    submit(&mut c, "other-alerts", "evt-2", "{}").await;
    let total = motes_reach(&mut c, &first.instance_id, after_first + 3).await;

    assert_eq!(
        total,
        after_first + 3,
        "THE REPLAY ADDED NO WORK. Had it bound and submitted DIFFERENT work — the \
         lost-race shape `submit` documents — the run would hold more than {} Motes. \
         Identical re-submitted work is absorbed by Mote content-addressing and is \
         invisible here, which the module header records",
        after_first + 3
    );
}

/// ★ A workflow-target trigger dry-runs against the WORKFLOW.
///
/// This arm was written RED. `submit` routes workflow → App → recipe, but `test` branches
/// on the App handle alone, so a workflow-target dry run fell through to the recipe
/// binder with an empty handle and reported a failure about a recipe that was never
/// named. The verb exists to tell an operator whether their trigger will work; for one of
/// its three target kinds it said the opposite of the truth.
#[tokio::test]
async fn a_workflow_target_dry_runs_against_the_workflow_not_an_empty_recipe() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    save_workflow(&mut c, WORKFLOW, 2).await;
    c.register_trigger(wf_trigger("wf-dry", WORKFLOW))
        .await
        .expect("register the workflow-target trigger");

    assert!(
        !any_run_started(&mut c).await,
        "no run exists before the dry run"
    );
    let dry = dry_run(&mut c, "wf-dry")
        .await
        .expect("the dry run answers");

    assert!(
        dry.ok,
        "a workflow-target trigger whose workflow is saved and active must DRY-RUN \
         CLEANLY. Before the workflow arm existed this reported a recipe failure, \
         because the dry run fell through to the recipe binder with an empty handle: {}",
        dry.detail
    );
    assert!(
        dry.detail.contains(WORKFLOW),
        "and the detail names the WORKFLOW it validated, not a recipe: {}",
        dry.detail
    );
    assert!(
        !any_run_started(&mut c).await,
        "and it still starts nothing — the workflow arm must dry-run, not fire"
    );
}
