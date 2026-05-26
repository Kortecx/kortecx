//! Inline unit tests. Per-axis truth tables + edge cases for the warrant
//! narrowing surface. Extracted from the original `lib.rs` per Rule 3 with
//! bodies unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{ModelId, ToolName, ToolVersion};

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
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([
            Host("api.example.com:443".into()),
            Host("registry.example.com:443".into()),
        ])),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::from([
            ToolGrant {
                tool_id: ToolName("fs-read".into()),
                tool_version: ToolVersion("1".into()),
            },
            ToolGrant {
                tool_id: ToolName("http-get".into()),
                tool_version: ToolVersion("2".into()),
            },
        ]),
        model_route: ModelRoute {
            model_id: ModelId("gpt-4".into()),
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

fn role_with(spec: WarrantSpec) -> Role {
    Role {
        name: "child".into(),
        version: 1,
        spec,
        description: String::new(),
    }
}

// -----------------------------------------------------------------
// Identity: intersect with self == self (modulo silent-narrow noop)
// -----------------------------------------------------------------

#[test]
fn intersect_with_self_is_identity() {
    let parent = permissive_warrant();
    let role = role_with(parent.clone());
    let result = intersect(&parent, &role).expect("self-intersect is always Ok");
    assert_eq!(result, parent);
}

// -----------------------------------------------------------------
// fs_scope: per-axis truth table
// -----------------------------------------------------------------

#[test]
fn fs_scope_subset_path_subset_mode_ok() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.fs_scope = FsScope {
        mounts: BTreeMap::from([(PathBuf::from("/input"), FsMode::ReadOnly)]),
    };
    let result = intersect(&parent, &role_with(child_spec.clone())).unwrap();
    assert_eq!(result.fs_scope, child_spec.fs_scope);
}

#[test]
fn fs_scope_unknown_path_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.fs_scope = FsScope {
        mounts: BTreeMap::from([(PathBuf::from("/etc"), FsMode::ReadOnly)]),
    };
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::AttemptedWiden {
            field: WarrantField::FsScope,
            ..
        }
    ));
}

#[test]
fn fs_scope_mode_widen_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.fs_scope = FsScope {
        mounts: BTreeMap::from([(PathBuf::from("/input"), FsMode::ReadWrite)]),
    };
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::AttemptedWiden {
            field: WarrantField::FsScope,
            ..
        }
    ));
}

// -----------------------------------------------------------------
// net_scope: per-axis truth table
// -----------------------------------------------------------------

#[test]
fn net_scope_none_under_none_ok() {
    let mut parent = permissive_warrant();
    parent.net_scope = NetScope::None;
    let mut child_spec = parent.clone();
    child_spec.net_scope = NetScope::None;
    assert!(intersect(&parent, &role_with(child_spec)).is_ok());
}

#[test]
fn net_scope_allowlist_under_none_rejected() {
    let mut parent = permissive_warrant();
    parent.net_scope = NetScope::None;
    let mut child_spec = parent.clone();
    child_spec.net_scope = NetScope::EgressAllowlist(BTreeSet::from([Host("h:1".into())]));
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::AttemptedWiden {
            field: WarrantField::NetScope,
            ..
        }
    ));
}

#[test]
fn net_scope_allowlist_subset_ok() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.net_scope =
        NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())]));
    assert!(intersect(&parent, &role_with(child_spec)).is_ok());
}

#[test]
fn net_scope_allowlist_extra_host_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.net_scope = NetScope::EgressAllowlist(BTreeSet::from([
        Host("api.example.com:443".into()),
        Host("evil.example.com:443".into()),
    ]));
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::AttemptedWiden {
            field: WarrantField::NetScope,
            ..
        }
    ));
}

// -----------------------------------------------------------------
// tool_grants: set subset
// -----------------------------------------------------------------

#[test]
fn tool_grants_subset_ok() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.tool_grants = BTreeSet::from([ToolGrant {
        tool_id: ToolName("fs-read".into()),
        tool_version: ToolVersion("1".into()),
    }]);
    assert!(intersect(&parent, &role_with(child_spec)).is_ok());
}

#[test]
fn tool_grants_unknown_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.tool_grants = BTreeSet::from([ToolGrant {
        tool_id: ToolName("evil-tool".into()),
        tool_version: ToolVersion("1".into()),
    }]);
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::AttemptedWiden {
            field: WarrantField::ToolGrants,
            ..
        }
    ));
}

#[test]
fn tool_grants_wrong_version_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    // Same name, different version is a DIFFERENT ToolGrant (ToolVersion
    // is part of the tuple identity). Must be in the parent's set.
    child_spec.tool_grants = BTreeSet::from([ToolGrant {
        tool_id: ToolName("fs-read".into()),
        tool_version: ToolVersion("999".into()),
    }]);
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::AttemptedWiden {
            field: WarrantField::ToolGrants,
            ..
        }
    ));
}

// -----------------------------------------------------------------
// syscall_profile_ref: opaque equality (v0.1)
// -----------------------------------------------------------------

#[test]
fn syscall_profile_mismatch_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.syscall_profile_ref = ContentRef::from_bytes([1; 32]);
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(
        err,
        NarrowingError::SyscallProfileNotASubset { .. }
    ));
}

// -----------------------------------------------------------------
// model_route: zero ceilings rejected; quantitative axes narrow silently
// -----------------------------------------------------------------

#[test]
fn model_route_zero_max_calls_rejected() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.model_route.max_calls = 0;
    let err = intersect(&parent, &role_with(child_spec)).unwrap_err();
    assert!(matches!(err, NarrowingError::InvalidModelRoute { .. }));
}

