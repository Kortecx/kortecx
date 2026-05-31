//! Inline unit tests. Extracted from the original `lib.rs` per Rule 3 with
//! bodies unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{MoteId, ToolName, ToolVersion};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, ToolRequirement, WarrantField, WarrantSpec,
};

use super::*;

fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::from([
                (PathBuf::from("/input"), FsMode::ReadOnly),
                (PathBuf::from("/output"), FsMode::ReadWrite),
            ]),
        },
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())])),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: kx_mote::ModelId("m".into()),
            max_input_tokens: 8000,
            max_output_tokens: 2000,
            max_calls: 10,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 2000,
            mem_bytes: 4 << 30,
            wall_clock_ms: 60_000,
            fd_count: 256,
            disk_bytes: 4 << 30,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

fn sample_def(id: &str, version: &str, kind: ToolKind, req: ToolRequirement) -> ToolDef {
    // Default test-fixture class is `Token` (most permissive WM dispatch
    // path; any of the 4 variants would work for unit tests not exercising
    // the executor's protocol-selection branches — those tests live in
    // kx-executor at PR 9). Tests that DO want to exercise a specific
    // class construct ToolDef literally rather than via this helper.
    ToolDef {
        tool_id: ToolName(id.into()),
        tool_version: ToolVersion(version.into()),
        kind,
        required_capability: req,
        description: format!("test tool {id}@{version}"),
        idempotency_class: IdempotencyClass::Token,
    }
}

fn permissive_req() -> ToolRequirement {
    ToolRequirement {
        net_scope_required: NetScope::None,
        fs_scope_required: FsScope::empty(),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        min_resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
    }
}

// -----------------------------------------------------------------
// lookup
// -----------------------------------------------------------------

#[test]
fn lookup_returns_none_on_empty_registry() {
    let reg = InMemoryToolRegistry::new();
    assert!(reg
        .lookup(&ToolName("nope".into()), &ToolVersion("1".into()))
        .is_none());
}

#[test]
fn lookup_returns_some_after_human_register() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("t", "1", ToolKind::Builtin, permissive_req());
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    assert_eq!(reg.lookup(&def.tool_id, &def.tool_version), Some(def));
}

#[test]
fn lookup_returns_none_during_pending_human_review() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("self-gen", "1", ToolKind::Builtin, permissive_req());
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant: permissive_warrant(),
                generating_mote: MoteId([0; 32]),
            },
        )
        .unwrap();
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
}

// -----------------------------------------------------------------
// resolve — happy path
// -----------------------------------------------------------------

#[test]
fn resolve_succeeds_under_permissive_warrant() {
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg
        .register(
            sample_def("custom", "1", ToolKind::Builtin, permissive_req()),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    let grant = ToolGrant {
        tool_id: ToolName("custom".into()),
        tool_version: ToolVersion("1".into()),
    };
    let resolved = reg.resolve(&grant, &permissive_warrant()).unwrap();
    assert_eq!(resolved.def.tool_id.0, "custom");
    assert_eq!(resolved.event.tool_id.0, "custom");
    // event_ref == event.to_ref() — sanity.
    assert_eq!(resolved.event_ref, resolved.event.to_ref());
}

#[test]
fn resolve_event_is_deterministic() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("det", "1", ToolKind::Builtin, permissive_req());
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    let grant = ToolGrant {
        tool_id: def.tool_id.clone(),
        tool_version: def.tool_version.clone(),
    };
    let a = reg.resolve(&grant, &permissive_warrant()).unwrap();
    let b = reg.resolve(&grant, &permissive_warrant()).unwrap();
    assert_eq!(a.event_ref, b.event_ref);
}

// -----------------------------------------------------------------
// resolve_run_versions (M1.2, D79)
// -----------------------------------------------------------------

#[test]
fn resolve_run_versions_orders_by_grant_and_is_empty_for_zero_grants() {
    let mut reg = InMemoryToolRegistry::new();
    for id in ["beta", "alpha"] {
        reg.register(
            sample_def(id, "1", ToolKind::Builtin, permissive_req()),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    }
    let mut warrant = permissive_warrant();
    warrant.tool_grants = BTreeSet::from([
        ToolGrant {
            tool_id: ToolName("beta".into()),
            tool_version: ToolVersion("1".into()),
        },
        ToolGrant {
            tool_id: ToolName("alpha".into()),
            tool_version: ToolVersion("1".into()),
        },
    ]);
    let events = resolve_run_versions(&reg, &warrant).unwrap();
    // BTreeSet iteration is canonical (tool_id, tool_version) order → alpha, beta.
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].tool_id.0, "alpha");
    assert_eq!(events[1].tool_id.0, "beta");

    // Zero-grant warrant → empty Vec (no metadata to capture).
    let mut empty = permissive_warrant();
    empty.tool_grants = BTreeSet::new();
    assert!(resolve_run_versions(&reg, &empty).unwrap().is_empty());
}

