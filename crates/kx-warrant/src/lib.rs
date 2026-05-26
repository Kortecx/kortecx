// SPDX-License-Identifier: Apache-2.0
//! `kx-warrant` — the runtime-enforced capability layer (D30).
//!
//! A **Warrant** is the scoped capability boundary a Mote executes under: a
//! runtime-ENFORCED FACT, typed and structured (never prose), re-derivable
//! bit-for-bit on recovery. A **Role** is a named, versioned, content-addressed
//! `WarrantSpec` template — the RBAC surface.
//!
//! The load-bearing invariant of this crate is **monotonic narrowing**:
//!
//! ```text
//! child.warrant = intersect(parent.warrant, child.role)
//! ```
//!
//! - The **runtime ENFORCES** the intersection; the **model PROPOSES** which
//!   role to assume and may narrow within it.
//! - The model **never authorizes a widen** on any axis. Widening is a typed
//!   error (`NarrowingError::AttemptedWiden`).
//! - The intersection function is **PURE**: same inputs → byte-identical
//!   output. No I/O, no clock, no journal access. Recovery re-derives warrants
//!   bit-for-bit (machine-independent).
//!
//! # Six narrowable axes (qualitative — widening rejected as typed error)
//!
//! | Axis                  | Semantics                                              |
//! |-----------------------|--------------------------------------------------------|
//! | `fs_scope`            | path-set intersection; per-path mode min-bound         |
//! | `net_scope`           | egress allowlist subset; `None` blocks all egress      |
//! | `syscall_profile_ref` | opaque content-ref; subset check deferred to compiler  |
//! | `tool_grants`         | set subset on `(ToolName, ToolVersion)`                |
//! | `executor_class`      | set by child's role; not narrowed from parent          |
//! | `environment_ref`     | set by child's role; not narrowed from parent          |
//!
//! # Quantitative axes (narrowed silently via `min()`)
//!
//! - `resource_ceiling.*` — cpu_milli, mem_bytes, wall_clock_ms, fd_count, disk_bytes.
//! - `model_route.max_input_tokens` / `max_output_tokens` / `max_calls`.
//!
//! # `mote_class` and `nd_class` are set by child's role (NOT inherited).
//!
//! A child may be `Pure` under a `WorldMutating` parent (workers may choose to
//! be tighter). The intersection function leaves these fields as the child's
//! declared value without narrowing.
//!
//! # Content-addressed identity
//!
//! ```text
//! warrant_ref = blake3(canonical_bincode(WarrantSpec))
//! ```
//!
//! Two semantically-identical warrants produce byte-identical refs;
//! identity-bearing. See [`warrant_ref_of`].
//!
//! # Reading further
//!
//! - `docs/design/warrant.md` (private corpus) — the locked spec for D30.
//! - `docs/design/decisions.md` D30, D32, D33, D35, D36 — interlocking decisions.
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// TODO(workspace.lints cleanup): kx-warrant uses `.expect()` on
// canonical-bincode encode (documented infallible) for `warrant_ref_of`.
// Follow-up cleanup PR migrates to typed error or extracts the encode
// call to a shared helper that returns Result. Until then, the documented
// `expect(...)` is the audit trail.
#![allow(clippy::expect_used)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::needless_pass_by_value
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{canonical_config, ModelId, NdClass, ToolName, ToolVersion};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Small enums (stable u8 discriminants — wire-stable)
// ---------------------------------------------------------------------------

/// The non-determinism class a Mote attempts under. Mirrors [`NdClass`] from
/// `kx-mote`; restated here so the warrant layer carries its own semantically
/// equivalent enum without coupling to the journal-side discriminant.
///
/// Set by the child's role; **NOT inherited** from the parent warrant. A child
/// may be `Pure` under a `WorldMutating` parent (workers may be tighter than
/// their parent on this axis).
///
/// # Example
///
/// ```
/// use kx_warrant::MoteClass;
/// assert_eq!(MoteClass::Pure as u8, 0);
/// assert_eq!(MoteClass::ReadOnlyNondet as u8, 1);
/// assert_eq!(MoteClass::WorldMutating as u8, 2);
/// ```
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MoteClass {
    /// Pure: bit-stable function of inputs. No side effects. Safe to re-run.
    Pure = 0,
    /// Reads from a non-deterministic source (model inference, RNG) but causes
    /// no external state change. NEVER re-run once Committed.
    ReadOnlyNondet = 1,
    /// Performs an external effect (filesystem write, network call, etc.).
    /// Validate-then-commit per D20; effect-once via the broker.
    WorldMutating = 2,
}

