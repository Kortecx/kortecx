//! W2 — an operator NARROWS what an agent may reach, and the runtime obeys.
//!
//! ## The scenario, named before the code (Rule 47)
//!
//! An operator is running an agent that has been granted a set of tools. They decide it
//! should reach fewer of them, so they define a role naming only the tools they are happy
//! with and assign their party to it. From that moment the App's capability manifest — the
//! surface the console and `kx app manifest` both read to answer *"what can this App
//! actually do"* — must report exactly the narrowed set, and deleting the role must widen
//! it back. The failure branches that matter: assigning a role that does not exist must be
//! refused rather than silently ignored (a silent success leaves the operator believing a
//! narrowing is in force while the party is fully un-narrowed — the failure that looks
//! exactly like the safe state), and a role naming NO tool is a coherent request that
//! refuses everything.
//!
//! These four RPCs (`PutPolicyRole` / `ListPolicyRoles` / `DeletePolicyRole` /
//! `AssignPolicyRole`) had no test at any level before this file.
//!
//! ## Two arms, because the ceiling has two sizes — and a claim this file refuted
//!
//! Policy narrowing bites through exactly one function, `app_run::principal_tool_ceiling`,
//! whose answer is `broker_registered_grants ∩ tool_registry`. **A policy that narrows an
//! EMPTY ceiling is unobservable**, so what the broker has registered decides what this
//! file can prove.
//!
//! It was first written as a live-only test, on the reasoning that every capability able to
//! populate the broker needs a served model (`fs-list`/`fs-read` register only when
//! `serve_model.is_some() && KX_SERVE_FS_ROOT` resolves; `mcp-echo/echo` likewise;
//! `retrieve@1` needs `serve-engine + hnsw`) and that the ceiling is therefore empty
//! without one. **Measurement refuted that.** With `KX_SERVE_OLLAMA=off` and no GGUF the
//! ceiling is `[("http", "1")]`, not empty: `http@1` registers under `mcp-gateway`, a
//! DEFAULT feature, with no model anywhere. So a model-free arm is possible, and omitting
//! it would have cost this family every bit of continuous CI coverage it could have had.
//!
//! - [`a_policy_role_narrows_a_model_free_serve`] — runs in the ordinary suite: one
//!   registered tool, narrowed to none, widened back.
//! - [`a_policy_role_narrows_what_an_inherit_reach_app_can_reach`] — `#[ignore]`, the
//!   fuller scenario against a served model, where the ceiling also holds `fs-list@1`,
//!   `fs-read@1` and `mcp-echo/echo@1` and the narrowing is many-to-one, not one-to-none.
//!
//! ## Determinism, and why the model-free arm pins the engine
//!
//! `KX_SERVE_OLLAMA` UNSET means `auto`, which is ON whenever no GGUF is configured — so on
//! a box with an Ollama daemon the "model-free" arm would quietly acquire a served model
//! and a four-tool ceiling. The first run of this experiment was confounded exactly that
//! way and read four tools where it should have read one. The model-free arm therefore pins
//! `KX_SERVE_OLLAMA=off` itself rather than trusting the ambient environment.
//!
//! Nothing here samples a model: every assertion is deterministic given the serve's own
//! reported ceiling, which is why the live arm is single-engine by design — a second engine
//! would measure the same bytes.
//!
//! ```text
//!   # the model-free arm runs by default:
//!   cargo test -p kx-gateway --test policy_admin_e2e
//!   # the live arm:
//!   KX_SERVE_MODEL_GGUF=~/.kx-models/gemma-4-12b-it-q4_k_m.gguf \
//!     cargo test -p kx-gateway --features inference --test policy_admin_e2e \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The two arms set different `KX_SERVE_*` variables, which are process-global, so running
//! them TOGETHER (`--include-ignored`) needs `--test-threads=1`. By default they never run
//! together — one is `#[ignore]`.

#![cfg(feature = "mcp-gateway")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tonic::transport::Channel;

/// The party a `--dev-allow-local` caller resolves to (`auth::DevAllowLocal`). Policy roles
/// are assigned to a PARTY and the manifest resolves the ceiling for the calling principal,
/// so these must be the same string or the narrowing silently does nothing.
const PARTY: &str = "local-dev";

const APP_HANDLE: &str = "apps/local/policy-reach";
const ROLE: &str = "narrowed";

fn serve_model() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("KX_SERVE_MODEL_GGUF")?);
    p.is_file().then_some(p)
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