#[test]
fn model_route_silently_narrows_quantitative_axes() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.model_route.max_input_tokens = 99_999_999; // wider than parent (8000)
    let result = intersect(&parent, &role_with(child_spec)).unwrap();
    // Silently narrowed to parent's ceiling (8000), no error.
    assert_eq!(result.model_route.max_input_tokens, 8000);
}

#[test]
fn model_route_child_can_name_different_model() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.model_route.model_id = ModelId("claude-3.7".into());
    let result = intersect(&parent, &role_with(child_spec)).unwrap();
    assert_eq!(result.model_route.model_id.0, "claude-3.7");
}

// -----------------------------------------------------------------
// resource_ceiling: silent min() narrowing
// -----------------------------------------------------------------

#[test]
fn resource_ceiling_narrows_to_min() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.resource_ceiling.cpu_milli = 99_999; // wider than parent (2000)
    child_spec.resource_ceiling.mem_bytes = 1 << 20; // tighter than parent
    let result = intersect(&parent, &role_with(child_spec)).unwrap();
    assert_eq!(result.resource_ceiling.cpu_milli, 2000); // min(99_999, 2000)
    assert_eq!(result.resource_ceiling.mem_bytes, 1 << 20); // min(1<<20, 4<<30)
}

// -----------------------------------------------------------------
// mote_class / nd_class set by child, NOT inherited
// -----------------------------------------------------------------

#[test]
fn mote_class_set_by_child_not_inherited() {
    let mut parent = permissive_warrant();
    parent.mote_class = MoteClass::WorldMutating;
    parent.nd_class = MoteClass::WorldMutating;
    let mut child_spec = parent.clone();
    child_spec.mote_class = MoteClass::Pure;
    child_spec.nd_class = MoteClass::Pure;
    let result = intersect(&parent, &role_with(child_spec)).unwrap();
    assert_eq!(result.mote_class, MoteClass::Pure);
    assert_eq!(result.nd_class, MoteClass::Pure);
}

// -----------------------------------------------------------------
// executor_class / environment_ref set by child
// -----------------------------------------------------------------

#[test]
fn executor_class_set_by_child() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.executor_class = ExecutorClass::OciDaemon;
    let result = intersect(&parent, &role_with(child_spec)).unwrap();
    assert_eq!(result.executor_class, ExecutorClass::OciDaemon);
}

#[test]
fn environment_ref_set_by_child() {
    let parent = permissive_warrant();
    let mut child_spec = parent.clone();
    child_spec.environment_ref = Some(ContentRef::from_bytes([42; 32]));
    let result = intersect(&parent, &role_with(child_spec)).unwrap();
    assert_eq!(
        result.environment_ref,
        Some(ContentRef::from_bytes([42; 32]))
    );
}

// -----------------------------------------------------------------
// check_tool_requirement: per-axis truth table
// -----------------------------------------------------------------

#[test]
fn tool_req_within_warrant_ok() {
    let warrant = permissive_warrant();
    let req = ToolRequirement {
        net_scope_required: NetScope::EgressAllowlist(BTreeSet::from([Host(
            "api.example.com:443".into(),
        )])),
        fs_scope_required: FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/input"), FsMode::ReadOnly)]),
        },
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        min_resource_ceiling: ResourceCeiling {
            cpu_milli: 100,
            mem_bytes: 1 << 20,
            wall_clock_ms: 1_000,
            fd_count: 16,
            disk_bytes: 1 << 20,
        },
    };
    assert!(check_tool_requirement(&req, &warrant).is_ok());
}

#[test]
fn tool_req_too_much_memory_rejected() {
    let warrant = permissive_warrant();
    let req = ToolRequirement {
        net_scope_required: NetScope::None,
        fs_scope_required: FsScope::empty(),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        min_resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 1 << 50,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
    };
    assert!(matches!(
        check_tool_requirement(&req, &warrant),
        Err(ToolDenied {
            field: WarrantField::ResourceCeiling
        })
    ));
}

// -----------------------------------------------------------------
// warrant_ref_of / role_id_of: byte-determinism
// -----------------------------------------------------------------

#[test]
fn warrant_ref_of_is_deterministic() {
    let spec = permissive_warrant();
    let a = warrant_ref_of(&spec);
    let b = warrant_ref_of(&spec);
    assert_eq!(a, b);
}

#[test]
fn warrant_ref_of_changes_with_byte_change() {
    let spec_a = permissive_warrant();
    let mut spec_b = spec_a.clone();
    spec_b.model_route.max_calls += 1;
    assert_ne!(warrant_ref_of(&spec_a), warrant_ref_of(&spec_b));
}

#[test]
fn role_id_of_is_deterministic() {
    let role = role_with(permissive_warrant());
    assert_eq!(role_id_of(&role), role_id_of(&role));
}

// -----------------------------------------------------------------
// narrow == intersect alias
// -----------------------------------------------------------------

#[test]
fn narrow_is_alias_for_intersect() {
    let parent = permissive_warrant();
    let role = role_with(parent.clone());
    assert_eq!(intersect(&parent, &role), narrow(&parent, &role));
}

// -----------------------------------------------------------------
// MoteClass conversion round-trip
// -----------------------------------------------------------------

#[test]
fn mote_class_nd_class_roundtrip() {
    for mc in [
        MoteClass::Pure,
        MoteClass::ReadOnlyNondet,
        MoteClass::WorldMutating,
    ] {
        assert_eq!(MoteClass::from_nd_class(mc.to_nd_class()), mc);
    }
}
