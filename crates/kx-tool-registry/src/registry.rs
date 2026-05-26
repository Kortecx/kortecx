//! [`ToolRegistry`] trait (the D32 seam — OSS impl vs hosted impl behind the
//! same trait) + the OSS-default [`InMemoryToolRegistry`] implementation.

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_mote::{canonical_config, ToolName, ToolVersion};
use kx_warrant::{
    check_tool_requirement, FsScope, NetScope, ResourceCeiling, ToolGrant, WarrantSpec,
};

use crate::errors::{RegistrationError, ResolutionError};
use crate::idempotency_class::IdempotencyClass;
use crate::ids::{RegistrationToken, ReviewerId};
use crate::provenance::{RegistrationStatus, ToolProvenance};
use crate::token::registration_token_of;
use crate::tool_def::{ResolvedTool, ToolDef, ToolResolutionEvent};
use crate::tool_kind::ToolKind;

/// The registry seam (D32). OSS ships [`InMemoryToolRegistry`]; the cloud-side
/// hosted impl plugs in behind the same trait per D28.
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
    ///     IdempotencyClass, InMemoryToolRegistry, ResolutionError, ReviewerId,
    ///     ToolDef, ToolKind, ToolProvenance, ToolRegistry,
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
    ///     idempotency_class: IdempotencyClass::Token,
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
///     IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind,
///     ToolProvenance, ToolRegistry,
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
///     idempotency_class: IdempotencyClass::Readback,
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

        // fs-read: reads bytes from a path declared in the warrant's fs_scope.
        // Read-only operation; naturally idempotent. IdempotencyClass::Readback
        // is the natural fit — the dispatch IS the probe; re-dispatch is safe
        // because reads don't mutate state.
        let _ = reg.register(
            ToolDef {
                tool_id: ToolName("fs-read".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: kx_warrant::ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: empty_ceiling,
                },
                description: "Read bytes from a path declared in the warrant's fs_scope. Read-only; naturally idempotent.".into(),
                idempotency_class: IdempotencyClass::Readback,
            },
            ToolProvenance::HumanAuthored {
                author: author.clone(),
            },
        );

        // fs-write: writes bytes to a path declared in the warrant's fs_scope.
        //
        // SEMANTICS LOCKED TO OVERWRITE-ONLY, FULL-CONTENT writes. The tool
        // accepts the COMPLETE intended file content and replaces the file at
        // the target path atomically (open-write-rename). It DOES NOT support
        // append mode or any partial-write semantics. This is the precondition
        // that makes IdempotencyClass::Staged safe: re-dispatch after a
        // pre-commit crash writes the same complete bytes to the same path,
        // producing the same final state — idempotent.
        //
        // If a future workflow needs append semantics, the answer is a
        // separate tool (e.g., `fs-append`) with `IdempotencyClass::Readback`
        // (probe the file's current length/contents before deciding to write)
        // or `IdempotencyClass::AtLeastOnce` (explicit author ack required).
        // Append-mode under `Staged` would double-write on re-dispatch.
        //
        // The `Staged` class itself is DECLARED HERE BUT NOT YET ENFORCED at
        // runtime — see IdempotencyClass::Staged docs. PR 7 (kx-journal v1→v2
        // adds the EffectStaged kind) + PR 9 (kx-executor wires the protocol)
        // close the runtime contract. Today's resolver returns the resolved
        // tool correctly; only the recovery-time re-dispatch refusal is
        // pending.
        let _ = reg.register(
            ToolDef {
                tool_id: ToolName("fs-write".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: kx_warrant::ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: empty_ceiling,
                },
                description: "Write the complete intended file content to a path declared in the warrant's fs_scope (overwrite-only; no append; staged-intent dispatch).".into(),
                idempotency_class: IdempotencyClass::Staged,
            },
            ToolProvenance::HumanAuthored {
                author: author.clone(),
            },
        );

        // text-summarize: pure transformation (input bytes → summarized
        // string). IdempotencyClass::Readback fits — for a deterministic
        // pure-transformation tool, "probe + skip if applied" collapses to
        // "the journal already has a Committed result_ref for this Mote",
        // which the executor's memoizer (P1.7.9) handles via the same cache
        // lookup path; the tool's idempotency-class declaration is the
        // dispatch-protocol signal, not a separate cache.
        let _ = reg.register(
            ToolDef {
                tool_id: ToolName("text-summarize".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: kx_warrant::ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: empty_ceiling,
                },
                description: "Deterministic text summarization heuristic. Pure transformation; naturally idempotent.".into(),
                idempotency_class: IdempotencyClass::Readback,
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
