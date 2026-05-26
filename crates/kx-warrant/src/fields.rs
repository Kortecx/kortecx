//! [`WarrantField`] (axis identifier used in typed errors) + the [`Host`]
//! newtype used by [`crate::NetScope::EgressAllowlist`].

use serde::{Deserialize, Serialize};

/// Identifier for a [`crate::WarrantSpec`] axis; used by
/// [`crate::NarrowingError`] to report which axis a child role tried to widen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WarrantField {
    /// Filesystem scope.
    FsScope,
    /// Network egress scope.
    NetScope,
    /// Seccomp-bpf syscall profile.
    SyscallProfile,
    /// Tool grants.
    ToolGrants,
    /// Model route.
    ModelRoute,
    /// Resource ceilings.
    ResourceCeiling,
    /// Executor class.
    ExecutorClass,
    /// Environment rootfs.
    EnvironmentRef,
}

/// A host:port pair in the egress allowlist of
/// [`crate::NetScope::EgressAllowlist`].
///
/// Stored as an opaque string so the warrant layer doesn't reimplement URL
/// parsing; validation happens at workflow-author time (SDK / CLI front door).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Host(pub String);
