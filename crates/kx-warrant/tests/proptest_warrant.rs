// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `intersect`, `narrow`, `check_tool_requirement`, and
//! `warrant_ref_of` (SN-4 v2 #6 — pinned per D30).
//!
//! Properties:
//!
//! 1. `intersect(p, r)` is TOTAL — never panics on any arbitrary input pair.
//! 2. `intersect(p, r)` is DETERMINISTIC — same inputs → same outcome.
//! 3. `intersect(p, p_as_role)` is IDENTITY — intersecting a parent with a role
//!    whose spec equals the parent's returns Ok(parent).
//! 4. `warrant_ref_of(s)` is DETERMINISTIC and PURE — same spec → same ref.
//! 5. Quantitative-axis narrowing is correct: `result.resource_ceiling.X ==
//!    min(parent.X, child.X)` for every axis.
//! 6. Widening on a qualitative axis is REFUSED: if the child role proposes
//!    `tool_grants` not ⊆ parent's, intersect returns `AttemptedWiden`.
//! 7. The narrowing is MONOTONIC: if `child2.role` is itself a narrowing of
//!    `child1.warrant`, then `intersect(child1.warrant, child2_role)` produces
//!    a warrant whose quantitative axes are ≤ `child1.warrant`'s.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_warrant::{
    check_tool_requirement, intersect, narrow, warrant_ref_of, CostCeiling, ExecutorClass, FsMode,
    FsScope, Host, ModelRoute, MoteClass, NarrowingError, NetScope, ResourceCeiling, Role,
    SecretRef, SecretScope, ToolGrant, ToolRequirement, WarrantField, WarrantSpec,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_mote_class() -> impl Strategy<Value = MoteClass> {
    prop_oneof![
        Just(MoteClass::Pure),
        Just(MoteClass::ReadOnlyNondet),
        Just(MoteClass::WorldMutating),
    ]
}

// MUST update on new `ExecutorClass` variant. Canonical-classifier-cannot-drift
// pattern (PR 6 / kx-normalizer; STEP 6.2 of PR 4.5). The match in
// `kx_warrant::intersect`'s `executor_class` branch is exhaustive at the source
// level; this strategy enumerates ALL variants so any new addition that doesn't
// update this list is caught at the test surface. The exhaustive-match-at-source
// + ALL-variants-in-strategy combination makes silent variant addition
// impossible.
fn arb_executor_class() -> impl Strategy<Value = ExecutorClass> {
    prop_oneof![
        Just(ExecutorClass::Bwrap),
        Just(ExecutorClass::OciDaemon),
        Just(ExecutorClass::CloudMicroVm),
        Just(ExecutorClass::MacOsSandbox),
    ]
}

fn arb_fs_mode() -> impl Strategy<Value = FsMode> {
    prop_oneof![
        Just(FsMode::ReadOnly),
        Just(FsMode::ReadWrite),
        Just(FsMode::ExecOnly)
    ]
}

fn arb_path() -> impl Strategy<Value = PathBuf> {
    prop_oneof![
        Just(PathBuf::from("/input")),
        Just(PathBuf::from("/output")),
        Just(PathBuf::from("/tmp")),
        Just(PathBuf::from("/etc")),
        Just(PathBuf::from("/var/run")),
    ]
}

fn arb_fs_scope() -> impl Strategy<Value = FsScope> {
    proptest::collection::btree_map(arb_path(), arb_fs_mode(), 0..=4)
        .prop_map(|mounts| FsScope { mounts })
}

fn arb_host() -> impl Strategy<Value = Host> {
    prop_oneof![
        Just(Host("api.example.com:443".into())),
        Just(Host("registry.example.com:443".into())),
        Just(Host("storage.example.com:443".into())),
        Just(Host("evil.example.com:443".into())),
    ]
}

fn arb_net_scope() -> impl Strategy<Value = NetScope> {
    prop_oneof![
        Just(NetScope::None),
        proptest::collection::btree_set(arb_host(), 0..=3).prop_map(NetScope::EgressAllowlist),
    ]
}

