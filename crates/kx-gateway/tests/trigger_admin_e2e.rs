//! D113 trigger-seam RPC e2e: drives `RegisterTrigger` / `ListTriggers` /
//! `DeregisterTrigger` / `SubmitTrigger` / `TestTrigger` through the REAL gateway
//! service (the handlers + the host `TriggerAdmin` seam + the off-journal triggers.db),
//! deterministically (no model). The actual event→run FIRE (trigger → real Gemma run)
//! is covered by the live dual-engine validation; here we prove the admin RPC wiring,
//! the governance view (secret referenced by NAME only), idempotent re-register, and
//! the not-found / dry-run paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, GatewayConfig};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tonic::transport::Channel;

mod common;

fn config(dir: &TempDir) -> GatewayConfig {
    common::gateway_config(dir, true, HashMap::new())
}

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
async fn trigger_admin_register_list_deregister_round_trips() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // Register a WEBHOOK trigger with an HMAC secret referenced by NAME.
    let reg = c
        .register_trigger(proto::RegisterTriggerRequest {
            name: "alert".to_string(),
            kind: proto::TriggerKind::Webhook as i32,
            recipe_handle: "kx/recipes/react".to_string(),
            auth: proto::TriggerAuth::HmacSha256 as i32,
            auth_secret_ref: "HOOK_SECRET".to_string(),
            schedule_spec: String::new(),
            enabled: true,
            app_handle: String::new(),
            timezone: String::new(),
            require_approval: false,
        })
        .await
        .expect("register")
        .into_inner();
    assert_eq!(
        reg.trigger_id.len(),
        16,
        "server-derived 16-byte trigger id"
    );
    assert_ne!(reg.trigger_id, vec![0u8; 16], "id is derived, not zero");

    // List shows the governance row — auth_secret_present, NEVER the secret value.
    let list = c
        .list_triggers(proto::ListTriggersRequest {
            limit: 0,
            after_name: String::new(),
        })
        .await
        .expect("list")
        .into_inner();
    let row = list
        .triggers
        .iter()
        .find(|t| t.name == "alert")
        .expect("the registered trigger is listed");
    assert_eq!(row.kind, proto::TriggerKind::Webhook as i32);
    assert_eq!(row.auth, proto::TriggerAuth::HmacSha256 as i32);
    assert!(
        row.auth_secret_present,
        "an auth-secret ref NAME is attached"
    );
    assert!(row.enabled);
    // The secret NAME may travel (it is a reference, D81), but no value/secret bytes
    // are on the row — the row carries only `auth_secret_present` (a bool).
    let row_dbg = format!("{row:?}");
    assert!(
        !row_dbg.contains("HOOK_SECRET_VALUE"),
        "no secret value on the governance row"
    );

    // Re-register the same name is idempotent (same server-derived id).
    let reg2 = c
        .register_trigger(proto::RegisterTriggerRequest {
            name: "alert".to_string(),
            kind: proto::TriggerKind::Webhook as i32,
            recipe_handle: "kx/recipes/react".to_string(),
            auth: proto::TriggerAuth::Bearer as i32,
            auth_secret_ref: "HOOK_SECRET".to_string(),
            schedule_spec: String::new(),
            enabled: true,
            app_handle: String::new(),
            timezone: String::new(),
            require_approval: false,
        })
        .await
        .expect("re-register")
        .into_inner();
    assert_eq!(reg2.trigger_id, reg.trigger_id, "re-register keeps the id");

    // Deregister removes it (and is a no-op the second time).
    assert!(
        c.deregister_trigger(proto::DeregisterTriggerRequest {
            name: "alert".to_string(),
        })
        .await
        .expect("deregister")
        .into_inner()
        .removed
    );
    assert!(
        !c.deregister_trigger(proto::DeregisterTriggerRequest {
            name: "alert".to_string(),
        })
        .await
        .expect("deregister again")
        .into_inner()
        .removed,
        "second deregister is a no-op"
    );
}

