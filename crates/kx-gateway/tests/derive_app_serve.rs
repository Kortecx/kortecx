//! `DeriveApp` — the RPC behind "describe an app and get one" — driven on a live model.
//!
//! It had **no test at any level**: not a unit test through the RPC, not an
//! integration test, not a live one. The inline `#[cfg(test)]` modules in
//! `derive_host` / `derive_plan` drive `derive_blocking` and `decode_derived`
//! directly against a scripted backend, so nothing has ever exercised the handler,
//! the host→wire field mapping, or a real decode.
//!
//! ⚠ WHY IT WAS MISSED, and why this file is gated the way it is. The deriver is
//! wired under `#[cfg(feature = "serve-engine")]`, but the only live-serve test
//! file that could have witnessed it is `#![cfg(feature = "inference")]`. Since
//! `inference = ["serve-engine", ...]` is one-directional, that file compiles to an
//! EMPTY harness under `console,serve-engine,hnsw,hosted-apps,observability` — the
//! feature set the live proofs actually build. The RPC was live on a build where
//! its would-be witness was switched off. Proven by probe: renaming a helper in
//! that file leaves the RC-feature build compiling clean and running 0 tests;
//! renaming one in THIS file breaks it.
//!
//! ⚠ WHAT TO ASSERT. `DeriveApp` degrades a failed file-plan decode to a NOTICE
//! (`derive_host.rs`), so "the RPC returned an app" is compatible with the app
//! carrying nothing. Every assertion below is on the ARTEFACT, and the notice list
//! is checked for the degrade markers rather than ignored.
//!
//! ```text
//! KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma4:12b \
//!   cargo test -p kx-gateway --features serve-engine,hnsw \
//!   --test derive_app_serve -- --ignored --nocapture --test-threads=1
//! ```
#![cfg(feature = "serve-engine")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// Markers a HEALTHY derive must not carry. Each is a substring of a notice
/// `derive_host` emits when it degrades rather than refuses — the whole reason a
/// "successful" derive can be empty. Sourced from the emission sites, not guessed.
const DEGRADE_MARKERS: [&str; 6] = [
    "file plan could not be prepared",
    "outside what this account can fire",
    "not found, so not attached",
    "than fit the design prompt",
    "rather than to a step",
    "is not an available template",
];

/// The frameworks the hosted lane accepts (`HOSTED_FRAMEWORKS`).
const HOSTED_FRAMEWORKS: [&str; 3] = ["vite_react", "next_js", "svelte"];

fn serve_gguf() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var("KX_SERVE_MODEL_GGUF").ok()?);
    p.is_file().then_some(p)
}

fn ollama_opted_in() -> bool {
    std::env::var("KX_SERVE_OLLAMA").is_ok_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        matches!(v.as_str(), "1" | "on" | "true" | "yes")
    })
}

fn resolve_engine() -> Option<&'static str> {
    if ollama_opted_in() {
        return Some("ollama");
    }
    let gguf = serve_gguf()?;
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    Some("llamacpp")
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// Notices carrying a degrade marker, with the marker named so a failure says
/// WHICH degradation happened rather than only that one did.
fn degradations(notices: &[String]) -> Vec<String> {
    notices
        .iter()
        .filter(|n| DEGRADE_MARKERS.iter().any(|m| n.contains(m)))
        .cloned()
        .collect()
}

fn derive_request(kind: &str, prompt: &str, framework: &str) -> proto::DeriveAppRequest {
    proto::DeriveAppRequest {
        kind: kind.to_string(),
        mode: String::new(),
        prompt: prompt.to_string(),
        framework: framework.to_string(),
        attachments: vec![],
    }
}

