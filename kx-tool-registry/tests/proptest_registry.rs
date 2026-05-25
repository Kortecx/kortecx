//! Property tests for `kx-tool-registry` (SN-4 v2 #6 — pinned per D32).
//!
//! Properties:
//!
//! 1. `lookup` is DETERMINISTIC — repeated calls return the same Option.
//! 2. `register` then `lookup` is CONSISTENT — after a HumanAuthored register,
//!    lookup returns Some(def); after a SelfGenerated register, lookup returns
//!    None (PendingHumanReview).
//! 3. `register` is IDEMPOTENT on identity — same `(def, provenance)` produces
//!    the same `RegistrationToken`.
//! 4. `resolve` is DETERMINISTIC — same `(grant, warrant)` against the same
//!    registry state returns the same outcome.
//! 5. `resolve` REFUSES on capability exceedance: tool with `mem_bytes`
//!    requirement greater than warrant's ceiling produces
//!    `CapabilityExceedsWarrant`.
//! 6. `ToolResolutionEvent::to_ref` is BYTE-DETERMINISTIC — same event → same
//!    `ContentRef`.
//! 7. The subset-check at resolve and the subset-check at approve are
//!    CONSISTENT — a `(req, warrant)` pair that fails resolve also fails
//!    `approve_registration` (and vice versa).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{ModelId, MoteId, ToolName, ToolVersion};
use kx_tool_registry::{
    registration_token_of, IdempotencyClass, InMemoryToolRegistry, McpEndpointId,
    RegistrationError, ResolutionError, ReviewerId, ToolDef, ToolKind, ToolProvenance,
    ToolRegistry, ToolResolutionEvent,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, ToolRequirement, WarrantField, WarrantSpec,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_tool_name() -> impl Strategy<Value = ToolName> {
    prop_oneof![
        Just(ToolName("fs-read".into())),
        Just(ToolName("fs-write".into())),
        Just(ToolName("http-get".into())),
        Just(ToolName("text-summarize".into())),
        Just(ToolName("custom-a".into())),
        Just(ToolName("custom-b".into())),
    ]
}

fn arb_tool_version() -> impl Strategy<Value = ToolVersion> {
    prop_oneof![
        Just(ToolVersion("1".into())),
        Just(ToolVersion("2".into())),
        Just(ToolVersion("3".into())),
    ]
}

fn arb_tool_kind() -> impl Strategy<Value = ToolKind> {
    prop_oneof![
        Just(ToolKind::Builtin),
        Just(ToolKind::LocalScript {
            script_ref: ContentRef::from_bytes([7; 32])
        }),
        Just(ToolKind::External {
            source_url: "https://example.com/tool".into()
        }),
        Just(ToolKind::Mcp {
            endpoint: McpEndpointId("mcp-endpoint-1".into()),
            remote_name: "remote".into(),
        }),
    ]
}

fn arb_mem_ceiling() -> impl Strategy<Value = u64> {
    0u64..=(8u64 << 30)
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

/// Strategy enumerating ALL `IdempotencyClass` variants. **MUST be updated
/// when a variant is added** — this strategy is the test surface's gate
/// against silent variant addition (per `journal-txn.md` STEP 6.2 + the
/// canonical-classifier-cannot-drift contract).
fn arb_idempotency_class() -> impl Strategy<Value = IdempotencyClass> {
    prop_oneof![
        Just(IdempotencyClass::Token),
        Just(IdempotencyClass::Readback),
        Just(IdempotencyClass::Staged),
        Just(IdempotencyClass::AtLeastOnce),
    ]
}

prop_compose! {
    fn arb_def()(
        name in arb_tool_name(),
        version in arb_tool_version(),
        kind in arb_tool_kind(),
        mem_req in arb_mem_ceiling(),
        idempotency_class in arb_idempotency_class(),
    ) -> ToolDef {
        let mut req = permissive_req();
        req.min_resource_ceiling.mem_bytes = mem_req;
        ToolDef {
            tool_id: name,
            tool_version: version,
            kind,
            required_capability: req,
            description: "arb tool".into(),
            idempotency_class,
        }
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1: `lookup` is DETERMINISTIC.
    #[test]
    fn prop_lookup_is_deterministic(def in arb_def()) {
        let mut reg = InMemoryToolRegistry::new();
        let _ = reg.register(
            def.clone(),
            ToolProvenance::HumanAuthored { author: "ops".into() },
        ).unwrap();
        let a = reg.lookup(&def.tool_id, &def.tool_version);
        let b = reg.lookup(&def.tool_id, &def.tool_version);
        prop_assert_eq!(a, b);
    }

    /// Property 2: HumanAuthored register → Some(def); SelfGenerated register → None.
    #[test]
    fn prop_register_then_lookup_is_consistent(def in arb_def()) {
        // HumanAuthored.
        {
            let mut reg = InMemoryToolRegistry::new();
            let _ = reg.register(
                def.clone(),
                ToolProvenance::HumanAuthored { author: "ops".into() },
            ).unwrap();
            prop_assert_eq!(reg.lookup(&def.tool_id, &def.tool_version), Some(def.clone()));
        }
        // SelfGenerated.
        {
            let mut reg = InMemoryToolRegistry::new();
            let _ = reg.register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: permissive_warrant(),
                    generating_mote: MoteId([3; 32]),
                },
            ).unwrap();
            prop_assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
        }
    }

    /// Property 3: token is IDEMPOTENT on identity.
    #[test]
    fn prop_token_is_deterministic(def in arb_def()) {
        let prov = ToolProvenance::HumanAuthored { author: "ops".into() };
        let t1 = registration_token_of(&def, &prov);
        let t2 = registration_token_of(&def, &prov);
        prop_assert_eq!(t1, t2);
    }

    /// Property 4: `resolve` is DETERMINISTIC.
    #[test]
    fn prop_resolve_is_deterministic(def in arb_def()) {
        let mut reg = InMemoryToolRegistry::new();
        let _ = reg.register(
            def.clone(),
            ToolProvenance::HumanAuthored { author: "ops".into() },
        ).unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id.clone(),
            tool_version: def.tool_version.clone(),
        };
        let warrant = permissive_warrant();
        let a = reg.resolve(&grant, &warrant);
        let b = reg.resolve(&grant, &warrant);
        prop_assert_eq!(a, b);
    }

    /// Property 5: `resolve` REFUSES on memory ceiling exceedance.
    #[test]
    fn prop_resolve_refuses_when_mem_req_exceeds_warrant(
        mem_req in (5u64 << 30)..=(8u64 << 30),
    ) {
        // mem_req in [5 GiB, 8 GiB], strictly greater than the 4 GiB warrant.
        let mut def = ToolDef {
            tool_id: ToolName("big".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: permissive_req(),
            description: "needs lots of memory".into(),
            idempotency_class: IdempotencyClass::Token,
        };
        def.required_capability.min_resource_ceiling.mem_bytes = mem_req;
        let mut reg = InMemoryToolRegistry::new();
        let _ = reg.register(
            def.clone(),
            ToolProvenance::HumanAuthored { author: "ops".into() },
        ).unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id,
            tool_version: def.tool_version,
        };
        let outcome = reg.resolve(&grant, &permissive_warrant());
        let is_correct = matches!(
            outcome,
            Err(ResolutionError::CapabilityExceedsWarrant {
                axis: WarrantField::ResourceCeiling
            })
        );
        prop_assert!(is_correct, "expected CapabilityExceedsWarrant ResourceCeiling");
    }

    /// Property 6: `ToolResolutionEvent::to_ref` is BYTE-DETERMINISTIC.
    #[test]
    fn prop_event_to_ref_is_deterministic(
        name in arb_tool_name(),
        version in arb_tool_version(),
        kind in arb_tool_kind(),
    ) {
        let event = ToolResolutionEvent {
            tool_id: name,
            tool_version: version,
            resolved_kind: kind,
            resolved_def_hash: ContentRef::from_bytes([42; 32]),
        };
        prop_assert_eq!(event.to_ref(), event.to_ref());
    }

    /// Property 7: subset check at resolve == subset check at approve.
    /// Same `(req, warrant)`: if resolve refuses with CapabilityExceedsWarrant,
    /// approve refuses with InvalidLineageSubset on the same axis (modulo
    /// renaming).
    #[test]
    fn prop_resolve_and_approve_subset_checks_agree(mem_req in arb_mem_ceiling()) {
        let mut def = ToolDef {
            tool_id: ToolName("agree".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: permissive_req(),
            description: "subset-check parity".into(),
            idempotency_class: IdempotencyClass::Token,
        };
        def.required_capability.min_resource_ceiling.mem_bytes = mem_req;
        let warrant = permissive_warrant();

        // (a) Resolve path: register as HumanAuthored + resolve.
        let resolve_ok = {
            let mut reg = InMemoryToolRegistry::new();
            let _ = reg.register(
                def.clone(),
                ToolProvenance::HumanAuthored { author: "ops".into() },
            ).unwrap();
            let grant = ToolGrant {
                tool_id: def.tool_id.clone(),
                tool_version: def.tool_version.clone(),
            };
            reg.resolve(&grant, &warrant).is_ok()
        };

        // (b) Approve path: register as SelfGenerated under warrant + approve.
        let approve_ok = {
            let mut reg = InMemoryToolRegistry::new();
            let token = reg.register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: warrant.clone(),
                    generating_mote: MoteId([0; 32]),
                },
            ).unwrap();
            reg.approve_registration(token, ReviewerId("alice".into())).is_ok()
        };

        // Both paths apply the same `check_tool_requirement` against the same
        // warrant → outcomes must agree.
        prop_assert_eq!(resolve_ok, approve_ok);
    }
}

