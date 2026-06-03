//! The composite structs that make up a warrant: [`ToolGrant`], [`ModelRoute`],
//! [`ResourceCeiling`], the top-level [`WarrantSpec`], [`Role`] (the RBAC
//! template), and [`ToolRequirement`] (tool-side capability spec).

use std::collections::BTreeSet;

use kx_content::ContentRef;
use kx_mote::{ModelId, ToolName, ToolVersion};
use serde::{Deserialize, Serialize};

use crate::classes::{ExecutorClass, MoteClass};
use crate::scope::{FsScope, NetScope};
use crate::secret::SecretScope;

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

/// A quantitative cost ceiling, in micro-dollars (D115).
///
/// `micro_usd` is the MAXIMUM spend permitted under this warrant. Integer
/// fixed-point — **no float on the identity path** (SN-8). `0` = no spend
/// permitted (the fail-closed default); [`u64::MAX`] = effectively unlimited.
/// Narrowed silently via `min()` like [`ResourceCeiling`] (a child can never
/// raise the ceiling).
///
/// **M5.3 reserves the axis only** (D115.1): the field + `min()`-narrowing exist
/// here; the spend FOLD (accumulating cost) and the `spent ≤ cost_ceiling`
/// enforcement land in M11 (`kx-trace`). At M5.3 the ceiling is recorded +
/// narrowed but never consulted (no cost accounting is built).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CostCeiling {
    /// Maximum spend in micro-dollars (USD × 1_000_000).
    pub micro_usd: u64,
}

impl CostCeiling {
    /// Per-axis `min` narrowing (a child can only lower the ceiling).
    #[inline]
    #[must_use]
    pub fn narrow(&self, parent: &Self) -> Self {
        Self {
            micro_usd: self.micro_usd.min(parent.micro_usd),
        }
    }
}

/// The complete capability envelope a Mote attempts under.
///
/// Computed via [`crate::intersect`] of the parent's warrant and the child's
/// role. Content-addressed via [`crate::warrant_ref_of`]; recovery re-derives
/// bit-for-bit. [`WarrantSpec::default`] is a fail-closed **deny-all** envelope
/// (no tools, no egress, no filesystem, zero resource/cost ceilings, TLS
/// required, `Pure`) — the safe base every fixture and future axis builds on via
/// `..Default::default()`.
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
    /// Which secret references this warrant may resolve (D110.3). Narrowable via
    /// subset; widening → `AttemptedWiden`. `None` authorizes no secret.
    pub secret_scope: SecretScope,
    /// Quantitative cost ceiling (D115). Narrowed silently via `min()`. Axis
    /// reserved at M5.3; spend enforcement is M11 (see [`CostCeiling`]).
    pub cost_ceiling: CostCeiling,
    /// Force TLS on remote MCP egress (D118.5). Tighten-only: a parent requiring
    /// TLS forces every child to require it too (narrow = `||`). Consumed by the
    /// HTTP transport (`https_only`) in M5.3 PR-B.
    pub tls_required: bool,
}

impl Default for WarrantSpec {
    /// A fail-closed **deny-all** warrant: no tools, no egress, no filesystem,
    /// zero resource + cost ceilings, TLS required, `Pure` class, the default
    /// executor backend. The least-privilege base for `..Default::default()`.
    fn default() -> Self {
        Self {
            mote_class: MoteClass::Pure,
            nd_class: MoteClass::Pure,
            fs_scope: FsScope::empty(),
            net_scope: NetScope::None,
            syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
            tool_grants: BTreeSet::new(),
            model_route: ModelRoute {
                model_id: ModelId(String::new()),
                max_input_tokens: 0,
                max_output_tokens: 0,
                max_calls: 0,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
            environment_ref: None,
            executor_class: ExecutorClass::Bwrap,
            secret_scope: SecretScope::None,
            cost_ceiling: CostCeiling { micro_usd: 0 },
            tls_required: true,
        }
    }
}

/// A `(name, version, spec, description)` tuple identifying a named, versioned
/// capability template — the RBAC surface authored ahead of time.
///
/// Roles are content-addressed via [`crate::role_id_of`]: two byte-identical
/// roles have the same `RoleId`; any byte-level change produces a new `RoleId`.
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
/// Checked by [`crate::check_tool_requirement`] at resolution.
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
