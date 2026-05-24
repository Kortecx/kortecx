// SPDX-License-Identifier: Apache-2.0
//! `kx-tool-registry` — the two-file tool layer (D32).
//!
//! **Two files, not one.**
//! - **Per-workflow file** (the CLI config form of a workflow) — markdown body
//!   for human intent + typed FRONT-MATTER for the enforceable Warrant/Role.
//!   The front-matter is the ONLY enforcement source. Parsed by the SDK/CLI at
//!   workflow-submit time (out of scope for this crate).
//! - **Shared organizational registry** (this crate) — holds the available
//!   tools. Each tool declares its OWN `ToolRequirement`. Built-ins ship on
//!   fresh install; custom tools accrete.
//!
//! Workflow `tool_grants` are **REFERENCES** into the registry, not copies.
//! The registry holds the spec; workflows reference by `(ToolName, ToolVersion)`.
//!
//! # Resolution path (D32 §5)
//!
//! `local → registry → MCP`. Invisible to the capability model — the warrant
//! sees only the `(ToolName, ToolVersion)` reference. BUT the resolved tier is
//! **journaled** via a content-addressed [`ToolResolutionEvent`] so replay
//! resolves identically. At resolution time, [`check_tool_requirement`]
//! enforces `tool.required_capability ⊆ warrant`; the broker (P1.8.5) never
//! sees a tool whose capability exceeds the warrant.
//!
//! [`check_tool_requirement`]: kx_warrant::check_tool_requirement
//!
//! # MCP tools as egress (monotonic with `net_scope`)
//!
//! MCP tools are remote → granting one requires the warrant's `net_scope` to
//! permit the MCP endpoint's host. A `net_scope = None` warrant cannot resolve
//! any MCP tool — the subset check rejects the resolution at the registry
//! layer, before any dispatch.
//!
//! # Self-generated tools INERT until human review
//!
//! Tools emitted by Motes are recorded with
//! [`ToolProvenance::SelfGenerated`] and start in
//! [`RegistrationStatus::PendingHumanReview`]. They are **INERT** —
//! [`ToolRegistry::resolve`] returns [`ResolutionError::PendingHumanReview`]
//! until [`ToolRegistry::approve_registration`] is called. Approval enforces
//! `def.required_capability ⊆ generating_lineage_warrant`. This closes the
//! privilege-laundering path where a model could emit a tool with broader
//! scope than the lineage that authored it (SN-8: model proposes, runtime
//! enforces).
//!
//! # OSS impl vs cloud impl
//!
//! [`InMemoryToolRegistry`] is the OSS impl — accretes within a single process
//! lifetime; appropriate for the OSS demo + local dev. The trait surface admits
//! a future `kx-cloud-tool-registry-hosted` crate (per-tenant persistence,
//! multi-host accretion, attestation) without trait change per D28.
//!
//! # Reading further
//!
//! - `docs/design/tool-registry.md` (private corpus) — the locked D32 spec.
//! - `docs/design/decisions.md` D32 — interlocking with D24 (broker), D30
//!   (warrant), D29 (validator).
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::needless_pass_by_value,
    // ToolProvenance::SelfGenerated carries a WarrantSpec (~hundreds of bytes);
    // HumanAuthored carries just a small String. The size disparity is
    // intentional — boxing the WarrantSpec would obscure the semantic shape
    // (the lineage warrant is part of the provenance, not a side reference)
    // for a negligible memory win. SelfGenerated registrations are also rare
    // (most tools are HumanAuthored).
    clippy::large_enum_variant
)]

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_mote::{canonical_config, MoteId, ToolName, ToolVersion};
use kx_warrant::{check_tool_requirement, ToolGrant, ToolRequirement, WarrantField, WarrantSpec};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Identifier newtypes
// ---------------------------------------------------------------------------

/// Identifier for an MCP endpoint registered with this registry.
///
/// Opaque string; the registry treats it as a handle. The actual MCP protocol
/// dispatch is the broker's responsibility (P1.8.5); this crate only carries
/// the identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct McpEndpointId(pub String);

/// Identifier for a human reviewer authorized to approve self-generated tools.
///
/// Opaque string (likely an org email or user id in real deployments).
/// Tracked in the registry's audit log; not enforcement-bearing in v0.1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReviewerId(pub String);

/// Content-addressed token returned by [`ToolRegistry::register`].
///
/// `RegistrationToken = blake3(canonical_bincode((ToolDef, ToolProvenance)))`.
/// Deterministic: re-submitting the same `(def, provenance)` produces the same
/// token. The token is the registry's primary key for the pending registration
/// (used by [`ToolRegistry::approve_registration`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegistrationToken(pub ContentRef);

// ---------------------------------------------------------------------------
// ToolKind — where the tool lives / how it's resolved
// ---------------------------------------------------------------------------

