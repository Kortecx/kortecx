//! A document attached to a run reaches the model, and deleting it stops the next run by
//! NAME.
//!
//! ## The scenario, named before the code (Rule 47)
//!
//! Someone attaches a document — a spec, a policy, a roster — to work they are asking the
//! runtime to do, and expects the answer to be grounded in it. Later they delete the
//! document, because it is wrong or because it should not have been shared. The next run
//! must then fail, saying which attachment it could not find — not quietly run without it
//! and answer from the model's own priors, which is the same answer shape as a grounded
//! one and is indistinguishable to the reader.
//!
//! ## What was uncovered
//!
//! `DeleteContextBundle`'s handler had no test caller in any language: the CLI verb and
//! the SDKs call it, none of their tests do, and the store-level `delete` test never
//! reaches the RPC. More striking, `context_bundles` — the field that makes a bundle
//! model-facing at all — is `vec![]` in EVERY integration test in the repo. A search for a
//! non-empty literal returns nothing. So the whole path from "a handle on a submit
//! request" to "text in the model's prompt" had never been exercised end to end.
//!
//! ## Two arms
//!
//! - [`an_attached_bundle_binds_and_deleting_it_fails_the_next_run_by_name`] — ordinary
//!   suite, no model. The entire attach → resolve → refuse path is observable at the RPC
//!   boundary, because the bind happens server-side during `SubmitWorkflow` and a missing
//!   handle is an `INVALID_ARGUMENT` carrying the handle. This is the arm that runs in CI.
//! - [`an_attached_document_is_quoted_in_the_answer`] — ignore-gated, needs a served
//!   model. The only way to show the text actually reaches the model is to ask for a fact
//!   that exists NOWHERE except the attachment.
//!
//! The live arm is dual-engine by necessity: it samples a model, so a result from one
//! engine is a claim about that engine and not about the runtime.
//!
//! ```text
//!   # the model-free arm runs by default:
//!   cargo test -p kx-gateway --test context_bundle_run_e2e
//!   # the live arm, Ollama:
//!   KX_SERVE_OLLAMA=on cargo test -p kx-gateway --features serve-engine \
//!     --test context_bundle_run_e2e -- --ignored --nocapture --test-threads=1
//!   # the live arm, llama.cpp (stop Ollama first — one engine per run):
//!   KX_SERVE_OLLAMA=off KX_SERVE_MODEL_GGUF=~/.kx-models/<model>.gguf \
//!     cargo test -p kx-gateway --features inference \
//!     --test context_bundle_run_e2e -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

const BUNDLE: &str = "team/ctx/roster";

/// A fact that exists nowhere but the attachment — not in the model's training data, not
/// in the prompt, not in any other fixture. If it comes back, it came from the document.
const ONLY_IN_THE_DOCUMENT: &str = "TRESTLE-62";

fn serve_model() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("KX_SERVE_MODEL_GGUF")?);
    p.is_file().then_some(p)
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// Upload the document and bind it under `BUNDLE`; return the content ref.
async fn attach_document(c: &mut KxGatewayClient<Channel>, body: &str) -> Vec<u8> {
    let content_ref = c
        .put_content(proto::PutContentRequest {
            payload: body.as_bytes().to_vec(),
            media_type: "text/plain".into(),
            filename: "roster.txt".into(),
        })
        .await
        .expect("PutContent reaches the gateway")
        .into_inner()
        .content_ref;

    c.put_context_bundle(proto::PutContextBundleRequest {
        handle: BUNDLE.into(),
        description: "the roster a run is grounded in".into(),
        items: vec![proto::ContextItem {
            name: "roster".into(),
            content_ref: content_ref.clone(),
            media_type: "text/plain".into(),
        }],
    })
    .await
    .expect("PutContextBundle reaches the gateway");

    content_ref
}

/// A one-step workflow, attaching `bundles`. `PURE` needs no model, so the model-free arm
/// exercises exactly the attach → resolve → refuse path and nothing else.
fn submit_req(bundles: Vec<String>) -> proto::SubmitWorkflowRequest {
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![proto::WorkflowStep {
            kind: proto::WorkflowStepKind::Pure as i32,
            model_id: String::new(),
            prompt: String::new(),
            body_signature_id: Vec::new(),
            tool_contract: HashMap::new(),
            params: HashMap::new(),
        }],
        edges: vec![],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: bundles,
    }
}

