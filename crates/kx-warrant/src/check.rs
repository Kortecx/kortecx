//! [`check_tool_requirement`] — per-axis subset check of a tool's
//! [`ToolRequirement`] against a Mote's [`WarrantSpec`]. Called by the tool
//! registry at resolution time (D32).

use crate::errors::ToolDenied;
use crate::fields::WarrantField;
use crate::spec::{ToolRequirement, WarrantSpec};

/// Check that a tool's [`ToolRequirement`] is satisfied by the Mote's warrant.
///
/// Called by the tool registry at resolution time (per D32 — refuses dispatch
/// if the tool's required capability exceeds the warrant on any axis). The
/// broker (P1.8.5) NEVER sees a tool whose capability exceeds the warrant.
///
/// # Returns
///
/// `Ok(())` iff every axis of `req` is ⊆ the matching axis of `warrant`.
///
/// # Errors
///
/// Returns [`ToolDenied`] with the offending axis when the requirement exceeds
/// the warrant.
///
/// # Example
///
/// ```
/// use kx_warrant::{check_tool_requirement, ExecutorClass, FsScope, MoteClass,
///     ModelRoute, NetScope, ResourceCeiling, ToolRequirement, WarrantSpec};
/// use kx_content::ContentRef;
/// use kx_mote::ModelId;
/// use std::collections::BTreeSet;
///
/// let warrant = WarrantSpec {
///     mote_class: MoteClass::Pure, nd_class: MoteClass::Pure,
///     fs_scope: FsScope::empty(), net_scope: NetScope::None,
///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///     tool_grants: BTreeSet::new(),
///     model_route: ModelRoute {
///         model_id: ModelId("m".into()), max_input_tokens: 100,
///         max_output_tokens: 100, max_calls: 1,
///     },
///     resource_ceiling: ResourceCeiling {
///         cpu_milli: 500, mem_bytes: 1 << 28, wall_clock_ms: 5_000,
///         fd_count: 64, disk_bytes: 1 << 28,
///     },
///     environment_ref: None, executor_class: ExecutorClass::Bwrap,
/// };
///
/// // A tool requiring more memory than the warrant permits → ToolDenied.
/// let req = ToolRequirement {
///     net_scope_required: NetScope::None,
///     fs_scope_required: FsScope::empty(),
///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///     min_resource_ceiling: ResourceCeiling {
///         cpu_milli: 100, mem_bytes: 1 << 40, wall_clock_ms: 1_000,
///         fd_count: 16, disk_bytes: 1 << 20,
///     },
/// };
/// assert!(check_tool_requirement(&req, &warrant).is_err());
/// ```
#[tracing::instrument(level = "debug", skip_all)]
pub fn check_tool_requirement(
    req: &ToolRequirement,
    warrant: &WarrantSpec,
) -> Result<(), ToolDenied> {
    if !req.fs_scope_required.is_subset_of(&warrant.fs_scope) {
        return Err(ToolDenied {
            field: WarrantField::FsScope,
        });
    }
    if !req.net_scope_required.is_subset_of(&warrant.net_scope) {
        return Err(ToolDenied {
            field: WarrantField::NetScope,
        });
    }
    if req.syscall_profile_ref != warrant.syscall_profile_ref {
        return Err(ToolDenied {
            field: WarrantField::SyscallProfile,
        });
    }
    let need = req.min_resource_ceiling;
    let have = warrant.resource_ceiling;
    if need.cpu_milli > have.cpu_milli
        || need.mem_bytes > have.mem_bytes
        || need.wall_clock_ms > have.wall_clock_ms
        || need.fd_count > have.fd_count
        || need.disk_bytes > have.disk_bytes
    {
        return Err(ToolDenied {
            field: WarrantField::ResourceCeiling,
        });
    }
    Ok(())
}