/// What kind of tool this is, and how it was sourced.
///
/// Reflected in [`ToolResolutionEvent::resolved_kind`] so replay can verify
/// the same tier resolved the same tool.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolKind {
    /// A built-in tool that ships with the OSS runtime (`fs-read`,
    /// `fs-write`, `http-get`, `text-summarize`, …).
    Builtin,
    /// A local script registered against this registry. The bytes of the
    /// script live in the content store at the given `script_ref`.
    LocalScript {
        /// Content-store reference to the script bytes.
        script_ref: ContentRef,
    },
    /// An external tool sourced from a URL (e.g., a hosted registry entry).
    External {
        /// Origin URL (opaque to this crate; resolved by the broker).
        source_url: String,
    },
    /// A tool exposed via MCP at the given endpoint with the given remote
    /// name. **Granting an MCP tool requires the warrant's `net_scope` to
    /// permit the MCP endpoint's host** — enforced by the subset check at
    /// resolution time.
    Mcp {
        /// Which MCP endpoint serves this tool.
        endpoint: McpEndpointId,
        /// The tool's name on the remote MCP server.
        remote_name: String,
    },
    /// A self-generated tool emitted by a Mote at the given identity. INERT
    /// until human review per D32; capability ⊆ generating lineage's warrant
    /// at approve time.
    SelfGenerated {
        /// The Mote that emitted this tool.
        generated_at_mote: MoteId,
    },
}

// ---------------------------------------------------------------------------
// ToolDef — the spec the registry stores
// ---------------------------------------------------------------------------

/// A tool's full specification, content-addressed by its
/// [`canonical_bincode`][canonical_config] bytes.
///
/// The registry's primary record. Workflows reference by `(tool_id,
/// tool_version)`; the registry resolves to a `ToolDef`. The `description`
/// field is free-form human prose and is **NEVER parsed for enforcement** —
/// it's there for operator-readable inspection only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name (the workflow's reference key, paired with `tool_version`).
    pub tool_id: ToolName,
    /// Pinned version of the tool. Different versions of the same `tool_id`
    /// are distinct registry entries.
    pub tool_version: ToolVersion,
    /// What kind of tool this is and where it lives.
    pub kind: ToolKind,
    /// The capability requirements this tool declares. Checked at resolution
    /// time against the Mote's warrant via
    /// [`kx_warrant::check_tool_requirement`].
    pub required_capability: ToolRequirement,
    /// Free-form human description. NEVER parsed for enforcement.
    pub description: String,
}

// ---------------------------------------------------------------------------
// Resolution-side types
// ---------------------------------------------------------------------------

/// The content-addressed fact that "tool X version Y was resolved as kind Z
/// from this registry at the resolution event corresponding to this `ContentRef`."
///
/// Journaled by the executor at the registry-resolution event so replay
/// resolves identically. **Identity excludes wall-clock time** — including time
/// would break content-addressing (two runs would produce different refs for
/// the same resolution). Time, if needed for audit, lives in the journal
/// entry's header.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolResolutionEvent {
    /// The tool that was resolved.
    pub tool_id: ToolName,
    /// The pinned version.
    pub tool_version: ToolVersion,
    /// What kind it resolved as (which tier served the resolution).
    pub resolved_kind: ToolKind,
    /// blake3 of the resolved `ToolDef`'s canonical bytes. Pins the exact
    /// `ToolDef` shape that was used for this resolution.
    pub resolved_def_hash: ContentRef,
}

impl ToolResolutionEvent {
    /// Compute the content-addressed `ContentRef` for this event.
    ///
    /// `event_ref = blake3(canonical_bincode(self))`. Deterministic and pure;
    /// recovery re-derives the same `ContentRef` bit-for-bit.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_tool_registry::{ToolKind, ToolResolutionEvent};
    /// use kx_mote::{ToolName, ToolVersion};
    /// use kx_content::ContentRef;
    ///
    /// let event = ToolResolutionEvent {
    ///     tool_id: ToolName("fs-read".into()),
    ///     tool_version: ToolVersion("1".into()),
    ///     resolved_kind: ToolKind::Builtin,
    ///     resolved_def_hash: ContentRef::from_bytes([0; 32]),
    /// };
    /// // Same event → same ref (deterministic).
    /// assert_eq!(event.to_ref(), event.to_ref());
    /// ```
    #[must_use]
    pub fn to_ref(&self) -> ContentRef {
        let bytes = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("canonical bincode encoding of ToolResolutionEvent cannot fail");
        ContentRef::of(&bytes)
    }
}

/// The result of [`ToolRegistry::resolve`]: the resolved tool's definition,
/// the journaling-ready resolution event with its content-addressed ref, and
/// the post-check effective capability (which is the tool's
/// `required_capability` — same per-axis values, but pinned to this resolution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTool {
    /// The resolved tool's spec.
    pub def: ToolDef,
    /// The resolution event (the executor writes its canonical bytes into the
    /// content store and journals `event_ref`).
    pub event: ToolResolutionEvent,
    /// `event.to_ref()` precomputed — the executor verifies this matches what
    /// the content store assigns.
    pub event_ref: ContentRef,
    /// The capability the tool will operate with after the subset check.
    /// Equal to `def.required_capability` on success.
    pub effective_capability: ToolRequirement,
}