// ---------------------------------------------------------------------------
// Arm 1 — the ordinary suite. No model, no non-default feature.
// ---------------------------------------------------------------------------

/// ★ An attached bundle binds; deleting it makes the identical submit fail, naming it.
///
/// The accepting control is the SAME request before the delete. Without it, an
/// `INVALID_ARGUMENT` after the delete would be equally consistent with a submit that was
/// never valid — a bad step, an unsupported mode, a malformed handle.
#[tokio::test(flavor = "multi_thread")]
async fn an_attached_bundle_binds_and_deleting_it_fails_the_next_run_by_name() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    attach_document(&mut c, "roster: kestrel, TRESTLE-62, harrier").await;

    // THE ACCEPTING CONTROL: the attached submit is accepted. The bind resolves the
    // handle server-side, so this succeeding IS the proof the attachment was resolved.
    let ok = c
        .submit_workflow(submit_req(vec![BUNDLE.to_string()]))
        .await
        .expect("a submit carrying an attached bundle is accepted")
        .into_inner();
    assert_eq!(
        ok.instance_id.len(),
        16,
        "the attached run was registered and carries an identity"
    );

    // The one variable: the document is deleted.
    assert!(
        c.delete_context_bundle(proto::DeleteContextBundleRequest {
            handle: BUNDLE.into(),
        })
        .await
        .expect("DeleteContextBundle reaches the gateway")
        .into_inner()
        .removed,
        "the bundle existed and was removed"
    );

    let err = c
        .submit_workflow(submit_req(vec![BUNDLE.to_string()]))
        .await
        .expect_err("the identical submit is now refused");
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "a missing attachment is a bad request, not an internal failure; got {err:?}"
    );
    assert!(
        err.message().contains(BUNDLE),
        "THE REFUSAL MUST NAME THE HANDLE. A run that quietly proceeded without its \
         attachment would answer from the model's own priors, in the same shape as a \
         grounded answer, and nothing downstream could tell them apart; got {:?}",
        err.message()
    );

    // And deleting it again reports not-removed — the control that `removed` is a real
    // answer rather than a constant.
    assert!(
        !c.delete_context_bundle(proto::DeleteContextBundleRequest {
            handle: BUNDLE.into(),
        })
        .await
        .expect("DeleteContextBundle reaches the gateway")
        .into_inner()
        .removed,
        "deleting an absent bundle reports not-removed"
    );
}

/// ★ A handle that never existed is refused the same way, and an EMPTY attachment list is
/// unaffected.
///
/// The empty case is the compatibility boundary: a submit that attaches nothing must not
/// touch the bundle store at all, so every existing caller is unchanged.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_handle_is_refused_and_attaching_nothing_still_works() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let err = c
        .submit_workflow(submit_req(vec!["team/ctx/never-existed".to_string()]))
        .await
        .expect_err("a handle that was never stored is refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got {err:?}");
    assert!(
        err.message().contains("team/ctx/never-existed"),
        "the refusal names the handle it could not resolve; got {:?}",
        err.message()
    );

    // THE CONTROL, one variable: attaching NOTHING is accepted on the same gateway.
    let ok = c
        .submit_workflow(submit_req(vec![]))
        .await
        .expect("a submit attaching nothing is unaffected")
        .into_inner();
    assert_eq!(ok.instance_id.len(), 16);
}

// ---------------------------------------------------------------------------
// Arm 2 — against a served model. The only way to show the text ARRIVES.
// ---------------------------------------------------------------------------

