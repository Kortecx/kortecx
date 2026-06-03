//! The load-bearing [`intersect`] function (and its alias [`narrow`]) —
//! monotonic per-axis narrowing of parent warrant + child role into the
//! child's effective warrant. **Pure**: same inputs → byte-identical output.

use crate::errors::NarrowingError;
use crate::fields::WarrantField;
use crate::spec::{Role, WarrantSpec};

/// Compute the child Mote's effective warrant via monotonic narrowing.
///
/// The child proposes its role; the runtime enforces:
/// `child.warrant = intersect(parent.warrant, child.role.spec)`.
///
/// # Returns
///
/// `Ok(child_warrant)` — the narrowed warrant, with every axis no wider than
/// the parent's. Quantitative axes (`resource_ceiling.*`,
/// `model_route.max_*`) are silently narrowed via `min()`. Qualitative axes
/// that the child proposed wider produce typed errors.
///
/// # Errors
///
/// Returns [`NarrowingError::AttemptedWiden`] if the child proposed wider on
/// any of: `fs_scope`, `net_scope`, `tool_grants`. Returns
/// [`NarrowingError::SyscallProfileNotASubset`] if the child's syscall
/// profile differs from the parent's (the seccomp compiler's subset check is
/// stubbed here as profile-ref equality; the real check lives in the seccomp
/// compiler, out of scope).
///
/// # Purity
///
/// This function is **pure**: same inputs → byte-identical output. No clock,
/// no I/O, no journal access. Recovery may call it freely.
///
/// # Example
///
/// ```
/// use kx_warrant::{intersect, ExecutorClass, FsScope, MoteClass, NetScope,
///     ModelRoute, ResourceCeiling, Role, WarrantSpec};
/// use kx_content::ContentRef;
/// use kx_mote::ModelId;
/// use std::collections::BTreeSet;
///
/// // Build a permissive parent warrant.
/// let parent = WarrantSpec {
///     mote_class: MoteClass::Pure,
///     nd_class: MoteClass::Pure,
///     fs_scope: FsScope::empty(),
///     net_scope: NetScope::None,
///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///     tool_grants: BTreeSet::new(),
///     model_route: ModelRoute {
///         model_id: ModelId("gpt-4".into()),
///         max_input_tokens: 8000,
///         max_output_tokens: 2000,
///         max_calls: 10,
///     },
///     resource_ceiling: ResourceCeiling {
///         cpu_milli: 1000, mem_bytes: 1 << 30, wall_clock_ms: 60_000,
///         fd_count: 256, disk_bytes: 1 << 30,
///     },
///     environment_ref: None,
///     // The `executor_class` axis is set by the child's role (per-axis
///     // narrowing inherits the AXES, not the VALUES); the macOS sibling
///     // variant `MacOsSandbox` is a sibling default on macOS hosts.
///     executor_class: ExecutorClass::Bwrap,
///     ..Default::default()
/// };
///
/// // A child role that strictly tightens (max_calls 10 → 5) AND selects
/// // the macOS sandbox backend (per-axis: executor_class is child-set, not
/// // intersected — workers MAY be tighter than master on this axis).
/// let mut child_spec = parent.clone();
/// child_spec.model_route.max_calls = 5;
/// child_spec.executor_class = ExecutorClass::MacOsSandbox;
/// let role = Role {
///     name: "tighter-child".into(),
///     version: 1,
///     spec: child_spec,
///     description: String::new(),
/// };
///
/// // Intersection returns Ok with the tighter ceiling + the child-set backend.
/// let child_warrant = intersect(&parent, &role).expect("tightening is allowed");
/// assert_eq!(child_warrant.model_route.max_calls, 5);
/// assert_eq!(child_warrant.executor_class, ExecutorClass::MacOsSandbox);
/// ```
#[tracing::instrument(level = "debug", skip_all, fields(role_name = %role.name, role_version = role.version))]
pub fn intersect(parent: &WarrantSpec, role: &Role) -> Result<WarrantSpec, NarrowingError> {
    let proposed = &role.spec;

    // fs_scope: child must reference only paths the parent references, with
    // mode ⊆ parent's mode on each.
    if !proposed.fs_scope.is_subset_of(&parent.fs_scope) {
        return Err(NarrowingError::AttemptedWiden {
            field: WarrantField::FsScope,
            parent: format!("{:?}", parent.fs_scope),
            proposed: format!("{:?}", proposed.fs_scope),
        });
    }

    // net_scope: None blocks all; allowlist must be subset of parent's.
    if !proposed.net_scope.is_subset_of(&parent.net_scope) {
        return Err(NarrowingError::AttemptedWiden {
            field: WarrantField::NetScope,
            parent: format!("{:?}", parent.net_scope),
            proposed: format!("{:?}", proposed.net_scope),
        });
    }

    // syscall_profile_ref: treated opaquely. v0.1 subset check is equality
    // (the seccomp compiler will enforce the real subset relation).
    if proposed.syscall_profile_ref != parent.syscall_profile_ref {
        return Err(NarrowingError::SyscallProfileNotASubset {
            profile_ref: proposed.syscall_profile_ref,
        });
    }

    // tool_grants: child grants must be subset of parent grants.
    if !proposed.tool_grants.is_subset(&parent.tool_grants) {
        return Err(NarrowingError::AttemptedWiden {
            field: WarrantField::ToolGrants,
            parent: format!("{:?}", parent.tool_grants),
            proposed: format!("{:?}", proposed.tool_grants),
        });
    }

    // secret_scope: None authorizes nothing; allowlist must be subset of
    // parent's (the D110.3 authorization axis; mirrors net_scope).
    if !proposed.secret_scope.is_subset_of(&parent.secret_scope) {
        return Err(NarrowingError::AttemptedWiden {
            field: WarrantField::SecretScope,
            parent: format!("{:?}", parent.secret_scope),
            proposed: format!("{:?}", proposed.secret_scope),
        });
    }

    // model_route: child names its own model; quantitative ceilings narrow.
    // Reject zero ceilings as structurally invalid (a route with zero tokens
    // is useless; surface loudly).
    if proposed.model_route.max_input_tokens == 0
        || proposed.model_route.max_output_tokens == 0
        || proposed.model_route.max_calls == 0
    {
        return Err(NarrowingError::InvalidModelRoute {
            reason: "max_input_tokens, max_output_tokens, and max_calls must all be > 0".into(),
        });
    }
    let model_route = proposed
        .model_route
        .narrow_quantitative(&parent.model_route);

    // resource_ceiling: per-axis min.
    let resource_ceiling = proposed.resource_ceiling.narrow(&parent.resource_ceiling);

    // cost_ceiling: per-axis min — a child can only lower the dollar ceiling
    // (D115; mirrors resource_ceiling). Axis reserved; enforcement is M11.
    let cost_ceiling = proposed.cost_ceiling.narrow(&parent.cost_ceiling);

    // tls_required: tighten-only. A child can add TLS but never remove a
    // parent's requirement — the boolean dual of the quantitative `min()` axes
    // (safe value = true), so a relaxation is structurally impossible rather
    // than a loud error (D118.5).
    let tls_required = proposed.tls_required || parent.tls_required;

    // mote_class / nd_class: set by child's role (NOT inherited).
    // executor_class: set by child's role (orthogonal to narrowing).
    // environment_ref: set by child's role (orthogonal to narrowing).

    Ok(WarrantSpec {
        mote_class: proposed.mote_class,
        nd_class: proposed.nd_class,
        fs_scope: proposed.fs_scope.clone(),
        net_scope: proposed.net_scope.clone(),
        syscall_profile_ref: proposed.syscall_profile_ref,
        tool_grants: proposed.tool_grants.clone(),
        model_route,
        resource_ceiling,
        environment_ref: proposed.environment_ref,
        executor_class: proposed.executor_class,
        secret_scope: proposed.secret_scope.clone(),
        cost_ceiling,
        tls_required,
    })
}

/// Typed wrapper over [`intersect`]; aliases for call-site readability.
///
/// `narrow(parent, child_role)` ≡ `intersect(parent, child_role)`. Use
/// whichever reads better in context.
#[inline]
pub fn narrow(parent: &WarrantSpec, child_role: &Role) -> Result<WarrantSpec, NarrowingError> {
    intersect(parent, child_role)
}