/// An App whose tool axis INHERITS the caller's ceiling rather than declaring its own — the
/// only reach under which a policy role is observable at all. Envelope shape as in
/// `eval_bench_real::reach_app_envelope`.
fn inherit_app_envelope() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "kortecx.app/v1",
        "version": "1",
        "name": "policy-reach",
        "blueprint": { "steps": [ { "kind": "model", "prompt":
            "List what you can see, then answer." } ] },
        "steering_config": {
            "tools": { "reach": "inherit_principal" },
            "guards": { "max_turns": 4, "max_tool_calls": 2 }
        }
    }))
    .expect("the inherit-reach App envelope encodes")
}

/// The `(tool_id, tool_version)` pairs the manifest reports as reachable, sorted.
///
/// Pairs rather than ids so a role can be built from the SERVE's own answer: pinning a
/// version literal would make the intersection empty whenever the registered version
/// differed, and the test would then fail for a reason with nothing to do with policy.
async fn reachable_tools(c: &mut KxGatewayClient<Channel>) -> Vec<(String, String)> {
    let m = c
        .get_app_manifest(proto::GetAppManifestRequest {
            handle: APP_HANDLE.to_string(),
        })
        .await
        .expect("read the App manifest")
        .into_inner();
    assert!(m.found, "the App is saved and owned by this caller");
    assert!(
        m.reach_inherit,
        "the App must report reach_inherit — without it a policy role is unobservable and \
         every assertion below would pass over the App's own declared set instead"
    );
    let mut ids: Vec<(String, String)> = m
        .tools
        .iter()
        .filter(|t| t.in_policy)
        .map(|t| (t.id.clone(), t.version.clone()))
        .collect();
    ids.sort();
    ids
}

async fn put_role(c: &mut KxGatewayClient<Channel>, tools: &[(String, String)]) {
    c.put_policy_role(proto::PutPolicyRoleRequest {
        name: ROLE.to_string(),
        description: "the narrowed set".to_string(),
        tools: tools
            .iter()
            .map(|(id, v)| proto::PolicyRoleTool {
                tool_id: id.clone(),
                tool_version: v.clone(),
            })
            .collect(),
    })
    .await
    .expect("store the role");
}

async fn assign_role(c: &mut KxGatewayClient<Channel>, name: &str) {
    c.assign_policy_role(proto::AssignPolicyRoleRequest {
        party: PARTY.to_string(),
        name: name.to_string(),
    })
    .await
    .expect("assign the role");
}

async fn save_inherit_app(c: &mut KxGatewayClient<Channel>) {
    c.save_app(proto::SaveAppRequest {
        handle: APP_HANDLE.to_string(),
        envelope_json: inherit_app_envelope(),
        source_digest: Vec::new(),
    })
    .await
    .expect("save the inherit-reach App");
}

async fn delete_role(c: &mut KxGatewayClient<Channel>) -> bool {
    c.delete_policy_role(proto::DeletePolicyRoleRequest {
        name: ROLE.to_string(),
    })
    .await
    .expect("delete the role")
    .into_inner()
    .removed
}

// ---------------------------------------------------------------------------
// Arm 1 — the ordinary suite. No model, no non-default feature.
// ---------------------------------------------------------------------------

/// ★ A role that names NO tool refuses everything, and deleting it gives the authority
/// back. `http@1` registers under `mcp-gateway` with no model, so the un-narrowed ceiling
/// here is exactly one tool and the narrowing is one-to-none.
#[tokio::test(flavor = "multi_thread")]
async fn a_policy_role_narrows_a_model_free_serve() {
    // Pin the engine OFF. Unset means `auto`, which is ON when no GGUF is configured, so an
    // ambient Ollama daemon would otherwise hand this "model-free" arm a served model and a
    // four-tool ceiling — which is how the first run of this experiment misread its result.
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    save_inherit_app(&mut c).await;

    // The precondition, asserted rather than assumed: a narrowing over an EMPTY ceiling is
    // unobservable and every assertion below would pass vacuously.
    let unnarrowed = reachable_tools(&mut c).await;
    assert!(
        !unnarrowed.is_empty(),
        "the UN-NARROWED ceiling must hold at least one tool or this test proves nothing"
    );

    // A role naming nothing: the documented "refuses everything" case.
    put_role(&mut c, &[]).await;
    assign_role(&mut c, ROLE).await;
    assert!(
        reachable_tools(&mut c).await.is_empty(),
        "a role naming no tool intersects the ceiling to nothing"
    );

    // The ACCEPTING CONTROL, one variable changed back: deleting the role WIDENS every
    // party still assigned to it. Without this the assertion above would pass on any
    // implementation that simply reported an empty manifest.
    assert!(
        delete_role(&mut c).await,
        "the role existed and was removed"
    );
    assert_eq!(
        reachable_tools(&mut c).await,
        unnarrowed,
        "deleting an assigned role restores the un-narrowed authority"
    );
}