impl MoteClass {
    /// Convert from kx-mote's `NdClass` to keep wire-format parity.
    #[inline]
    #[must_use]
    pub fn from_nd_class(nd: NdClass) -> Self {
        match nd {
            NdClass::Pure => Self::Pure,
            NdClass::ReadOnlyNondet => Self::ReadOnlyNondet,
            NdClass::WorldMutating => Self::WorldMutating,
        }
    }

    /// Convert to kx-mote's `NdClass`.
    #[inline]
    #[must_use]
    pub fn to_nd_class(self) -> NdClass {
        match self {
            Self::Pure => NdClass::Pure,
            Self::ReadOnlyNondet => NdClass::ReadOnlyNondet,
            Self::WorldMutating => NdClass::WorldMutating,
        }
    }
}

/// Which executor backend is responsible for running the Mote. Set by the
/// child's role; orthogonal to narrowing.
///
/// `Bwrap` is the OSS default on Linux; `OciDaemon` is a warrant-declared
/// opt-in for narrow cases (GPU passthrough or live-service environments);
/// `CloudMicroVm` is cloud-side per D28.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ExecutorClass {
    /// Bubblewrap over an extracted, content-addressed OCI rootfs. Daemonless;
    /// ms-spawn; least-privilege. The default on Linux.
    Bwrap = 0,
    /// Container runtime (Podman/runc preferred over Docker). Warrant-declared
    /// opt-in for narrow cases.
    OciDaemon = 1,
    /// Cloud-side microVM (firecracker / kata). Stub in OSS; concrete impl
    /// lives behind the cloud feature flag.
    CloudMicroVm = 2,
    /// macOS sandbox-exec / Seatbelt sibling of `Bwrap`. The default on macOS
    /// (the platform-conditional `default_executor()` factory picks this on
    /// `target_os = "macos"`). Compiles a `WarrantSpec` into an SBPL profile +
    /// spawns the Mote body via `posix_spawn` under `sandbox_init`-equivalent
    /// enforcement. Additive variant; existing warrant_refs are preserved
    /// because the discriminant is appended (variant 3, not interleaved).
    MacOsSandbox = 3,
}

/// Filesystem access mode for a mount in [`FsScope`].
///
/// Modes form a total order under "permits at most": `ReadOnly < ReadWrite`
/// for write access; `ExecOnly` is orthogonal (permits `exec` but not
/// read/write). The intersection per path is set-intersection on the
/// permitted operations.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FsMode {
    /// Read access only.
    ReadOnly = 0,
    /// Read + write access.
    ReadWrite = 1,
    /// Execute access only (no read/write).
    ExecOnly = 2,
}

impl FsMode {
    /// `true` iff `self` is no wider than `parent` on every operation.
    #[inline]
    #[must_use]
    pub fn is_subset_of(self, parent: FsMode) -> bool {
        matches!(
            (self, parent),
            (Self::ReadOnly, Self::ReadOnly | Self::ReadWrite)
                | (Self::ReadWrite, Self::ReadWrite)
                | (Self::ExecOnly, Self::ExecOnly)
        )
    }
}

/// Identifier for a [`WarrantField`]; used by [`NarrowingError`] to report
/// which axis a child role tried to widen.
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

// ---------------------------------------------------------------------------
// Composite axes
// ---------------------------------------------------------------------------

/// A host:port pair in the egress allowlist of [`NetScope::EgressAllowlist`].
///
/// Stored as an opaque string so the warrant layer doesn't reimplement URL
/// parsing; validation happens at workflow-author time (SDK / CLI front door).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Host(pub String);

/// Filesystem scope: a mapping from mount points to access modes.
///
/// Intersection is **set-intersection** on the path keys, with per-path mode
/// intersection on the values (see [`FsMode::is_subset_of`]). A child may
/// reference only paths the parent also references; a child's mode at any
/// path must be `is_subset_of` the parent's mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FsScope {
    /// Mount points and their access modes. `BTreeMap` for canonical
    /// iteration order (bincode-canonical encoding depends on this).
    pub mounts: BTreeMap<PathBuf, FsMode>,
}

impl FsScope {
    /// Construct an empty `FsScope` (no mounts; no filesystem access).
    #[inline]
    #[must_use]
    pub fn empty() -> Self {
        Self {
            mounts: BTreeMap::new(),
        }
    }

