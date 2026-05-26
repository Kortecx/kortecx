//! Content-addressed identity helpers: [`warrant_ref_of`] (for
//! [`crate::WarrantSpec`]) + [`role_id_of`] (for [`crate::Role`]).

use kx_content::ContentRef;
use kx_mote::canonical_config;

use crate::spec::{Role, WarrantSpec};

/// Compute the content-addressed [`ContentRef`] for a [`WarrantSpec`].
///
/// `warrant_ref = blake3(canonical_bincode(WarrantSpec))`.
///
/// The bincode configuration is the workspace canonical
/// ([`kx_mote::canonical_config`]): standard + little-endian + fixed-int.
/// Two semantically-identical warrants produce byte-identical refs.
///
/// # Panics
///
/// Panics if the bincode encoder fails — which cannot happen for the
/// `WarrantSpec` shape (all fields are deterministically encodable). The
/// panic surfaces loudly rather than corrupting identity.
///
/// # Example
///
/// ```
/// use kx_warrant::{warrant_ref_of, ExecutorClass, FsScope, MoteClass,
///     ModelRoute, NetScope, ResourceCeiling, WarrantSpec};
/// use kx_content::ContentRef;
/// use kx_mote::ModelId;
/// use std::collections::BTreeSet;
///
/// let spec = WarrantSpec {
///     mote_class: MoteClass::Pure, nd_class: MoteClass::Pure,
///     fs_scope: FsScope::empty(), net_scope: NetScope::None,
///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///     tool_grants: BTreeSet::new(),
///     model_route: ModelRoute {
///         model_id: ModelId("m".into()), max_input_tokens: 10,
///         max_output_tokens: 10, max_calls: 1,
///     },
///     resource_ceiling: ResourceCeiling {
///         cpu_milli: 0, mem_bytes: 0, wall_clock_ms: 0,
///         fd_count: 0, disk_bytes: 0,
///     },
///     environment_ref: None, executor_class: ExecutorClass::Bwrap,
/// };
/// // Same spec → same ref (idempotent).
/// assert_eq!(warrant_ref_of(&spec), warrant_ref_of(&spec));
/// ```
#[tracing::instrument(level = "trace", skip_all)]
#[must_use]
pub fn warrant_ref_of(spec: &WarrantSpec) -> ContentRef {
    let bytes = bincode::serde::encode_to_vec(spec, canonical_config())
        .expect("canonical bincode encoding of WarrantSpec cannot fail");
    ContentRef::of(&bytes)
}

/// Compute the content-addressed [`ContentRef`] for a [`Role`].
///
/// `role_id = blake3(canonical_bincode(Role))`. Two byte-identical roles
/// produce identical IDs; one byte changed → new ID.
///
/// # Panics
///
/// Panics if bincode encoding fails (impossible for the `Role` shape).
#[must_use]
pub fn role_id_of(role: &Role) -> ContentRef {
    let bytes = bincode::serde::encode_to_vec(role, canonical_config())
        .expect("canonical bincode encoding of Role cannot fail");
    ContentRef::of(&bytes)
}