#[tokio::test]
async fn submit_trigger_unknown_name_is_not_found() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let err = c
        .submit_trigger(proto::SubmitTriggerRequest {
            name: "ghost".to_string(),
            idempotency_key: String::new(),
            payload_json: "{}".to_string(),
        })
        .await
        .expect_err("an unknown trigger is refused");
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn register_trigger_rejects_unknown_kind_and_missing_fields() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // Unspecified kind ⇒ invalid_argument.
    let err = c
        .register_trigger(proto::RegisterTriggerRequest {
            name: "bad".to_string(),
            kind: proto::TriggerKind::Unspecified as i32,
            recipe_handle: "kx/recipes/react".to_string(),
            auth: proto::TriggerAuth::None as i32,
            auth_secret_ref: String::new(),
            schedule_spec: String::new(),
            enabled: true,
            app_handle: String::new(),
            timezone: String::new(),
            require_approval: false,
        })
        .await
        .expect_err("unknown kind refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    // HMAC auth with no secret ref ⇒ invalid_argument (cannot verify).
    let err = c
        .register_trigger(proto::RegisterTriggerRequest {
            name: "nohmac".to_string(),
            kind: proto::TriggerKind::Webhook as i32,
            recipe_handle: "kx/recipes/react".to_string(),
            auth: proto::TriggerAuth::HmacSha256 as i32,
            auth_secret_ref: String::new(),
            schedule_spec: String::new(),
            enabled: true,
            app_handle: String::new(),
            timezone: String::new(),
            require_approval: false,
        })
        .await
        .expect_err("hmac without a secret ref refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// T-APP-TRIGGER-TARGET: a cron trigger targeting a saved App with a 5-field crontab
// expression, a timezone, and the per-trigger HITL posture round-trips through the
// governance view (the App-run seam is wired on the default e2e serve; register does
// not check App existence — a fire against a missing App fails at submit).
#[tokio::test]
async fn register_app_target_cron_trigger_round_trips_view() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let reg = c
        .register_trigger(proto::RegisterTriggerRequest {
            name: "standup".to_string(),
            kind: proto::TriggerKind::Cron as i32,
            recipe_handle: String::new(),
            app_handle: "standup-digest".to_string(),
            auth: proto::TriggerAuth::None as i32,
            auth_secret_ref: String::new(),
            schedule_spec: "0 9 * * 1-5".to_string(),
            timezone: "America/New_York".to_string(),
            enabled: true,
            require_approval: true,
        })
        .await
        .expect("register app-target cron trigger")
        .into_inner();
    assert_eq!(reg.trigger_id.len(), 16);

    let list = c
        .list_triggers(proto::ListTriggersRequest {
            limit: 0,
            after_name: String::new(),
        })
        .await
        .expect("list")
        .into_inner();
    let row = list
        .triggers
        .iter()
        .find(|t| t.name == "standup")
        .expect("the app-target trigger is listed");
    assert_eq!(row.app_handle, "standup-digest", "App target surfaced");
    assert!(row.recipe_handle.is_empty(), "no recipe target");
    assert_eq!(row.schedule_spec, "0 9 * * 1-5");
    assert_eq!(row.timezone, "America/New_York");
    assert!(row.require_approval, "HITL posture surfaced");
}

// D213: a hosted (experience) App carries no blueprint and is served by the hosted-app
// supervisor, never scheduled — REGISTER must refuse it as a trigger target (fail fast).
#[tokio::test]
async fn register_rejects_a_hosted_experience_app_as_a_trigger_target() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // Save a hosted (experience) App: schema kortecx.experience/v1, a hosted config, no
    // blueprint, a branch_handle (the project file tree).
    let envelope = br#"{"schema":"kortecx.experience/v1","name":"landing","version":"1","hosted":{"framework":"vite_react"},"branch_handle":"my/landing/main"}"#;
    c.save_app(proto::SaveAppRequest {
        handle: "my/landing/site".to_string(),
        envelope_json: envelope.to_vec(),
        source_digest: Vec::new(),
    })
    .await
    .expect("save the experience app")
    .into_inner();

    // Registering a cron trigger pointing at it must fail closed (invalid_argument).
    let err = c
        .register_trigger(proto::RegisterTriggerRequest {
            name: "nope".to_string(),
            kind: proto::TriggerKind::Cron as i32,
            recipe_handle: String::new(),
            app_handle: "my/landing/site".to_string(),
            auth: proto::TriggerAuth::None as i32,
            auth_secret_ref: String::new(),
            schedule_spec: "0 9 * * 1-5".to_string(),
            timezone: "America/New_York".to_string(),
            enabled: true,
            require_approval: false,
        })
        .await
        .expect_err("a hosted app must not be schedulable");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got {err:?}");
}

// T-APP-TRIGGER-TARGET: exactly one of recipe_handle | app_handle.
#[tokio::test]
async fn register_rejects_both_and_neither_targets() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let mk = |recipe: &str, app: &str| proto::RegisterTriggerRequest {
        name: "x".to_string(),
        kind: proto::TriggerKind::Grpc as i32,
        recipe_handle: recipe.to_string(),
        app_handle: app.to_string(),
        auth: proto::TriggerAuth::None as i32,
        auth_secret_ref: String::new(),
        schedule_spec: String::new(),
        timezone: String::new(),
        enabled: true,
        require_approval: false,
    };
    // Neither.
    let err = c
        .register_trigger(mk("", ""))
        .await
        .expect_err("neither target refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    // Both.
    let err = c
        .register_trigger(mk("kx/recipes/react", "some-app"))
        .await
        .expect_err("both targets refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// T-APP-TRIGGER-TARGET: a malformed 5-field cron expression / unknown timezone is
// rejected at register (a synchronous error, never a silent never-firing trigger).
#[tokio::test]
async fn register_cron_rejects_bad_expr_and_timezone() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let mk = |sched: &str, tz: &str| proto::RegisterTriggerRequest {
        name: "cronbad".to_string(),
        kind: proto::TriggerKind::Cron as i32,
        recipe_handle: "kx/recipes/react".to_string(),
        app_handle: String::new(),
        auth: proto::TriggerAuth::None as i32,
        auth_secret_ref: String::new(),
        schedule_spec: sched.to_string(),
        timezone: tz.to_string(),
        enabled: true,
        require_approval: false,
    };
    let err = c
        .register_trigger(mk("not a cron", ""))
        .await
        .expect_err("bad cron refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    let err = c
        .register_trigger(mk("0 9 * * *", "Mars/Phobos"))
        .await
        .expect_err("bad timezone refused");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    // A legacy interval-seconds spec still registers (back-compat).
    c.register_trigger(mk("300", ""))
        .await
        .expect("legacy interval still valid");
}