    /// `true` iff `self` is no wider than `parent`: every path of `self` is a
    /// path of `parent`, AND `self`'s mode at that path is a subset of
    /// `parent`'s mode there.
    #[must_use]
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        self.mounts.iter().all(|(path, mode)| {
            parent
                .mounts
                .get(path)
                .is_some_and(|p_mode| mode.is_subset_of(*p_mode))
        })
    }
}

/// Network egress scope.
///
/// `None` blocks all egress. `EgressAllowlist({h1, h2})` permits egress to
/// exactly the listed hosts. Intersection respects monotonic narrowing:
/// `None ∩ anything = None`; `EgressAllowlist(C) ∩ EgressAllowlist(P) = C`
/// only when `C ⊆ P` (else widening, refused).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetScope {
    /// No egress permitted.
    None,
    /// Egress permitted to exactly the listed hosts.
    EgressAllowlist(BTreeSet<Host>),
}

impl NetScope {
    /// `true` iff `self` permits no more egress than `parent`.
    #[must_use]
    pub fn is_subset_of(&self, parent: &Self) -> bool {
        match (self, parent) {
            (Self::None, _) => true,
            (Self::EgressAllowlist(_), Self::None) => false,
            (Self::EgressAllowlist(child), Self::EgressAllowlist(parent)) => {
                child.is_subset(parent)
            }
        }
    }
}

/// A reference to a tool in the shared registry (per D32).
///
/// Carries `(ToolName, ToolVersion)` — a reference, NOT an embedded copy of
/// the tool's specification. The registry (P1.7.7) holds the spec; this is
/// only a handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ToolGrant {
    /// The tool's name in the registry.
    pub tool_id: ToolName,
    /// The pinned version of the tool.
    pub tool_version: ToolVersion,
}

/// The model that this Mote attempts under, with quantitative ceilings.
///
/// In OSS v0.1, `model_id` is **named by the user** (per D35 — no auto
/// selection). The dispatcher routes to this exact model; switching models
/// busts the cache (the model id participates in the idempotency key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRoute {
    /// Named model identifier.
    pub model_id: ModelId,
    /// Maximum input tokens per call.
    pub max_input_tokens: u32,
    /// Maximum output tokens per call.
    pub max_output_tokens: u32,
    /// Maximum inference calls under this warrant.
    pub max_calls: u32,
}

impl ModelRoute {
    /// `true` iff every quantitative axis of `self` is no greater than the
    /// matching axis of `parent`. Different `model_id`s are allowed (the
    /// child's role may name a different model — see D30 §4.2).
    #[must_use]
    pub fn is_within(&self, parent: &Self) -> bool {
        self.max_input_tokens <= parent.max_input_tokens
            && self.max_output_tokens <= parent.max_output_tokens
            && self.max_calls <= parent.max_calls
    }

    /// Narrow `self` to the per-axis min against `parent`, returning the
    /// resulting `ModelRoute`. `model_id` is taken from `self` (child's choice).
    #[must_use]
    pub fn narrow_quantitative(&self, parent: &Self) -> Self {
        Self {
            model_id: self.model_id.clone(),
            max_input_tokens: self.max_input_tokens.min(parent.max_input_tokens),
            max_output_tokens: self.max_output_tokens.min(parent.max_output_tokens),
            max_calls: self.max_calls.min(parent.max_calls),
        }
    }
}

/// Quantitative resource ceilings for the Mote attempt.
///
/// Narrowed silently per axis via `min()`. Negative narrowing is never an
/// error: child asking for less than parent is the EXPECTED case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceCeiling {
    /// CPU allocation in milli-cores.
    pub cpu_milli: u32,
    /// Memory cap in bytes.
    pub mem_bytes: u64,
    /// Wall-clock timeout in milliseconds (also the inference-call timeout per D35).
    pub wall_clock_ms: u64,
    /// Open file descriptor cap.
    pub fd_count: u32,
    /// Disk byte cap.
    pub disk_bytes: u64,
}

impl ResourceCeiling {
    /// Per-axis `min` narrowing.
    #[inline]
    #[must_use]
    pub fn narrow(&self, parent: &Self) -> Self {
        Self {
            cpu_milli: self.cpu_milli.min(parent.cpu_milli),
            mem_bytes: self.mem_bytes.min(parent.mem_bytes),
            wall_clock_ms: self.wall_clock_ms.min(parent.wall_clock_ms),
            fd_count: self.fd_count.min(parent.fd_count),
            disk_bytes: self.disk_bytes.min(parent.disk_bytes),
        }
    }
}