/// ★ A fact that exists ONLY in the attached document comes back in the answer.
///
/// Everything above proves the handle resolved. None of it proves the document's TEXT
/// reached the model — the bundle could resolve to item refs that are then dropped before
/// the prompt is rendered, and every assertion so far would still pass. The only witness
/// is an answer that could not have been produced without reading the attachment.
///
/// Dual-engine by necessity: this samples a model, so one engine's result is a claim
/// about that engine.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a served model; opt in with --ignored"]
async fn an_attached_document_is_quoted_in_the_answer() {
    let ollama = std::env::var("KX_SERVE_OLLAMA").is_ok_and(|v| v == "on" || v == "auto");
    let gguf = serve_model();
    if !ollama && gguf.is_none() {
        eprintln!(
            "skipping: no served model — set KX_SERVE_MODEL_GGUF to a real GGUF, or \
             KX_SERVE_OLLAMA=on"
        );
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    if let Some(p) = &gguf {
        std::env::set_var("KX_SERVE_MODEL_GGUF", p);
    }

    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    attach_document(
        &mut c,
        &format!(
            "Vessel roster.\nThe hull registration of the vessel named Kestrel is \
             {ONLY_IN_THE_DOCUMENT}.\nThe hull registration of Harrier is BRAMBLE-08.\n"
        ),
    )
    .await;

    let model_step = vec![proto::WorkflowStep {
        kind: proto::WorkflowStepKind::Model as i32,
        model_id: String::new(),
        prompt: "What is the hull registration of the vessel named Kestrel? \
                 Answer with the registration only."
            .to_string(),
        body_signature_id: Vec::new(),
        tool_contract: HashMap::new(),
        params: HashMap::new(),
    }];
    let mut req = submit_req(vec![BUNDLE.to_string()]);
    req.steps.clone_from(&model_step);

    let grounded = run_and_read(&mut c, req).await;
    eprintln!("context-bundle live answer WITH the attachment:    {grounded}");
    assert!(
        grounded.contains(ONLY_IN_THE_DOCUMENT),
        "THE ANSWER MUST CARRY THE FACT THAT EXISTS ONLY IN THE ATTACHMENT. Without it \
         the model answered from its own priors, which is the failure this arm exists to \
         detect — every model-free assertion in this file passes in that case too. \
         Answer was: {grounded}"
    );

    // ★ THE REFUSING CONTROL, and the reason this arm is an instrument rather than a
    // ceremony: the IDENTICAL question with NO attachment must NOT produce the fact. If it
    // did, the assertion above would pass whether or not the document ever reached the
    // model, and the whole arm would be measuring the model's priors.
    let mut bare = submit_req(vec![]);
    bare.steps.clone_from(&model_step);
    let ungrounded = run_and_read(&mut c, bare).await;
    eprintln!("context-bundle live answer WITHOUT the attachment: {ungrounded}");
    assert!(
        !ungrounded.contains(ONLY_IN_THE_DOCUMENT),
        "the same question WITHOUT the attachment produced the fact anyway, so the \
         assertion above proves nothing about whether the document reached the model. \
         Change the fact to something the model cannot know. Answer was: {ungrounded}"
    );
}

/// Submit and poll until THIS submission's terminal Mote commits, returning its result.
///
/// Scoped by `terminal_mote_id`, never by position. A serve hosts one run, so the
/// projection accumulates every submission's Motes under the same `instance_id` — reading
/// the first committed Mote returns an EARLIER submission's answer. That is not a
/// hypothetical: the control below read the grounded answer back for the ungrounded run
/// and reported the attachment as unnecessary.
async fn run_and_read(
    c: &mut KxGatewayClient<Channel>,
    req: proto::SubmitWorkflowRequest,
) -> String {
    let handle = c
        .submit_workflow(req)
        .await
        .expect("SubmitWorkflow reaches the gateway")
        .into_inner();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: handle.instance_id.clone(),
                at_seq: None,
            })
            .await
            .expect("GetProjection reaches the gateway")
            .into_inner();
        let mine = view.motes.iter().find(|m| {
            m.mote_id == handle.terminal_mote_id
                && m.state == proto::MoteSnapshotState::Committed as i32
        });
        if let Some(result_ref) = mine.and_then(|m| m.result_ref.clone()) {
            return String::from_utf8_lossy(
                &c.get_content(proto::GetContentRequest {
                    content_ref: result_ref,
                    instance_id: handle.instance_id.clone(),
                })
                .await
                .expect("read the answer")
                .into_inner()
                .payload,
            )
            .to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("the model step did not settle within the budget");
}