/// ★ The refusal, asserting its REASON. A role that does not exist must not be assignable:
/// a silent success would leave the operator believing a narrowing is in force while the
/// party is fully un-narrowed.
#[tokio::test(flavor = "multi_thread")]
async fn assigning_a_role_that_does_not_exist_is_refused_by_name() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let err = c
        .assign_policy_role(proto::AssignPolicyRoleRequest {
            party: PARTY.to_string(),
            name: "no-such-role".to_string(),
        })
        .await
        .expect_err("assigning an unknown role is refused");
    assert_eq!(
        err.code(),
        tonic::Code::NotFound,
        "the refusal names the missing role rather than failing generically; got {err:?}"
    );

    // The one-variable ACCEPTING control: the identical call against a role that DOES exist
    // succeeds. Without it the assertion above would pass on any failure at all — a broken
    // store, a bad party, an unwired registry.
    put_role(&mut c, &[("http".to_string(), "1".to_string())]).await;
    assign_role(&mut c, ROLE).await;

    // And the role is listed, with what it names.
    let roles = c
        .list_policy_roles(proto::ListPolicyRolesRequest { limit: 0 })
        .await
        .expect("list roles")
        .into_inner();
    let row = roles
        .roles
        .iter()
        .find(|r| r.name == ROLE)
        .expect("the stored role is listed");
    assert_eq!(row.tools.len(), 1, "the role names exactly the one tool");
}

// ---------------------------------------------------------------------------
// Arm 2 — the fuller scenario, against a served model.
// ---------------------------------------------------------------------------

/// ★ Many-to-one narrowing on a real serve. With a served model and a granted read root the
/// ceiling also holds `fs-list@1`, `fs-read@1` and `mcp-echo/echo@1`, so the operator keeps
/// ONE of several rather than none of one — the shape the scenario is really about.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a served model for the fs + echo capabilities to register; opt in with --ignored"]
async fn a_policy_role_narrows_what_an_inherit_reach_app_can_reach() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF to a real GGUF");
        return;
    };
    let dir = TempDir::new().unwrap();
    let fs_root = dir.path().join("granted");
    std::fs::create_dir_all(&fs_root).unwrap();
    std::fs::write(fs_root.join("note.txt"), b"hello").unwrap();
    // Process-global reads taken during `start`; this file is its own binary and the live
    // arm runs under --test-threads=1 (see the module header). Precedent: `rerank_serve`,
    // `app_scaffold_live_serve`.
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_FS_ROOT", &fs_root);

    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    save_inherit_app(&mut c).await;

    let unnarrowed = reachable_tools(&mut c).await;
    assert!(
        unnarrowed.len() >= 2,
        "the UN-NARROWED ceiling must hold at least two tools for a MANY-to-one narrowing \
         — got {unnarrowed:?}. The fs capabilities register only when a model is served AND \
         KX_SERVE_FS_ROOT resolves; if this is short the precondition vanished silently."
    );
    let keep = unnarrowed
        .iter()
        .find(|(id, _)| id.contains("fs-list"))
        .unwrap_or_else(|| panic!("fs-list is registered on this serve; got {unnarrowed:?}"))
        .clone();
    eprintln!("policy: un-narrowed ceiling = {unnarrowed:?}; keeping {keep:?}");

    put_role(&mut c, std::slice::from_ref(&keep)).await;
    assign_role(&mut c, ROLE).await;
    assert_eq!(
        reachable_tools(&mut c).await,
        vec![keep.clone()],
        "an assigned role INTERSECTS the ceiling — the manifest reports exactly the tool \
         the role names, and the run resolves the SAME ceiling this surface reports"
    );

    assert!(
        delete_role(&mut c).await,
        "the role existed and was removed"
    );
    assert_eq!(
        reachable_tools(&mut c).await,
        unnarrowed,
        "deleting an assigned role WIDENS every party still assigned back to its \
         un-narrowed authority — the accepting control for the narrowing above"
    );
}