// ---------------------------------------------------------------------------
// Registration-side types
// ---------------------------------------------------------------------------

/// Who/what authored this tool — drives the registration lifecycle (D32 §7).
///
/// `HumanAuthored` → registration is immediately
/// [`RegistrationStatus::Approved`]. `SelfGenerated` → registration is
/// [`RegistrationStatus::PendingHumanReview`] until
/// [`ToolRegistry::approve_registration`] is called with the lineage subset
/// check passing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolProvenance {
    /// Authored by a human (operator, workflow author, org maintainer).
    /// Approved on registration.
    HumanAuthored {
        /// Free-form author identifier (audit log only; not enforcement).
        author: String,
    },
    /// Emitted by a Mote. INERT until reviewed.
    SelfGenerated {
        /// The warrant in effect when the Mote emitted the tool. Used at
        /// approve time to enforce `def.required_capability ⊆
        /// generating_lineage_warrant`.
        generating_lineage_warrant: WarrantSpec,
        /// The Mote that emitted the tool.
        generating_mote: MoteId,
    },
}

/// Lifecycle state of a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegistrationStatus {
    /// Active and resolvable.
    Approved,
    /// Recorded but INERT — `resolve` refuses with `PendingHumanReview`.
    PendingHumanReview,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Reason [`ToolRegistry::resolve`] refused.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ResolutionError {
    /// No tool registered with this `(tool_id, tool_version)`.
    #[error("tool not found in registry: {tool_id:?}@{tool_version:?}")]
    NotFound {
        /// The requested tool id.
        tool_id: ToolName,
        /// The requested version.
        tool_version: ToolVersion,
    },
    /// The tool's required capability exceeds the warrant on this axis.
    #[error("tool requirement exceeds warrant on field {axis:?}")]
    CapabilityExceedsWarrant {
        /// The axis that was exceeded.
        axis: WarrantField,
    },
    /// MCP endpoint unreachable. Reserved for future use (the broker performs
    /// the actual reachability check; the registry surfaces this when
    /// short-circuiting at resolution).
    #[error("MCP endpoint unreachable: {endpoint:?}")]
    McpUnreachable {
        /// The unreachable endpoint.
        endpoint: McpEndpointId,
    },
    /// The tool exists but is `PendingHumanReview` — INERT until reviewed.
    #[error("registration pending human review: token={token:?}")]
    PendingHumanReview {
        /// The pending registration's token.
        token: RegistrationToken,
    },
}

/// Reason a registration operation failed.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RegistrationError {
    /// At approve time, `def.required_capability` is not a subset of the
    /// `generating_lineage_warrant` on this axis. Anti-privilege-laundering
    /// guard.
    #[error("self-generated tool's required capability exceeds the generating lineage's warrant on field {axis:?}")]
    InvalidLineageSubset {
        /// The axis where the subset check failed.
        axis: WarrantField,
    },
    /// The token doesn't match any registration in the registry.
    #[error("registration token unknown: {token:?}")]
    UnknownToken {
        /// The unknown token.
        token: RegistrationToken,
    },
    /// Approve was called on a registration that is already approved.
    #[error("registration already approved: {token:?}")]
    AlreadyApproved {
        /// The token whose registration was already approved.
        token: RegistrationToken,
    },
    /// A `HumanAuthored` registration cannot be approved separately — it is
    /// approved at registration. Surfaces if a caller tries to call
    /// `approve_registration` for a HumanAuthored token.
    #[error("HumanAuthored registrations are approved at register-time; nothing to do")]
    NotPendingReview {
        /// The token whose registration is not in PendingHumanReview state.
        token: RegistrationToken,
    },
}

// ---------------------------------------------------------------------------
// Helper: derive the content-addressed RegistrationToken
// ---------------------------------------------------------------------------

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
///     registration_token_of, ToolDef, ToolKind, ToolProvenance,
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

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

/// The registry seam (D32). OSS ships [`InMemoryToolRegistry`]; the cloud-side
/// hosted impl (`kx-cloud-tool-registry-hosted`) plugs in behind the same
/// trait per D28.
pub trait ToolRegistry: Send + Sync {
    /// Look up an approved tool by `(tool_id, tool_version)`.
    ///
    /// Returns `None` if the tool doesn't exist OR if its registration is
    /// still `PendingHumanReview`. (For pending-review tools, [`resolve`]
    /// distinguishes them via the dedicated error; `lookup` is the simple
    /// "is it usable" check.)
    ///
    /// [`resolve`]: ToolRegistry::resolve
    fn lookup(&self, tool_id: &ToolName, tool_version: &ToolVersion) -> Option<ToolDef>;