/// The SCHEDULED lane: a described app becomes a real multi-step design.
///
/// A scheduled decode failure is a hard `Rejected` (unlike the hosted file plan,
/// which degrades to a notice — one decoder, two policies), so a `Derived` here
/// means the model genuinely produced a plan that passed `compile_plan`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs Ollama (KX_SERVE_OLLAMA=on) or a GGUF; opt in with --ignored"]
async fn derive_app_designs_a_scheduled_app_from_a_description_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_OLLAMA=on or KX_SERVE_MODEL_GGUF");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let resp = c
        .derive_app(derive_request(
            "scheduled",
            "Every morning, collect the new issues filed on our repo overnight, \
             summarise them by theme, and write a short digest.",
            "",
        ))
        .await
        .expect("DeriveApp must be wired on a serve-engine build with a resolved model")
        .into_inner();

    let app = match resp.result {
        Some(proto::derive_app_response::Result::App(a)) => a,
        Some(proto::derive_app_response::Result::Rejected(r)) => {
            // Model-nondeterministic, and honest: the RPC + decode + compile path
            // was still exercised. Assert the refusal SAYS something, then stop.
            assert!(
                !r.reason.trim().is_empty(),
                "a refusal must carry a reason a user can act on"
            );
            eprintln!("LIVE derive/scheduled [{engine}]: rejected — {}", r.reason);
            running.shutdown().await.unwrap();
            return;
        }
        None => panic!("DeriveApp returned neither an app nor a rejection"),
    };

    eprintln!(
        "LIVE derive/scheduled [{engine}]: name={:?} steps={} edges={} tools={} notices={:?}",
        app.name,
        app.steps.len(),
        app.edges.len(),
        app.tools.len(),
        app.notices
    );

    // --- the ARTEFACT, not the fact that a response arrived ---
    assert!(!app.name.trim().is_empty(), "the design names the app");
    assert!(
        !app.steps.is_empty(),
        "a scheduled design carries its steps (this is the field a degraded derive loses)"
    );
    for (i, s) in app.steps.iter().enumerate() {
        assert!(
            !s.role.trim().is_empty(),
            "step {i} resolved a vetted role — the model cannot invent one"
        );
        assert!(
            !s.intent.trim().is_empty(),
            "step {i} says what it is for (the intent is what the author reviews)"
        );
    }
    // Edges index into steps; an out-of-range edge would mean the wire mapping
    // dropped or reordered a step. Nothing else tests that mapping.
    let n = u32::try_from(app.steps.len()).unwrap();
    for e in &app.edges {
        assert!(
            e.parent < n && e.child < n,
            "edge {}->{} indexes the {n} steps that crossed the wire",
            e.parent,
            e.child
        );
        assert_ne!(e.parent, e.child, "a step does not depend on itself");
    }
    // The hosted-only fields stay empty on this lane.
    assert!(app.files.is_empty(), "a scheduled design plans no files");
    assert!(
        app.framework.is_empty(),
        "framework is the hosted lane's field"
    );

    let degraded = degradations(&app.notices);
    assert!(
        degraded.is_empty(),
        "the derive DEGRADED rather than succeeding — these notices mean the design is \
         missing something the caller will not otherwise notice: {degraded:?}"
    );

    running.shutdown().await.unwrap();
}

/// The HOSTED lane: the file plan is the artefact, and the degrade-to-notice path
/// is exactly here.
///
/// `derive_host` folds BOTH a model dispatch failure and a manifest decode failure
/// into one notice and returns an app with `files` empty. So asserting `files` is
/// non-empty AND that the notice is absent is the only way to tell a real file plan
/// from a silently skipped one — they are the same response shape otherwise.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs Ollama (KX_SERVE_OLLAMA=on) or a GGUF; opt in with --ignored"]
async fn derive_app_plans_hosted_files_without_degrading_to_a_notice_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_OLLAMA=on or KX_SERVE_MODEL_GGUF");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let resp = c
        .derive_app(derive_request(
            "hosted",
            "A single-page dashboard that shows the status of our nightly jobs, \
             with a filter by job name and a panel for the most recent failures.",
            "vite_react",
        ))
        .await
        .expect("DeriveApp must be wired")
        .into_inner();

    let app = match resp.result {
        Some(proto::derive_app_response::Result::App(a)) => a,
        Some(proto::derive_app_response::Result::Rejected(r)) => {
            assert!(!r.reason.trim().is_empty(), "a refusal carries a reason");
            eprintln!("LIVE derive/hosted [{engine}]: rejected — {}", r.reason);
            running.shutdown().await.unwrap();
            return;
        }
        None => panic!("DeriveApp returned neither an app nor a rejection"),
    };

    eprintln!(
        "LIVE derive/hosted [{engine}]: name={:?} framework={:?} files={} notices={:?}",
        app.name,
        app.framework,
        app.files.len(),
        app.notices
    );

    assert!(!app.name.trim().is_empty(), "the design names the app");
    assert!(
        HOSTED_FRAMEWORKS.contains(&app.framework.as_str()),
        "framework {:?} is one the scaffold can build",
        app.framework
    );
    assert!(
        app.steps.is_empty(),
        "the hosted lane drops the reasoning steps — a hosted app has no DAG"
    );

    // THE ASSERTION THIS FILE EXISTS FOR. Both halves are required: a file plan
    // that decoded, and no notice saying it did not.
    let degraded = degradations(&app.notices);
    assert!(
        degraded.is_empty(),
        "the hosted derive DEGRADED — the response still looks successful, which is the \
         defect: {degraded:?}"
    );
    assert!(
        !app.files.is_empty(),
        "the file plan is the hosted lane's artefact; an empty plan with no degrade notice \
         means the decode returned nothing and nobody was told"
    );
    for f in &app.files {
        assert!(!f.path.trim().is_empty(), "every planned file has a path");
        assert!(
            !f.path.starts_with('/') && !f.path.contains(".."),
            "planned path {:?} stays inside the project",
            f.path
        );
    }

    running.shutdown().await.unwrap();
}

