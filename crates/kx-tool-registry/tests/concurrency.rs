// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Concurrency tests for `kx-tool-registry` (SN-4 v2 #7).
//!
//! - Compile-time `Send + Sync` over the full public-type set.
//! - 4-thread thread-independence of `resolve` (Arc<InMemoryToolRegistry>;
//!   read-only after registration; identical outcomes).
//! - `dyn ToolRegistry` is object-safe AND `Send + Sync` — proves the trait
//!   shape admits a cloud impl behind `Arc<dyn ToolRegistry>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use kx_content::ContentRef;
use kx_mote::{ModelId, MoteId, ToolName, ToolVersion};
use kx_tool_registry::{
    registration_token_of, IdempotencyClass, InMemoryToolRegistry, McpEndpointId,
    RegistrationError, RegistrationStatus, RegistrationToken, ResolutionError, ResolvedTool,
    ReviewerId, ToolDef, ToolKind, ToolProvenance, ToolRegistry, ToolResolutionEvent,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, ToolRequirement, WarrantSpec,
};

// ---------------------------------------------------------------------------
// Compile-time Send + Sync
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    // Identifier newtypes
    assert_send_sync::<McpEndpointId>();
    assert_send_sync::<ReviewerId>();
    assert_send_sync::<RegistrationToken>();

    // Spec types
    assert_send_sync::<IdempotencyClass>(); // NEW per PR 4.6 (D38 §2)
    assert_send_sync::<ToolKind>();
    assert_send_sync::<ToolDef>();
    assert_send_sync::<ToolProvenance>();
    assert_send_sync::<RegistrationStatus>();
    assert_send_sync::<ResolvedTool>();
    assert_send_sync::<ToolResolutionEvent>();

    // Errors
    assert_send_sync::<ResolutionError>();
    assert_send_sync::<RegistrationError>();

    // Trait + impl
    assert_send_sync::<InMemoryToolRegistry>();
    assert_send_sync::<Arc<dyn ToolRegistry>>();
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence of `resolve`
// ---------------------------------------------------------------------------

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
            model_id: ModelId("m".into()),
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

#[test]
fn resolve_is_thread_independent() {
    // Build a registry with a single Builtin tool.
    let mut reg = InMemoryToolRegistry::new();
    let def = ToolDef {
        tool_id: ToolName("t".into()),
        tool_version: ToolVersion("1".into()),
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
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
        },
        description: "thread test".into(),
        idempotency_class: IdempotencyClass::Token,
    };
    let _ = reg
        .register(
            def.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();

    // Share read-only.
    let reg: Arc<dyn ToolRegistry> = Arc::new(reg);
    let warrant = Arc::new(permissive_warrant());
    let grant = Arc::new(ToolGrant {
        tool_id: def.tool_id.clone(),
        tool_version: def.tool_version.clone(),
    });

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let r = Arc::clone(&reg);
            let w = Arc::clone(&warrant);
            let g = Arc::clone(&grant);
            thread::spawn(move || r.resolve(&g, &w).expect("ok"))
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(
            first.event_ref, r.event_ref,
            "resolve.event_ref must be thread-independent"
        );
        assert_eq!(first.def, r.def, "resolve.def must be thread-independent");
    }
}

// ---------------------------------------------------------------------------
// `registration_token_of` is thread-independent (idempotent on identity)
// ---------------------------------------------------------------------------

#[test]
fn registration_token_of_is_thread_independent() {
    let def = ToolDef {
        tool_id: ToolName("tok".into()),
        tool_version: ToolVersion("1".into()),
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
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
        },
        description: "test".into(),
        idempotency_class: IdempotencyClass::Token,
    };
    let prov = ToolProvenance::SelfGenerated {
        generating_lineage_warrant: permissive_warrant(),
        generating_mote: MoteId([5; 32]),
    };
    let def = Arc::new(def);
    let prov = Arc::new(prov);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let d = Arc::clone(&def);
            let p = Arc::clone(&prov);
            thread::spawn(move || registration_token_of(&d, &p))
        })
        .collect();

    let mut tokens = Vec::with_capacity(4);
    for h in handles {
        tokens.push(h.join().expect("worker did not panic"));
    }
    let first = tokens[0];
    for t in &tokens[1..] {
        assert_eq!(&first, t, "registration_token must be thread-independent");
    }
}
