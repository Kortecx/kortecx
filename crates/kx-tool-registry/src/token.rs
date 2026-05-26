//! [`registration_token_of`] — the deterministic content-addressed primary
//! key for a `(ToolDef, ToolProvenance)` pair.

use kx_content::ContentRef;
use kx_mote::canonical_config;

use crate::ids::RegistrationToken;
use crate::provenance::ToolProvenance;
use crate::tool_def::ToolDef;

/// Compute the deterministic `RegistrationToken` for a `(ToolDef, ToolProvenance)`
/// pair.
///
/// `RegistrationToken = blake3(canonical_bincode((def, provenance)))`. Same
/// inputs → same token; re-submitting an identical registration produces the
/// same token (idempotent on identity).
///
/// # Panics
///
/// Panics if bincode encoding fails (impossible for the shape).
///
/// # Example
///
/// ```
/// use kx_tool_registry::{
///     registration_token_of, IdempotencyClass, ToolDef, ToolKind, ToolProvenance,
/// };
/// use kx_mote::{ToolName, ToolVersion};
/// use kx_content::ContentRef;
/// use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};
///
/// let def = ToolDef {
///     tool_id: ToolName("fs-read".into()),
///     tool_version: ToolVersion("1".into()),
///     kind: ToolKind::Builtin,
///     required_capability: ToolRequirement {
///         net_scope_required: NetScope::None,
///         fs_scope_required: FsScope::empty(),
///         syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///         min_resource_ceiling: ResourceCeiling {
///             cpu_milli: 0, mem_bytes: 0, wall_clock_ms: 0,
///             fd_count: 0, disk_bytes: 0,
///         },
///     },
///     description: String::new(),
///     idempotency_class: IdempotencyClass::Token,
/// };
/// let prov = ToolProvenance::HumanAuthored { author: "ops".into() };
///
/// // Token is deterministic over identity.
/// assert_eq!(
///     registration_token_of(&def, &prov),
///     registration_token_of(&def, &prov)
/// );
/// ```
#[must_use]
pub fn registration_token_of(def: &ToolDef, provenance: &ToolProvenance) -> RegistrationToken {
    let bytes = bincode::serde::encode_to_vec((def, provenance), canonical_config())
        .expect("canonical bincode encoding of (ToolDef, ToolProvenance) cannot fail");
    RegistrationToken(ContentRef::of(&bytes))
}