    /// Resolve a tool grant against the Mote's warrant. Enforces
    /// `tool.required_capability ⊆ warrant` per axis via
    /// [`kx_warrant::check_tool_requirement`].
    ///
    /// On success returns a [`ResolvedTool`] carrying the content-addressed
    /// `ToolResolutionEvent` — the executor writes its bytes to the content
    /// store and journals `event_ref`.
    ///
    /// # Errors
    ///
    /// - [`ResolutionError::NotFound`] — no tool with this `(id, version)`.
    /// - [`ResolutionError::PendingHumanReview`] — INERT self-generated tool.
    /// - [`ResolutionError::CapabilityExceedsWarrant`] — subset check failed
    ///   on the specified axis.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
    /// use kx_mote::{ModelId, ToolName, ToolVersion};
    /// use kx_warrant::{
    ///     ExecutorClass, FsScope, MoteClass, NetScope, ModelRoute,
    ///     ResourceCeiling, ToolGrant, WarrantSpec,
    /// };
    /// use kx_content::ContentRef;
    /// use std::collections::BTreeSet;
    ///
    /// let reg = InMemoryToolRegistry::with_builtins();
    /// let grant = ToolGrant {
    ///     tool_id: ToolName("fs-read".into()),
    ///     tool_version: ToolVersion("1".into()),
    /// };
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
    ///         cpu_milli: 100, mem_bytes: 1 << 20, wall_clock_ms: 1000,
    ///         fd_count: 16, disk_bytes: 1 << 20,
    ///     },
    ///     environment_ref: None, executor_class: ExecutorClass::Bwrap,
    /// };
    /// let resolved = reg.resolve(&grant, &warrant).expect("builtin fs-read fits");
    /// assert_eq!(resolved.def.tool_id.0, "fs-read");
    /// ```
    fn resolve(
        &self,
        grant: &ToolGrant,
        warrant: &WarrantSpec,
    ) -> Result<ResolvedTool, ResolutionError>;

    /// Register a tool. For `HumanAuthored` provenance, the registration is
    /// immediately `Approved`. For `SelfGenerated`, the registration is
    /// `PendingHumanReview` until [`approve_registration`] is called.
    ///
    /// Returns the deterministic [`RegistrationToken`].
    ///
    /// [`approve_registration`]: ToolRegistry::approve_registration
    ///
    /// # Errors
    ///
    /// (No errors in v0.1; reserved for future capacity / quota / authz
    /// checks. Returns `Ok(token)` on every input pair.)
    fn register(
        &mut self,
        def: ToolDef,
        provenance: ToolProvenance,
    ) -> Result<RegistrationToken, RegistrationError>;