#[test]
fn resolve_run_versions_propagates_resolution_error() {
    // A grant for a tool that does not resolve cleanly fails the whole capture
    // (fail-closed: no partial/over-privileged metadata is ever journaled).
    let reg = InMemoryToolRegistry::new();
    let mut warrant = permissive_warrant();
    warrant.tool_grants = BTreeSet::from([ToolGrant {
        tool_id: ToolName("missing".into()),
        tool_version: ToolVersion("1".into()),
    }]);
    assert!(matches!(
        resolve_run_versions(&reg, &warrant),
        Err(ResolutionError::NotFound { .. })
    ));
}

// -----------------------------------------------------------------
// resolve — refusals
// -----------------------------------------------------------------

#[test]
fn resolve_not_found() {
    let reg = InMemoryToolRegistry::new();
    let grant = ToolGrant {
        tool_id: ToolName("missing".into()),
        tool_version: ToolVersion("1".into()),
    };
    assert!(matches!(
        reg.resolve(&grant, &permissive_warrant()),
        Err(ResolutionError::NotFound { .. })
    ));
}

#[test]
fn resolve_pending_review_refused() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("pending", "1", ToolKind::Builtin, permissive_req());
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant: permissive_warrant(),
                generating_mote: MoteId([0; 32]),
            },
        )
        .unwrap();
    let grant = ToolGrant {
        tool_id: def.tool_id,
        tool_version: def.tool_version,
    };
    assert!(matches!(
        reg.resolve(&grant, &permissive_warrant()),
        Err(ResolutionError::PendingHumanReview { .. })
    ));
}

#[test]
fn resolve_capability_exceeds_warrant_on_fs_scope() {
    let mut reg = InMemoryToolRegistry::new();
    let mut req = permissive_req();
    // Tool requires /etc read, but the warrant doesn't grant it.
    req.fs_scope_required = FsScope {
        mounts: BTreeMap::from([(PathBuf::from("/etc"), FsMode::ReadOnly)]),
    };
    let def = sample_def("fs-overreach", "1", ToolKind::Builtin, req);
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    let grant = ToolGrant {
        tool_id: def.tool_id,
        tool_version: def.tool_version,
    };
    assert!(matches!(
        reg.resolve(&grant, &permissive_warrant()),
        Err(ResolutionError::CapabilityExceedsWarrant {
            axis: WarrantField::FsScope
        })
    ));
}

#[test]
fn resolve_mcp_under_none_egress_refused() {
    // A warrant with net_scope = None.
    let mut warrant = permissive_warrant();
    warrant.net_scope = NetScope::None;

    let mut reg = InMemoryToolRegistry::new();
    let mut req = permissive_req();
    req.net_scope_required =
        NetScope::EgressAllowlist(BTreeSet::from([Host("mcp.example.com:443".into())]));
    let def = sample_def(
        "mcp-tool",
        "1",
        ToolKind::Mcp {
            endpoint: McpEndpointId("mcp-endpoint-1".into()),
            remote_name: "summarize".into(),
        },
        req,
    );
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    let grant = ToolGrant {
        tool_id: def.tool_id,
        tool_version: def.tool_version,
    };
    assert!(matches!(
        reg.resolve(&grant, &warrant),
        Err(ResolutionError::CapabilityExceedsWarrant {
            axis: WarrantField::NetScope
        })
    ));
}

#[test]
fn resolve_mcp_with_matching_egress_succeeds() {
    let mut warrant = permissive_warrant();
    warrant.net_scope =
        NetScope::EgressAllowlist(BTreeSet::from([Host("mcp.example.com:443".into())]));

    let mut reg = InMemoryToolRegistry::new();
    let mut req = permissive_req();
    req.net_scope_required =
        NetScope::EgressAllowlist(BTreeSet::from([Host("mcp.example.com:443".into())]));
    let def = sample_def(
        "mcp-tool",
        "1",
        ToolKind::Mcp {
            endpoint: McpEndpointId("mcp-endpoint-1".into()),
            remote_name: "summarize".into(),
        },
        req,
    );
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    let grant = ToolGrant {
        tool_id: def.tool_id,
        tool_version: def.tool_version,
    };
    assert!(reg.resolve(&grant, &warrant).is_ok());
}

// -----------------------------------------------------------------
// register: provenance routing
// -----------------------------------------------------------------

#[test]
fn register_human_is_approved_immediately() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("h", "1", ToolKind::Builtin, permissive_req());
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
}

#[test]
fn register_self_gen_is_pending() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("s", "1", ToolKind::Builtin, permissive_req());
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant: permissive_warrant(),
                generating_mote: MoteId([0; 32]),
            },
        )
        .unwrap();
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
}