/// THE ACCEPTING CONTROL for the two oracles above, and a refusal-reason oracle in
/// its own right.
///
/// An unknown `kind` is refused BEFORE any model turn, so this is deterministic
/// given a wired deriver. It earns its place twice: a negative test that passes on
/// any failure is worthless, so the reason is asserted; and if this passes while
/// the live oracles skip, the skip was the model, not the plumbing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a wired deriver (serve-engine + a resolved model); opt in with --ignored"]
async fn derive_app_refuses_an_unknown_kind_and_says_why_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_OLLAMA=on or KX_SERVE_MODEL_GGUF");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let resp = c
        .derive_app(derive_request("wharrgarbl", "Summarise my email.", ""))
        .await
        .expect("DeriveApp must be wired")
        .into_inner();

    match resp.result {
        Some(proto::derive_app_response::Result::Rejected(r)) => {
            eprintln!("LIVE derive/refusal [{engine}]: {}", r.reason);
            // Assert the REASON, not merely that it failed: silently designing the
            // wrong KIND of app is the one mistake the author cannot see in review,
            // so the refusal has to name the kind it did not understand.
            assert!(
                r.reason.contains("unknown app kind"),
                "the refusal names the cause, not just that something went wrong: {:?}",
                r.reason
            );
            assert!(
                r.reason.contains("wharrgarbl"),
                "the refusal quotes the offending value back: {:?}",
                r.reason
            );
            assert!(
                r.reason.contains("scheduled") && r.reason.contains("hosted"),
                "the refusal says what WOULD be accepted: {:?}",
                r.reason
            );
        }
        Some(proto::derive_app_response::Result::App(a)) => panic!(
            "an unknown kind was DEFAULTED into a design instead of refused (name={:?}) — \
             the author would review an app of a kind they never asked for",
            a.name
        ),
        None => panic!("DeriveApp returned neither an app nor a rejection"),
    }

    // The one-variable accepting control: same request, kind corrected. It must NOT
    // be refused for the same reason — otherwise the refusal above could be firing
    // on something else entirely (an unwired deriver, say).
    let ok = c
        .derive_app(derive_request("scheduled", "Summarise my email.", ""))
        .await
        .expect("DeriveApp must be wired")
        .into_inner();
    match ok.result {
        Some(proto::derive_app_response::Result::App(_)) => {}
        Some(proto::derive_app_response::Result::Rejected(r)) => assert!(
            !r.reason.contains("unknown app kind"),
            "the accepting control was refused for the SAME reason, so the negative arm \
             proves nothing about the kind check: {:?}",
            r.reason
        ),
        None => panic!("DeriveApp returned neither an app nor a rejection"),
    }

    running.shutdown().await.unwrap();
}

/// An empty prompt is refused at the HANDLER, before the deriver — a different
/// layer from the kind check, and the only refusal that never reaches the model.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a wired deriver (serve-engine + a resolved model); opt in with --ignored"]
async fn derive_app_refuses_an_empty_prompt_at_the_handler_live() {
    let Some(_engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_OLLAMA=on or KX_SERVE_MODEL_GGUF");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let status = c
        .derive_app(derive_request("scheduled", "   ", ""))
        .await
        .expect_err("an empty prompt is refused");
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "an empty prompt is the CALLER's error, not an internal one: {status:?}"
    );
    assert!(
        status.message().contains("prompt"),
        "the status names the field at fault: {:?}",
        status.message()
    );

    running.shutdown().await.unwrap();
}