fn arb_content_ref() -> impl Strategy<Value = ContentRef> {
    prop_oneof![
        Just(ContentRef::from_bytes([0; 32])),
        Just(ContentRef::from_bytes([1; 32])),
        Just(ContentRef::from_bytes([42; 32])),
    ]
}

fn arb_tool_grant() -> impl Strategy<Value = ToolGrant> {
    (
        prop_oneof![
            Just(ToolName("fs-read".into())),
            Just(ToolName("fs-write".into())),
            Just(ToolName("http-get".into())),
            Just(ToolName("text-summarize".into())),
        ],
        prop_oneof![
            Just(ToolVersion("1".into())),
            Just(ToolVersion("2".into())),
            Just(ToolVersion("3".into())),
        ],
    )
        .prop_map(|(tool_id, tool_version)| ToolGrant {
            tool_id,
            tool_version,
        })
}

fn arb_tool_grants() -> impl Strategy<Value = BTreeSet<ToolGrant>> {
    proptest::collection::btree_set(arb_tool_grant(), 0..=4)
}

fn arb_model_route() -> impl Strategy<Value = ModelRoute> {
    (
        prop_oneof![
            Just(ModelId("gpt-4".into())),
            Just(ModelId("claude-3.7".into())),
            Just(ModelId("llama-3-8b".into())),
        ],
        1u32..=128_000,
        1u32..=128_000,
        1u32..=1000,
    )
        .prop_map(
            |(model_id, max_input_tokens, max_output_tokens, max_calls)| ModelRoute {
                model_id,
                max_input_tokens,
                max_output_tokens,
                max_calls,
            },
        )
}

fn arb_resource_ceiling() -> impl Strategy<Value = ResourceCeiling> {
    (
        0u32..=100_000,
        0u64..=(64u64 << 30),
        0u64..=600_000,
        0u32..=4096,
        0u64..=(64u64 << 30),
    )
        .prop_map(
            |(cpu_milli, mem_bytes, wall_clock_ms, fd_count, disk_bytes)| ResourceCeiling {
                cpu_milli,
                mem_bytes,
                wall_clock_ms,
                fd_count,
                disk_bytes,
            },
        )
}

fn arb_secret_ref() -> impl Strategy<Value = SecretRef> {
    prop_oneof!["API_KEY", "DB_URL", "TOKEN", "SLACK_HOOK", "STRIPE_SK"]
        .prop_map(|s: String| SecretRef(s))
}

fn arb_secret_scope() -> impl Strategy<Value = SecretScope> {
    prop_oneof![
        Just(SecretScope::None),
        proptest::collection::btree_set(arb_secret_ref(), 0..4).prop_map(SecretScope::AllowList),
    ]
}

fn arb_cost_ceiling() -> impl Strategy<Value = CostCeiling> {
    (0u64..=10_000_000_000).prop_map(|micro_usd| CostCeiling { micro_usd })
}

fn arb_warrant_spec() -> impl Strategy<Value = WarrantSpec> {
    // Base 9-axis warrant (new M5.3 axes default-filled), then overlay the three
    // M5.3 axes (secret_scope / cost_ceiling / tls_required) so every property
    // exercises them. Two-stage compose keeps each tuple within proptest's arity.
    let base = (
        arb_mote_class(),
        arb_fs_scope(),
        arb_net_scope(),
        arb_content_ref(),
        arb_tool_grants(),
        arb_model_route(),
        arb_resource_ceiling(),
        proptest::option::of(arb_content_ref()),
        arb_executor_class(),
    )
        .prop_map(
            |(
                cls,
                fs_scope,
                net_scope,
                syscall_profile_ref,
                tool_grants,
                model_route,
                resource_ceiling,
                environment_ref,
                executor_class,
            )| WarrantSpec {
                mote_class: cls,
                nd_class: cls,
                fs_scope,
                net_scope,
                syscall_profile_ref,
                tool_grants,
                model_route,
                resource_ceiling,
                environment_ref,
                executor_class,
                ..Default::default()
            },
        );
    (base, arb_secret_scope(), arb_cost_ceiling(), any::<bool>()).prop_map(
        |(base, secret_scope, cost_ceiling, tls_required)| WarrantSpec {
            secret_scope,
            cost_ceiling,
            tls_required,
            ..base
        },
    )
}

