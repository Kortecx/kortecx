//! PR-6b-4 — the react-auto live-warrant-rebuild bind override, model-free.
//!
//! A bind of `kx/recipes/react-auto` admits a SERVER-built UNION warrant rebuilt
//! from the LIVE registry (admit-direct), so the autonomous loop can fire ANY
//! registered/dialed tool — not just one bundled seed tool. This test pins the
//! bind layer WITHOUT a live model (seeding only records a model id; bind does no
//! inference): the union grants the live set, the seed Mote's IDENTITY is
//! unchanged by the warrant override, `react_seed` is set, and a party without a
//! `Use` grant is refused. The full live drive lives in `react_auto_serve.rs`
//! (`#[ignore]`, real GGUF).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use kx_gateway::{DemoLibrary, HostRecipeBinder, REACT_AUTO_RECIPE_HANDLE};
use kx_gateway_core::{RecipeBinder, RegisteredToolsView};
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_tool_registry::{
    IdempotencyClass, SqliteToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{ExecutorClass, FsScope, Host, NetScope, ResourceCeiling, ToolRequirement};

/// A stub broker-fireable view returning a fixed `(id, version)` set.
struct StubRegistered(BTreeSet<(String, String)>);
impl RegisteredToolsView for StubRegistered {
    fn registered_grants(&self) -> BTreeSet<(String, String)> {
        self.0.clone()
    }
}

fn mcp_tool(name: &str, net: NetScope) -> ToolDef {
    ToolDef {
        tool_id: ToolName(name.into()),
        tool_version: ToolVersion("1".into()),
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
            net_scope_required: net,
            fs_scope_required: FsScope::empty(),
            syscall_profile_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: String::new(),
        idempotency_class: IdempotencyClass::Staged,
        input_schema: None,
    }
}

const ARGS: &[u8] = br#"{"instruction":"list files","max_turns":4,"max_tool_calls":2}"#;

/// Build a react-auto-seeded library + a registry holding `echo` + a dialed
/// egress tool, plus a view that reports both as broker-fireable.
fn fixture(
    dir: &std::path::Path,
) -> (
    Arc<DemoLibrary>,
    Arc<dyn ToolRegistry>,
    Arc<dyn RegisteredToolsView>,
) {
    let lib = Arc::new(
        DemoLibrary::open_complete(
            dir,
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            Some(&ModelId("kx-serve:test".into())),
            None,
            false,
            None,
            true, // autogrant ON ⇒ react-auto seeded
        )
        .unwrap(),
    );
    let registry = Arc::new(SqliteToolRegistry::open(dir.join("tools.db")).unwrap());
    registry
        .register_durable(
            mcp_tool("mcp-echo", NetScope::None),
            ToolProvenance::HumanAuthored {
                author: "test".into(),
            },
            None,
        )
        .unwrap();
    registry
        .register_durable(
            mcp_tool(
                "github/search",
                NetScope::EgressAllowlist([Host("api.github.com".into())].into_iter().collect()),
            ),
            ToolProvenance::HumanAuthored {
                author: "mcp-gateway:github".into(),
            },
            Some("api.github.com".into()),
        )
        .unwrap();
    let view: Arc<dyn RegisteredToolsView> = Arc::new(StubRegistered(
        [
            ("mcp-echo".to_string(), "1".to_string()),
            ("github/search".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect(),
    ));
    (lib, registry, view)
}

#[tokio::test]
async fn react_auto_bind_admits_the_live_union_warrant() {
    let dir = tempfile::tempdir().unwrap();
    let (lib, registry, view) = fixture(dir.path());
    let binder = HostRecipeBinder::from_shared_with_autogrant(lib, registry, view);

    let bound = binder
        .bind("alice@acme", REACT_AUTO_RECIPE_HANDLE, ARGS, &[], &[])
        .await
        .expect("alice holds Use on react-auto");

    // react-auto seeds a live chain.
    assert!(bound.react_seed, "react-auto must submit with react_seed");
    let (_mote, warrant) = &bound.motes[0];
    // The bound warrant auto-grants BOTH live tools (the union), not one seed tool.
    let granted: BTreeSet<String> = warrant
        .tool_grants
        .iter()
        .map(|g| g.tool_id.0.clone())
        .collect();
    assert!(granted.contains("mcp-echo"), "echo auto-granted");
    assert!(
        granted.contains("github/search"),
        "dialed tool auto-granted"
    );
    // The egress scope is the UNION (the dialed tool's host is reachable).
    match &warrant.net_scope {
        NetScope::EgressAllowlist(hosts) => {
            assert!(hosts.contains(&Host("api.github.com".into())));
        }
        NetScope::None => panic!("the union must convey the dialed tool's egress"),
    }
}

#[tokio::test]
async fn react_auto_override_preserves_the_seed_mote_identity() {
    let dir = tempfile::tempdir().unwrap();
    let (lib, registry, view) = fixture(dir.path());

    // WITHOUT autogrant: the placeholder warrant (empty tool_grants) binds.
    let plain = HostRecipeBinder::from_shared(lib.clone());
    let bound_plain = plain
        .bind("alice@acme", REACT_AUTO_RECIPE_HANDLE, ARGS, &[], &[])
        .await
        .unwrap();
    // WITH autogrant: the union warrant overrides.
    let auto = HostRecipeBinder::from_shared_with_autogrant(lib, registry, view);
    let bound_auto = auto
        .bind("alice@acme", REACT_AUTO_RECIPE_HANDLE, ARGS, &[], &[])
        .await
        .unwrap();

    // MoteId is warrant-INDEPENDENT (derived from the def/input/position), so the
    // override changes the warrant but never the Mote identity — the run-salted
    // seed-swap + durable anchor are unaffected.
    assert_eq!(
        bound_plain.motes[0].0.id, bound_auto.motes[0].0.id,
        "overriding the warrant must not change the seed Mote identity"
    );
    // The warrants DO differ (placeholder empty grants vs the live union).
    assert!(bound_plain.motes[0].1.tool_grants.is_empty());
    assert_eq!(bound_auto.motes[0].1.tool_grants.len(), 2);
}

#[tokio::test]
async fn react_auto_refuses_a_party_without_use() {
    let dir = tempfile::tempdir().unwrap();
    let (lib, registry, view) = fixture(dir.path());
    let binder = HostRecipeBinder::from_shared_with_autogrant(lib, registry, view);

    // The pre-override `bind_snapshot` gate still fires: a party with no `Use`
    // grant on react-auto is refused (the override never widens authorization).
    let outcome = binder
        .bind("mallory@evil", REACT_AUTO_RECIPE_HANDLE, ARGS, &[], &[])
        .await;
    assert!(
        matches!(outcome, Err(kx_gateway_core::BinderError::NotAuthorized)),
        "unauthorized party ⇒ NotAuthorized (no existence oracle)"
    );
}