#[test]
fn register_token_is_deterministic() {
    let mut reg1 = InMemoryToolRegistry::new();
    let mut reg2 = InMemoryToolRegistry::new();
    let def = sample_def("d", "1", ToolKind::Builtin, permissive_req());
    let prov = ToolProvenance::HumanAuthored {
        author: "ops".into(),
    };
    let t1 = reg1.register(def.clone(), prov.clone()).unwrap();
    let t2 = reg2.register(def, prov).unwrap();
    assert_eq!(t1, t2);
}

// -----------------------------------------------------------------
// approve_registration: subset check + status flips
// -----------------------------------------------------------------

#[test]
fn approve_self_gen_within_lineage_ok() {
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("g", "1", ToolKind::Builtin, permissive_req());
    let token = reg
        .register(
            def.clone(),
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant: permissive_warrant(),
                generating_mote: MoteId([1; 32]),
            },
        )
        .unwrap();
    // Permissive req fits inside permissive warrant.
    reg.approve_registration(token, ReviewerId("alice".into()))
        .unwrap();
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
}

#[test]
fn approve_self_gen_exceeding_lineage_refused() {
    let mut reg = InMemoryToolRegistry::new();
    let mut req = permissive_req();
    req.min_resource_ceiling.mem_bytes = 1 << 50; // wider than lineage warrant
    let def = sample_def("greedy", "1", ToolKind::Builtin, req);
    let token = reg
        .register(
            def.clone(),
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant: permissive_warrant(),
                generating_mote: MoteId([2; 32]),
            },
        )
        .unwrap();
    assert!(matches!(
        reg.approve_registration(token, ReviewerId("alice".into())),
        Err(RegistrationError::InvalidLineageSubset {
            axis: WarrantField::ResourceCeiling
        })
    ));
    // Still INERT after refused approval.
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
}

#[test]
fn approve_unknown_token_refused() {
    let mut reg = InMemoryToolRegistry::new();
    let fake = RegistrationToken(ContentRef::from_bytes([0; 32]));
    assert!(matches!(
        reg.approve_registration(fake, ReviewerId("alice".into())),
        Err(RegistrationError::UnknownToken { .. })
    ));
}

#[test]
fn approve_human_authored_refused_as_not_pending() {
    // A HumanAuthored registration is Approved on register; the registry
    // returns AlreadyApproved if the operator calls approve on its token.
    let mut reg = InMemoryToolRegistry::new();
    let def = sample_def("h", "1", ToolKind::Builtin, permissive_req());
    let prov = ToolProvenance::HumanAuthored {
        author: "ops".into(),
    };
    let token = reg.register(def, prov).unwrap();
    assert!(matches!(
        reg.approve_registration(token, ReviewerId("alice".into())),
        Err(RegistrationError::AlreadyApproved { .. })
    ));
}

// -----------------------------------------------------------------
// with_builtins
// -----------------------------------------------------------------

#[test]
fn with_builtins_seeds_three_tools() {
    let reg = InMemoryToolRegistry::with_builtins();
    assert_eq!(reg.len(), 3);
    assert!(!reg.is_empty());
    assert!(reg
        .lookup(&ToolName("fs-read".into()), &ToolVersion("1".into()))
        .is_some());
    assert!(reg
        .lookup(&ToolName("fs-write".into()), &ToolVersion("1".into()))
        .is_some());
    assert!(reg
        .lookup(&ToolName("text-summarize".into()), &ToolVersion("1".into()))
        .is_some());
}

// -----------------------------------------------------------------
// ToolResolutionEvent::to_ref deterministic + sensitive to changes
// -----------------------------------------------------------------

#[test]
fn tool_resolution_event_ref_is_deterministic() {
    let event = ToolResolutionEvent {
        tool_id: ToolName("t".into()),
        tool_version: ToolVersion("1".into()),
        resolved_kind: ToolKind::Builtin,
        resolved_def_hash: ContentRef::from_bytes([0; 32]),
    };
    assert_eq!(event.to_ref(), event.to_ref());
}

#[test]
fn tool_resolution_event_ref_changes_with_kind() {
    let e1 = ToolResolutionEvent {
        tool_id: ToolName("t".into()),
        tool_version: ToolVersion("1".into()),
        resolved_kind: ToolKind::Builtin,
        resolved_def_hash: ContentRef::from_bytes([0; 32]),
    };
    let e2 = ToolResolutionEvent {
        tool_id: ToolName("t".into()),
        tool_version: ToolVersion("1".into()),
        resolved_kind: ToolKind::External {
            source_url: "https://example.com/t".into(),
        },
        resolved_def_hash: ContentRef::from_bytes([0; 32]),
    };
    assert_ne!(e1.to_ref(), e2.to_ref());
}