/// The complete capability envelope a Mote attempts under.
///
/// Computed via [`intersect`] of the parent's warrant and the child's role.
/// Content-addressed via [`warrant_ref_of`]; recovery re-derives bit-for-bit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WarrantSpec {
    /// Non-determinism class. Set by child's role; NOT inherited.
    pub mote_class: MoteClass,
    /// Mirror of [`mote_class`](Self::mote_class) for journal-side parity.
    pub nd_class: MoteClass,
    /// Filesystem scope. Narrowable; widening → `AttemptedWiden`.
    pub fs_scope: FsScope,
    /// Network egress scope. Narrowable; widening → `AttemptedWiden`.
    pub net_scope: NetScope,
    /// Content-addressed seccomp-bpf profile. Treated opaquely; subset check
    /// happens at the seccomp compiler (out of scope for this crate).
    pub syscall_profile_ref: ContentRef,
    /// Set of granted tool refs. Narrowable via set subset.
    pub tool_grants: BTreeSet<ToolGrant>,
    /// Model route. Quantitative axes narrow via `min()`; `model_id` is set
    /// by the child's role.
    pub model_route: ModelRoute,
    /// Resource ceilings. Narrow silently via `min()` per axis.
    pub resource_ceiling: ResourceCeiling,
    /// Optional reference to an extracted OCI rootfs. Set by child's role;
    /// not narrowed from parent.
    pub environment_ref: Option<ContentRef>,
    /// Executor backend selection. Set by child's role.
    pub executor_class: ExecutorClass,
}

/// A `(name, version, spec, description)` tuple identifying a named, versioned
/// capability template — the RBAC surface authored ahead of time.
///
/// Roles are content-addressed via [`role_id_of`]: two byte-identical roles
/// have the same `RoleId`; any byte-level change produces a new `RoleId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Role {
    /// Human-readable handle (e.g., `"read-only-research"`).
    pub name: String,
    /// Monotonic version number.
    pub version: u32,
    /// The capability template.
    pub spec: WarrantSpec,
    /// Free-form human description. NEVER parsed for enforcement.
    pub description: String,
}

/// Capability requirements a tool declares at registration time (per D32).
/// Checked by [`check_tool_requirement`] at resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolRequirement {
    /// Required egress (must be ⊆ warrant.net_scope).
    pub net_scope_required: NetScope,
    /// Required filesystem access (must be ⊆ warrant.fs_scope).
    pub fs_scope_required: FsScope,
    /// Required seccomp profile (treated opaquely; compiler enforces subset).
    pub syscall_profile_ref: ContentRef,
    /// Minimum resource ceiling required (each axis ≤ warrant ceiling).
    pub min_resource_ceiling: ResourceCeiling,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Typed error returned by [`intersect`] when the child's role attempts to
/// widen on a qualitative axis. Quantitative axes never produce this error;
/// they narrow silently via `min()`.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum NarrowingError {
    /// Child's role proposed a value wider than the parent's on a qualitative
    /// axis. Always refused; the model NEVER authorizes a widen.
    #[error("child role attempted to widen warrant on field {field:?}: parent={parent} proposed={proposed}")]
    AttemptedWiden {
        /// The axis that was widened.
        field: WarrantField,
        /// Debug rendering of parent's value.
        parent: String,
        /// Debug rendering of child's proposed value.
        proposed: String,
    },
    /// Intersection on this axis is empty after narrowing.
    #[error("intersection on field {field:?} is empty")]
    EmptyIntersect {
        /// The axis with the empty intersection.
        field: WarrantField,
    },
    /// Syscall profile is not a subset of the parent's profile (per the
    /// seccomp compiler; treated opaquely here).
    #[error("syscall profile {profile_ref:?} is not a subset of parent")]
    SyscallProfileNotASubset {
        /// The non-subset profile reference.
        profile_ref: ContentRef,
    },
    /// Model route is structurally invalid (e.g., zero token ceiling).
    #[error("invalid model route: {reason}")]
    InvalidModelRoute {
        /// Description of why the route is invalid.
        reason: String,
    },
}

/// Typed error returned by [`check_tool_requirement`] when the tool's
/// `ToolRequirement` exceeds the Mote's warrant on a specific axis.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("tool requirement exceeds warrant on field {field:?}")]
pub struct ToolDenied {
    /// The axis that was exceeded.
    pub field: WarrantField,
}

// ---------------------------------------------------------------------------
// Pure functions: intersect, narrow, check_tool_requirement
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Unit tests (per-axis truth tables + edge cases)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