    /// Approve a `PendingHumanReview` registration. Enforces
    /// `def.required_capability ⊆ generating_lineage_warrant` per axis — the
    /// anti-privilege-laundering guard against model-emitted broad-scoped tools.
    ///
    /// # Errors
    ///
    /// - [`RegistrationError::UnknownToken`] — no such registration.
    /// - [`RegistrationError::AlreadyApproved`] — registration was already
    ///   approved (HumanAuthored, or a duplicate approve call).
    /// - [`RegistrationError::NotPendingReview`] — token references a
    ///   HumanAuthored registration (which doesn't enter PendingHumanReview).
    /// - [`RegistrationError::InvalidLineageSubset`] — subset check failed on
    ///   the specified axis.
    ///
    /// # Example
    ///
    /// ```
    /// use kx_tool_registry::{
    ///     InMemoryToolRegistry, ResolutionError, ReviewerId, ToolDef,
    ///     ToolKind, ToolProvenance, ToolRegistry,
    /// };
    /// use kx_mote::{ModelId, MoteId, ToolName, ToolVersion};
    /// use kx_warrant::{
    ///     ExecutorClass, FsScope, MoteClass, NetScope, ModelRoute,
    ///     ResourceCeiling, ToolGrant, ToolRequirement, WarrantSpec,
    /// };
    /// use kx_content::ContentRef;
    /// use std::collections::BTreeSet;
    ///
    /// let lineage_warrant = WarrantSpec {
    ///     mote_class: MoteClass::Pure, nd_class: MoteClass::Pure,
    ///     fs_scope: FsScope::empty(), net_scope: NetScope::None,
    ///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
    ///     tool_grants: BTreeSet::new(),
    ///     model_route: ModelRoute {
    ///         model_id: ModelId("m".into()), max_input_tokens: 100,
    ///         max_output_tokens: 100, max_calls: 1,
    ///     },
    ///     resource_ceiling: ResourceCeiling {
    ///         cpu_milli: 100, mem_bytes: 1 << 20, wall_clock_ms: 1000,
    ///         fd_count: 16, disk_bytes: 1 << 20,
    ///     },
    ///     environment_ref: None, executor_class: ExecutorClass::Bwrap,
    /// };
    ///
    /// let mut reg = InMemoryToolRegistry::new();
    /// let def = ToolDef {
    ///     tool_id: ToolName("emit".into()),
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
    /// };
    /// let token = reg.register(
    ///     def.clone(),
    ///     ToolProvenance::SelfGenerated {
    ///         generating_lineage_warrant: lineage_warrant,
    ///         generating_mote: MoteId([0; 32]),
    ///     },
    /// ).unwrap();
    ///
    /// // INERT until approved.
    /// let grant = ToolGrant {
    ///     tool_id: def.tool_id.clone(),
    ///     tool_version: def.tool_version.clone(),
    /// };
    ///
    /// // Approve — req ⊆ lineage on every axis.
    /// reg.approve_registration(token, ReviewerId("alice".into())).unwrap();
    /// assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
    /// ```
    fn approve_registration(
        &mut self,
        token: RegistrationToken,
        approver: ReviewerId,
    ) -> Result<(), RegistrationError>;
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

/// Internal per-registration record.
#[derive(Debug, Clone)]
struct RegistrationRecord {
    def: ToolDef,
    provenance: ToolProvenance,
    status: RegistrationStatus,
    /// `Some(reviewer)` once approved by an operator.
    approved_by: Option<ReviewerId>,
}

/// The OSS in-process registry. Accretes within a single process lifetime;
/// not persisted (D32). Backed by a `BTreeMap` for canonical iteration order.
///
/// # Example
///
/// ```
/// use kx_tool_registry::{
///     InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry,
/// };
/// use kx_mote::{ToolName, ToolVersion};
/// use kx_content::ContentRef;
/// use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};
///
/// let mut reg = InMemoryToolRegistry::new();
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
///     description: "Read files from /input".into(),
/// };
///
/// let token = reg.register(
///     def.clone(),
///     ToolProvenance::HumanAuthored { author: "ops".into() },
/// ).unwrap();
/// assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
/// ```
#[derive(Debug, Default)]
pub struct InMemoryToolRegistry {
    by_key: BTreeMap<(ToolName, ToolVersion), RegistrationRecord>,
    by_token: BTreeMap<RegistrationToken, (ToolName, ToolVersion)>,
}

impl InMemoryToolRegistry {
    /// Construct an empty registry. Use [`with_builtins`] for a registry
    /// seeded with the OSS built-in tool set.
    ///
    /// [`with_builtins`]: InMemoryToolRegistry::with_builtins
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a registry seeded with the OSS built-in tools (`fs-read`,
    /// `fs-write`, `text-summarize`). All built-ins are `HumanAuthored` (the
    /// kortecx maintainers) and `Approved` on creation.
    ///
    /// The built-in set is intentionally small in v0.1 — a sample for tests
    /// and the runtime demo. Real deployments accrete custom tools.
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        let author = "kortecx-oss".to_string();

        let empty_ceiling = ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        };

        // fs-read: reads from /input (under the warrant's fs_scope).
        let _ = reg.register(
            ToolDef {
                tool_id: ToolName("fs-read".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: empty_ceiling,
                },
                description: "Read bytes from a path declared in the warrant's fs_scope.".into(),
            },
            ToolProvenance::HumanAuthored {
                author: author.clone(),
            },
        );

        // fs-write: writes to /output.
        let _ = reg.register(
            ToolDef {
                tool_id: ToolName("fs-write".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: empty_ceiling,
                },
                description: "Write bytes to a path declared in the warrant's fs_scope.".into(),
            },
            ToolProvenance::HumanAuthored {
                author: author.clone(),
            },
        );

        // text-summarize: deterministic text transformation.
        let _ = reg.register(
            ToolDef {
                tool_id: ToolName("text-summarize".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: empty_ceiling,
                },
                description: "Deterministic text summarization heuristic.".into(),
            },
            ToolProvenance::HumanAuthored { author },
        );

        reg
    }

    /// Count of registrations in the registry (any status).
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// `true` iff the registry has no registrations.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

impl ToolRegistry for InMemoryToolRegistry {
    fn lookup(&self, tool_id: &ToolName, tool_version: &ToolVersion) -> Option<ToolDef> {
        let key = (tool_id.clone(), tool_version.clone());
        self.by_key.get(&key).and_then(|rec| match rec.status {
            RegistrationStatus::Approved => Some(rec.def.clone()),
            RegistrationStatus::PendingHumanReview => None,
        })
    }