fn arb_role(spec_strategy: impl Strategy<Value = WarrantSpec>) -> impl Strategy<Value = Role> {
    (prop_oneof!["[a-z]{3,8}"], 1u32..=100, spec_strategy).prop_map(|(name, version, spec)| Role {
        name,
        version,
        spec,
        description: String::new(),
    })
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1: `intersect` is TOTAL — never panics on any input pair.
    #[test]
    fn prop_intersect_is_total(
        parent in arb_warrant_spec(),
        role in arb_role(arb_warrant_spec()),
    ) {
        let _ = intersect(&parent, &role); // Reaching this line proves no panic.
    }

    /// Property 2: `intersect` is DETERMINISTIC — same inputs → same outcome.
    #[test]
    fn prop_intersect_is_deterministic(
        parent in arb_warrant_spec(),
        role in arb_role(arb_warrant_spec()),
    ) {
        let a = intersect(&parent, &role);
        let b = intersect(&parent, &role);
        prop_assert_eq!(a, b);
    }

    /// Property 3: `intersect(p, p_as_role) == Ok(p)` — identity property.
    /// A role whose spec equals the parent's must produce the parent back
    /// (modulo silent no-op narrowing which produces byte-identical fields).
    #[test]
    fn prop_intersect_self_is_identity(parent in arb_warrant_spec()) {
        let role = Role {
            name: "self".into(),
            version: 1,
            spec: parent.clone(),
            description: String::new(),
        };
        // Self-intersect succeeds AND produces byte-identical parent.
        let result = intersect(&parent, &role).expect("self-intersect always Ok");
        prop_assert_eq!(result, parent);
    }

    /// Property 4: `warrant_ref_of` is DETERMINISTIC — same spec → same ref.
    #[test]
    fn prop_warrant_ref_of_is_deterministic(spec in arb_warrant_spec()) {
        let a = warrant_ref_of(&spec);
        let b = warrant_ref_of(&spec);
        prop_assert_eq!(a, b);
    }

    /// Property 5: Quantitative-axis narrowing — `result.resource_ceiling.X ==
    /// min(parent.X, child.X)` for every axis.
    #[test]
    fn prop_resource_ceiling_narrows_to_min(
        parent in arb_warrant_spec(),
        child_ceiling in arb_resource_ceiling(),
    ) {
        // Build a child role that matches parent except resource_ceiling.
        let mut child_spec = parent.clone();
        child_spec.resource_ceiling = child_ceiling;
        // Also match the syscall_profile_ref so we don't trip that check.
        child_spec.syscall_profile_ref = parent.syscall_profile_ref;
        // Ensure model_route ceilings are valid (>0) to avoid InvalidModelRoute.
        // Already true by arb_model_route's 1.. bounds — preserved here.
        let role = Role {
            name: "rc-test".into(),
            version: 1,
            spec: child_spec.clone(),
            description: String::new(),
        };

        // Other axes (fs_scope, net_scope, tool_grants) inherited from parent
        // (identical to parent's), so widening rejection doesn't trip.
        let role = Role {
            spec: WarrantSpec {
                fs_scope: parent.fs_scope.clone(),
                net_scope: parent.net_scope.clone(),
                tool_grants: parent.tool_grants.clone(),
                ..role.spec
            },
            ..role
        };

        let result = intersect(&parent, &role);
        if let Ok(r) = result {
            prop_assert_eq!(
                r.resource_ceiling.cpu_milli,
                parent.resource_ceiling.cpu_milli.min(child_ceiling.cpu_milli)
            );
            prop_assert_eq!(
                r.resource_ceiling.mem_bytes,
                parent.resource_ceiling.mem_bytes.min(child_ceiling.mem_bytes)
            );
            prop_assert_eq!(
                r.resource_ceiling.wall_clock_ms,
                parent.resource_ceiling.wall_clock_ms.min(child_ceiling.wall_clock_ms)
            );
            prop_assert_eq!(
                r.resource_ceiling.fd_count,
                parent.resource_ceiling.fd_count.min(child_ceiling.fd_count)
            );
            prop_assert_eq!(
                r.resource_ceiling.disk_bytes,
                parent.resource_ceiling.disk_bytes.min(child_ceiling.disk_bytes)
            );
        }
    }

    /// Property 6: Widening tool_grants is REFUSED.
    /// If the child role's tool_grants is NOT a subset of parent's, intersect
    /// returns `AttemptedWiden { field: WarrantField::ToolGrants, .. }`.
    #[test]
    fn prop_tool_grants_widening_is_refused(
        parent in arb_warrant_spec(),
        extra_grant in arb_tool_grant(),
    ) {
        // Skip degenerate case where extra_grant is already in parent's set.
        if parent.tool_grants.contains(&extra_grant) {
            return Ok(());
        }
        let mut child_spec = parent.clone();
        let mut grants = parent.tool_grants.clone();
        grants.insert(extra_grant);
        child_spec.tool_grants = grants;
        let role = Role {
            name: "widening".into(),
            version: 1,
            spec: child_spec,
            description: String::new(),
        };

        match intersect(&parent, &role) {
            Err(NarrowingError::AttemptedWiden { field: WarrantField::ToolGrants, .. }) => {}
            other => prop_assert!(
                false,
                "expected ToolGrants AttemptedWiden, got {:?}", other
            ),
        }
    }

    /// Property 7: Two-level narrowing chains MONOTONICALLY.
    /// If `intersect(parent, role1) = Ok(child1)` and `intersect(child1, role2)
    /// = Ok(child2)`, then `child2.resource_ceiling.X ≤ child1.resource_ceiling.X`
    /// on every axis. (Monotonic narrowing across the chain.)
    #[test]
    fn prop_two_level_narrowing_is_monotonic(
        parent in arb_warrant_spec(),
        rc1 in arb_resource_ceiling(),
        rc2 in arb_resource_ceiling(),
    ) {
        // Build role1 that inherits everything except resource_ceiling.
        let mut spec1 = parent.clone();
        spec1.resource_ceiling = rc1;
        let role1 = Role { name: "l1".into(), version: 1, spec: spec1, description: String::new() };

        let child1 = match intersect(&parent, &role1) {
            Ok(c) => c,
            Err(_) => return Ok(()), // skip cases where role1 is rejected
        };

        // Build role2 inheriting from child1's qualitative shape with rc2.
        let mut spec2 = child1.clone();
        spec2.resource_ceiling = rc2;
        let role2 = Role { name: "l2".into(), version: 1, spec: spec2, description: String::new() };

        let child2 = match intersect(&child1, &role2) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        prop_assert!(child2.resource_ceiling.cpu_milli <= child1.resource_ceiling.cpu_milli);
        prop_assert!(child2.resource_ceiling.mem_bytes <= child1.resource_ceiling.mem_bytes);
        prop_assert!(child2.resource_ceiling.wall_clock_ms <= child1.resource_ceiling.wall_clock_ms);
        prop_assert!(child2.resource_ceiling.fd_count <= child1.resource_ceiling.fd_count);
        prop_assert!(child2.resource_ceiling.disk_bytes <= child1.resource_ceiling.disk_bytes);
    }

    /// Property 8 (D110.3): `cost_ceiling` narrows to the per-axis `min`.
    /// Mirrors property 5 for the new quantitative axis.
    #[test]
    fn prop_cost_ceiling_narrows_to_min(
        parent in arb_warrant_spec(),
        child_cost in arb_cost_ceiling(),
    ) {
        // Inherit every other axis from the parent so no widen trips; vary only cost.
        let mut child_spec = parent.clone();
        child_spec.cost_ceiling = child_cost;
        let role = Role { name: "cc".into(), version: 1, spec: child_spec, description: String::new() };
        if let Ok(r) = intersect(&parent, &role) {
            prop_assert_eq!(
                r.cost_ceiling.micro_usd,
                parent.cost_ceiling.micro_usd.min(child_cost.micro_usd)
            );
        }
    }

    /// Property 9 (D118.5): `tls_required` is TIGHTEN-ONLY (`parent || child`).
    /// A parent requiring TLS forces the result to require it; a relaxation is
    /// structurally impossible (never an error, always the safe value).
    #[test]
    fn prop_tls_required_tighten_only(
        parent in arb_warrant_spec(),
        child_tls in any::<bool>(),
    ) {
        let mut child_spec = parent.clone();
        child_spec.tls_required = child_tls;
        let role = Role { name: "tls".into(), version: 1, spec: child_spec, description: String::new() };
        if let Ok(r) = intersect(&parent, &role) {
            prop_assert_eq!(r.tls_required, parent.tls_required || child_tls);
            // Never relaxes: if the parent required TLS, the result requires it.
            if parent.tls_required {
                prop_assert!(r.tls_required);
            }
        }
    }

    /// Property 10 (D110.3): a `secret_scope` ⊆ the parent's NARROWS (Ok, value
    /// = child's). Mirrors the qualitative subset axes.
    #[test]
    fn prop_secret_scope_subset_narrows(parent in arb_warrant_spec()) {
        // A child whose secret_scope is `None` is always ⊆ any parent.
        let mut child_spec = parent.clone();
        child_spec.secret_scope = SecretScope::None;
        let role = Role { name: "ss".into(), version: 1, spec: child_spec, description: String::new() };
        if let Ok(r) = intersect(&parent, &role) {
            prop_assert_eq!(r.secret_scope, SecretScope::None);
        }
    }

    /// Property 11 (D110.3): WIDENING `secret_scope` is REFUSED with
    /// `AttemptedWiden { field: SecretScope }`. Mirrors property 6.
    #[test]
    fn prop_secret_scope_widening_is_refused(
        parent in arb_warrant_spec(),
        extra in arb_secret_ref(),
    ) {
        // Build a child scope that is a strict SUPERSET of the parent's (so it
        // can never be ⊆): parent's refs ∪ {extra}, with `extra` not already in.
        let mut child_set: std::collections::BTreeSet<SecretRef> = match &parent.secret_scope {
            SecretScope::None => std::collections::BTreeSet::new(),
            SecretScope::AllowList(s) => s.clone(),
        };
        if child_set.contains(&extra) {
            return Ok(()); // degenerate: already present, not a widen
        }
        child_set.insert(extra);
        let mut child_spec = parent.clone();
        child_spec.secret_scope = SecretScope::AllowList(child_set);
        let role = Role { name: "ssw".into(), version: 1, spec: child_spec, description: String::new() };
        match intersect(&parent, &role) {
            Err(NarrowingError::AttemptedWiden { field: WarrantField::SecretScope, .. }) => {}
            other => prop_assert!(false, "expected SecretScope AttemptedWiden, got {:?}", other),
        }
    }
}

// ---------------------------------------------------------------------------
// Spot-check: narrow == intersect alias
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, .. ProptestConfig::default() })]

    /// `narrow` is exactly `intersect`.
    #[test]
    fn narrow_is_alias_for_intersect(
        parent in arb_warrant_spec(),
        role in arb_role(arb_warrant_spec()),
    ) {
        prop_assert_eq!(intersect(&parent, &role), narrow(&parent, &role));
    }
}

// ---------------------------------------------------------------------------
// Spot-check: check_tool_requirement under permissive vs none warrants
// ---------------------------------------------------------------------------

#[test]
fn check_tool_requirement_smoke() {
    let warrant = WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/input"), FsMode::ReadOnly)]),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_calls: 1,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 500,
            mem_bytes: 1 << 28,
            wall_clock_ms: 5_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    };

    // Permissive req: ok.
    let ok_req = ToolRequirement {
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
    };
    assert!(check_tool_requirement(&ok_req, &warrant).is_ok());

    // Excessive memory req: rejected.
    let bad_req = ToolRequirement {
        min_resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 1 << 50,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        ..ok_req
    };
    assert!(check_tool_requirement(&bad_req, &warrant).is_err());
}