// ---------------------------------------------------------------------------
// Smoke: full self-gen lifecycle
// ---------------------------------------------------------------------------

#[test]
fn self_gen_lifecycle_smoke() {
    let mut reg = InMemoryToolRegistry::new();
    let def = ToolDef {
        tool_id: ToolName("emit-script".into()),
        tool_version: ToolVersion("1".into()),
        kind: ToolKind::Builtin,
        required_capability: permissive_req(),
        description: "self-emitted".into(),
        idempotency_class: IdempotencyClass::Token,
    };
    let token = reg
        .register(
            def.clone(),
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant: permissive_warrant(),
                generating_mote: MoteId([9; 32]),
            },
        )
        .unwrap();

    // 1. INERT — lookup returns None.
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());

    // 2. resolve refuses with PendingHumanReview.
    let grant = ToolGrant {
        tool_id: def.tool_id.clone(),
        tool_version: def.tool_version.clone(),
    };
    assert!(matches!(
        reg.resolve(&grant, &permissive_warrant()),
        Err(ResolutionError::PendingHumanReview { .. })
    ));

    // 3. approve succeeds (req ⊆ lineage).
    reg.approve_registration(token, ReviewerId("alice".into()))
        .unwrap();

    // 4. lookup + resolve now succeed.
    assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
    assert!(reg.resolve(&grant, &permissive_warrant()).is_ok());

    // 5. duplicate approve → AlreadyApproved.
    assert!(matches!(
        reg.approve_registration(token, ReviewerId("bob".into())),
        Err(RegistrationError::AlreadyApproved { .. })
    ));
}