    #[tracing::instrument(level = "debug", skip_all, fields(tool_id = ?grant.tool_id, tool_version = ?grant.tool_version))]
    fn resolve(
        &self,
        grant: &ToolGrant,
        warrant: &WarrantSpec,
    ) -> Result<ResolvedTool, ResolutionError> {
        let key = (grant.tool_id.clone(), grant.tool_version.clone());
        let rec = self
            .by_key
            .get(&key)
            .ok_or_else(|| ResolutionError::NotFound {
                tool_id: grant.tool_id.clone(),
                tool_version: grant.tool_version.clone(),
            })?;

        // INERT registrations refuse with a typed error.
        if matches!(rec.status, RegistrationStatus::PendingHumanReview) {
            let token = registration_token_of(&rec.def, &rec.provenance);
            return Err(ResolutionError::PendingHumanReview { token });
        }

        // Subset check per axis: tool.required_capability ⊆ warrant.
        if let Err(denied) = check_tool_requirement(&rec.def.required_capability, warrant) {
            return Err(ResolutionError::CapabilityExceedsWarrant { axis: denied.field });
        }

        // Build the content-addressed resolution event.
        let resolved_def_hash = {
            let bytes = bincode::serde::encode_to_vec(&rec.def, canonical_config())
                .expect("canonical bincode encoding of ToolDef cannot fail");
            ContentRef::of(&bytes)
        };
        let event = ToolResolutionEvent {
            tool_id: grant.tool_id.clone(),
            tool_version: grant.tool_version.clone(),
            resolved_kind: rec.def.kind.clone(),
            resolved_def_hash,
        };
        let event_ref = event.to_ref();

        Ok(ResolvedTool {
            def: rec.def.clone(),
            event,
            event_ref,
            effective_capability: rec.def.required_capability.clone(),
        })
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn register(
        &mut self,
        def: ToolDef,
        provenance: ToolProvenance,
    ) -> Result<RegistrationToken, RegistrationError> {
        let token = registration_token_of(&def, &provenance);
        let status = match &provenance {
            ToolProvenance::HumanAuthored { .. } => RegistrationStatus::Approved,
            ToolProvenance::SelfGenerated { .. } => RegistrationStatus::PendingHumanReview,
        };
        let key = (def.tool_id.clone(), def.tool_version.clone());
        let approved_by = None;
        self.by_key.insert(
            key.clone(),
            RegistrationRecord {
                def,
                provenance,
                status,
                approved_by,
            },
        );
        self.by_token.insert(token, key);
        Ok(token)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn approve_registration(
        &mut self,
        token: RegistrationToken,
        approver: ReviewerId,
    ) -> Result<(), RegistrationError> {
        let key = self
            .by_token
            .get(&token)
            .cloned()
            .ok_or(RegistrationError::UnknownToken { token })?;
        let rec = self
            .by_key
            .get_mut(&key)
            .expect("by_token and by_key are kept in sync");

        match rec.status {
            RegistrationStatus::Approved => {
                return Err(RegistrationError::AlreadyApproved { token });
            }
            RegistrationStatus::PendingHumanReview => {}
        }

        // Anti-privilege-laundering: enforce lineage subset.
        let lineage_warrant = match &rec.provenance {
            ToolProvenance::HumanAuthored { .. } => {
                // Shouldn't reach here in v0.1 — HumanAuthored is Approved on
                // register. Surface as a typed error if a future code path
                // forces it.
                return Err(RegistrationError::NotPendingReview { token });
            }
            ToolProvenance::SelfGenerated {
                generating_lineage_warrant,
                ..
            } => generating_lineage_warrant.clone(),
        };

        if let Err(denied) = check_tool_requirement(&rec.def.required_capability, &lineage_warrant)
        {
            return Err(RegistrationError::InvalidLineageSubset { axis: denied.field });
        }

        rec.status = RegistrationStatus::Approved;
        rec.approved_by = Some(approver);
        Ok(())
    }
}

// Re-exports for downstream ergonomic use.
pub use kx_warrant::{FsScope, NetScope, ResourceCeiling};

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::{ExecutorClass, FsMode, Host, ModelRoute, MoteClass};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

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
            net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host(
                "api.example.com:443".into(),
            )])),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            tool_grants: BTreeSet::new(),
            model_route: ModelRoute {
                model_id: kx_mote::ModelId("m".into()),
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

    fn sample_def(id: &str, version: &str, kind: ToolKind, req: ToolRequirement) -> ToolDef {
        ToolDef {
            tool_id: ToolName(id.into()),
            tool_version: ToolVersion(version.into()),
            kind,
            required_capability: req,
            description: format!("test tool {id}@{version}"),
        }
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

    // -----------------------------------------------------------------
    // lookup
    // -----------------------------------------------------------------

    #[test]
    fn lookup_returns_none_on_empty_registry() {
        let reg = InMemoryToolRegistry::new();
        assert!(reg
            .lookup(&ToolName("nope".into()), &ToolVersion("1".into()))
            .is_none());
    }

    #[test]
    fn lookup_returns_some_after_human_register() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("t", "1", ToolKind::Builtin, permissive_req());
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        assert_eq!(reg.lookup(&def.tool_id, &def.tool_version), Some(def));
    }

    #[test]
    fn lookup_returns_none_during_pending_human_review() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("self-gen", "1", ToolKind::Builtin, permissive_req());
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: permissive_warrant(),
                    generating_mote: MoteId([0; 32]),
                },
            )
            .unwrap();
        assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
    }

    // -----------------------------------------------------------------
    // resolve — happy path
    // -----------------------------------------------------------------

    #[test]
    fn resolve_succeeds_under_permissive_warrant() {
        let mut reg = InMemoryToolRegistry::with_builtins();
        let _ = reg
            .register(
                sample_def("custom", "1", ToolKind::Builtin, permissive_req()),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        let grant = ToolGrant {
            tool_id: ToolName("custom".into()),
            tool_version: ToolVersion("1".into()),
        };
        let resolved = reg.resolve(&grant, &permissive_warrant()).unwrap();
        assert_eq!(resolved.def.tool_id.0, "custom");
        assert_eq!(resolved.event.tool_id.0, "custom");
        // event_ref == event.to_ref() — sanity.
        assert_eq!(resolved.event_ref, resolved.event.to_ref());
    }

    #[test]
    fn resolve_event_is_deterministic() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("det", "1", ToolKind::Builtin, permissive_req());
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id.clone(),
            tool_version: def.tool_version.clone(),
        };
        let a = reg.resolve(&grant, &permissive_warrant()).unwrap();
        let b = reg.resolve(&grant, &permissive_warrant()).unwrap();
        assert_eq!(a.event_ref, b.event_ref);
    }

    // -----------------------------------------------------------------
    // resolve — refusals
    // -----------------------------------------------------------------

    #[test]
    fn resolve_not_found() {
        let reg = InMemoryToolRegistry::new();
        let grant = ToolGrant {
            tool_id: ToolName("missing".into()),
            tool_version: ToolVersion("1".into()),
        };
        assert!(matches!(
            reg.resolve(&grant, &permissive_warrant()),
            Err(ResolutionError::NotFound { .. })
        ));
    }

    #[test]
    fn resolve_pending_review_refused() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("pending", "1", ToolKind::Builtin, permissive_req());
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: permissive_warrant(),
                    generating_mote: MoteId([0; 32]),
                },
            )
            .unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id,
            tool_version: def.tool_version,
        };
        assert!(matches!(
            reg.resolve(&grant, &permissive_warrant()),
            Err(ResolutionError::PendingHumanReview { .. })
        ));
    }

    #[test]
    fn resolve_capability_exceeds_warrant_on_fs_scope() {
        let mut reg = InMemoryToolRegistry::new();
        let mut req = permissive_req();
        // Tool requires /etc read, but the warrant doesn't grant it.
        req.fs_scope_required = FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/etc"), FsMode::ReadOnly)]),
        };
        let def = sample_def("fs-overreach", "1", ToolKind::Builtin, req);
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id,
            tool_version: def.tool_version,
        };
        assert!(matches!(
            reg.resolve(&grant, &permissive_warrant()),
            Err(ResolutionError::CapabilityExceedsWarrant {
                axis: WarrantField::FsScope
            })
        ));
    }

    #[test]
    fn resolve_mcp_under_none_egress_refused() {
        // A warrant with net_scope = None.
        let mut warrant = permissive_warrant();
        warrant.net_scope = NetScope::None;

        let mut reg = InMemoryToolRegistry::new();
        let mut req = permissive_req();
        req.net_scope_required =
            NetScope::EgressAllowlist(BTreeSet::from([Host("mcp.example.com:443".into())]));
        let def = sample_def(
            "mcp-tool",
            "1",
            ToolKind::Mcp {
                endpoint: McpEndpointId("mcp-endpoint-1".into()),
                remote_name: "summarize".into(),
            },
            req,
        );
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id,
            tool_version: def.tool_version,
        };
        assert!(matches!(
            reg.resolve(&grant, &warrant),
            Err(ResolutionError::CapabilityExceedsWarrant {
                axis: WarrantField::NetScope
            })
        ));
    }

    #[test]
    fn resolve_mcp_with_matching_egress_succeeds() {
        let mut warrant = permissive_warrant();
        warrant.net_scope =
            NetScope::EgressAllowlist(BTreeSet::from([Host("mcp.example.com:443".into())]));

        let mut reg = InMemoryToolRegistry::new();
        let mut req = permissive_req();
        req.net_scope_required =
            NetScope::EgressAllowlist(BTreeSet::from([Host("mcp.example.com:443".into())]));
        let def = sample_def(
            "mcp-tool",
            "1",
            ToolKind::Mcp {
                endpoint: McpEndpointId("mcp-endpoint-1".into()),
                remote_name: "summarize".into(),
            },
            req,
        );
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        let grant = ToolGrant {
            tool_id: def.tool_id,
            tool_version: def.tool_version,
        };
        assert!(reg.resolve(&grant, &warrant).is_ok());
    }

    // -----------------------------------------------------------------
    // register: provenance routing
    // -----------------------------------------------------------------

    #[test]
    fn register_human_is_approved_immediately() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("h", "1", ToolKind::Builtin, permissive_req());
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
            )
            .unwrap();
        assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
    }

    #[test]
    fn register_self_gen_is_pending() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("s", "1", ToolKind::Builtin, permissive_req());
        let _ = reg
            .register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: permissive_warrant(),
                    generating_mote: MoteId([0; 32]),
                },
            )
            .unwrap();
        assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
    }

    #[test]
    fn register_token_is_deterministic() {
        let mut reg1 = InMemoryToolRegistry::new();
        let mut reg2 = InMemoryToolRegistry::new();
        let def = sample_def("d", "1", ToolKind::Builtin, permissive_req());
        let prov = ToolProvenance::HumanAuthored {
            author: "ops".into(),
        };
        let t1 = reg1.register(def.clone(), prov.clone()).unwrap();
        let t2 = reg2.register(def, prov).unwrap();
        assert_eq!(t1, t2);
    }

    // -----------------------------------------------------------------
    // approve_registration: subset check + status flips
    // -----------------------------------------------------------------

    #[test]
    fn approve_self_gen_within_lineage_ok() {
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("g", "1", ToolKind::Builtin, permissive_req());
        let token = reg
            .register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: permissive_warrant(),
                    generating_mote: MoteId([1; 32]),
                },
            )
            .unwrap();
        // Permissive req fits inside permissive warrant.
        reg.approve_registration(token, ReviewerId("alice".into()))
            .unwrap();
        assert!(reg.lookup(&def.tool_id, &def.tool_version).is_some());
    }

    #[test]
    fn approve_self_gen_exceeding_lineage_refused() {
        let mut reg = InMemoryToolRegistry::new();
        let mut req = permissive_req();
        req.min_resource_ceiling.mem_bytes = 1 << 50; // wider than lineage warrant
        let def = sample_def("greedy", "1", ToolKind::Builtin, req);
        let token = reg
            .register(
                def.clone(),
                ToolProvenance::SelfGenerated {
                    generating_lineage_warrant: permissive_warrant(),
                    generating_mote: MoteId([2; 32]),
                },
            )
            .unwrap();
        assert!(matches!(
            reg.approve_registration(token, ReviewerId("alice".into())),
            Err(RegistrationError::InvalidLineageSubset {
                axis: WarrantField::ResourceCeiling
            })
        ));
        // Still INERT after refused approval.
        assert!(reg.lookup(&def.tool_id, &def.tool_version).is_none());
    }

    #[test]
    fn approve_unknown_token_refused() {
        let mut reg = InMemoryToolRegistry::new();
        let fake = RegistrationToken(ContentRef::from_bytes([0; 32]));
        assert!(matches!(
            reg.approve_registration(fake, ReviewerId("alice".into())),
            Err(RegistrationError::UnknownToken { .. })
        ));
    }

    #[test]
    fn approve_human_authored_refused_as_not_pending() {
        // A HumanAuthored registration is Approved on register; the registry
        // returns AlreadyApproved if the operator calls approve on its token.
        let mut reg = InMemoryToolRegistry::new();
        let def = sample_def("h", "1", ToolKind::Builtin, permissive_req());
        let prov = ToolProvenance::HumanAuthored {
            author: "ops".into(),
        };
        let token = reg.register(def, prov).unwrap();
        assert!(matches!(
            reg.approve_registration(token, ReviewerId("alice".into())),
            Err(RegistrationError::AlreadyApproved { .. })
        ));
    }

    // -----------------------------------------------------------------
    // with_builtins
    // -----------------------------------------------------------------

    #[test]
    fn with_builtins_seeds_three_tools() {
        let reg = InMemoryToolRegistry::with_builtins();
        assert_eq!(reg.len(), 3);
        assert!(!reg.is_empty());
        assert!(reg
            .lookup(&ToolName("fs-read".into()), &ToolVersion("1".into()))
            .is_some());
        assert!(reg
            .lookup(&ToolName("fs-write".into()), &ToolVersion("1".into()))
            .is_some());
        assert!(reg
            .lookup(&ToolName("text-summarize".into()), &ToolVersion("1".into()))
            .is_some());
    }

    // -----------------------------------------------------------------
    // ToolResolutionEvent::to_ref deterministic + sensitive to changes
    // -----------------------------------------------------------------

    #[test]
    fn tool_resolution_event_ref_is_deterministic() {
        let event = ToolResolutionEvent {
            tool_id: ToolName("t".into()),
            tool_version: ToolVersion("1".into()),
            resolved_kind: ToolKind::Builtin,
            resolved_def_hash: ContentRef::from_bytes([0; 32]),
        };
        assert_eq!(event.to_ref(), event.to_ref());
    }

    #[test]
    fn tool_resolution_event_ref_changes_with_kind() {
        let e1 = ToolResolutionEvent {
            tool_id: ToolName("t".into()),
            tool_version: ToolVersion("1".into()),
            resolved_kind: ToolKind::Builtin,
            resolved_def_hash: ContentRef::from_bytes([0; 32]),
        };
        let e2 = ToolResolutionEvent {
            tool_id: ToolName("t".into()),
            tool_version: ToolVersion("1".into()),
            resolved_kind: ToolKind::External {
                source_url: "https://example.com/t".into(),
            },
            resolved_def_hash: ContentRef::from_bytes([0; 32]),
        };
        assert_ne!(e1.to_ref(), e2.to_ref());
    }
}
