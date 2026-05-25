// SPDX-License-Identifier: Apache-2.0
//! `kx-capability` — the capability-broker seam (D24).
//!
//! **The broker is the executor's sole route to every effect a workflow author
//! declared via [`MoteDef.tool_contract`][kx_mote::MoteDef::tool_contract] —
//! every external-system call, file write, MCP invocation, terminal
//! interaction.** The executor does not call external systems directly; it
//! asks the broker to do so on its behalf, and receives back a commit-ready
//! handle the executor uses to assemble the producer Mote's `Committed`
//! journal entry.
//!
//! # The four sub-decisions of D24
//!
//! 1. **Trait surface.** [`Capability`] (name, version, supported patterns,
//!    invoke, optional readback probe), [`CapabilityBroker`]
//!    ([`dispatch`][CapabilityBroker::dispatch],
//!    [`probe_readback`][CapabilityBroker::probe_readback]),
//!    [`BrokerHandle`] (a content-addressed staged ref + capability
//!    identity), [`EffectRequest`] (opaque payload + pattern +
//!    idempotency key + per-call net/fs scope), [`BrokerError`] (the typed
//!    refusal vocabulary).
//! 2. **P1 OSS impl is a trivial pass-through:** [`LocalCapabilityBroker`].
//!    In-process dispatch; no sandboxing; single-tenant. The point is the
//!    seam, not the isolation.
//! 3. **P5 cloud impl** is hardened bubblewrap + seccomp + per-tenant
//!    behind the **same trait** — never a fork. Lives in `kx-cloud` per
//!    D28.
//! 4. **Boundary with the resource manager (D25):** the broker handles
//!    workflow-declared effects (anything nameable in
//!    `MoteDef.tool_contract`); the resource manager handles runtime
//!    self-resources (cgroups, GPU, KV-cache). No third path; no fused
//!    calls.
//!
//! # Composition with D29 (model validator) and D30 (warrant)
//!
//! - **Validator + broker are non-overlapping seams.** The validator
//!   (`kx-model-validator`, D29) is the pre-bind interface check (does
//!   the model even have the required capabilities). The broker is the
//!   pre-effect contract check at dispatch time. The broker assumes the
//!   validator has already refused obvious type errors at bind time and
//!   runs the tighter per-call contract; it is also the **runtime
//!   backstop** for false declarations the v1 validator trusts
//!   (`Soundness::Declared`).
//! - **Warrant + broker are non-overlapping seams on the same axis at
//!   different lifecycle points.** The warrant (`kx-warrant`, D30) is the
//!   outer envelope: which capabilities may be used, with what
//!   net_scope / fs_scope / resource ceiling. The broker enforces the
//!   per-call contract within that envelope: every dispatch's capability
//!   must be in `warrant.tool_grants`; its declared net/fs scope must be
//!   ⊆ `warrant.*`. Violations surface as
//!   [`BrokerError::CapabilityExceedsWarrant`].
//!
//! # Recovery-state independence (D40 + STEP 5.4 layering pin)
//!
//! **The broker MUST NOT depend on `kx-projection` or `kx-journal`.**
//! Recovery re-dispatch decisions are the executor's; the broker is
//! stateless with respect to recovery and dispatches whatever
//! [`EffectRequest`] the executor hands it. This crate's `Cargo.toml`
//! declares NEITHER as a dependency; this is the structural enforcement
//! of the invariant.
//!
//! # D38 token plumbing
//!
//! - **§1 — `IdempotencyClass::Token` tools:** the executor sets
//!   `EffectRequest.idempotency_key = Some(*mote.id.as_bytes())` via the
//!   [`idempotency_token_for`] helper. The remote tool's idempotency
//!   contract is the runtime backstop for the effect→commit crash window.
//! - **§2a — `IdempotencyClass::Readback` tools:** the executor calls
//!   [`CapabilityBroker::probe_readback`] before dispatch. A `Some(handle)`
//!   return means the effect is already applied (use the existing
//!   `staged_ref`); a `None` return means proceed to dispatch.
//! - **§2b — `IdempotencyClass::Staged` tools:** the executor writes
//!   `EffectStaged` to the journal BEFORE calling broker.dispatch; the
//!   broker is unaware of this. The broker only dispatches what the
//!   executor hands it.
//! - **§2c — `IdempotencyClass::AtLeastOnce` tools:** the executor
//!   refuses to dispatch unless the workflow submission context carries
//!   `accept_at_least_once = true`. The broker does NOT read the
//!   submission context; refusal happens at the executor's submission-time
//!   predicate, BEFORE broker.dispatch is called.
//!
//! # Reading further
//!
//! - `docs/design/capability-broker.md` (private corpus) — the locked
//!   spec for D24, including dispatch flow §5, token plumbing §5.5,
//!   readback/staged dispatch §5.6, AtLeastOnce refusal §5.7.
//! - `docs/design/decisions.md` D24 (broker seam), D38 (tool-boundary
//!   idempotency), D30 (warrant + monotonic narrowing).
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// TODO(workspace.lints cleanup): kx-capability uses `.expect()` on RwLock
// `read()` / `write()` in `LocalCapabilityBroker`. These are documented
// infallible at the call sites (poisoning is only possible if a prior
// registration panicked while holding the write lock; the OSS impl
// performs no fallible work under the lock). Follow-up cleanup PR
// migrates to a typed `BrokerError::RegistryPoisoned` variant or to
// `parking_lot::RwLock` (no poisoning). Until then, the documented
// `expect(...)` is the audit trail. Pattern matches the existing
// kx-warrant precedent (its workspace.lints comment in Cargo.toml).
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
// Inline test modules use unwrap freely; expect is already allowed at
// crate level for the RwLock-poison sites. Integration tests under
// tests/*.rs carry their own per-file allow as usual.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::collections::BTreeMap;
use std::sync::RwLock;

use kx_content::{ContentRef, ContentStore};
use kx_mote::{EffectPattern, Mote, ToolName, ToolVersion};
use kx_warrant::{FsScope, NetScope, ToolGrant, WarrantField, WarrantSpec};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Helper: D38 §1 — derive the 32-byte idempotency token from a Mote
// ---------------------------------------------------------------------------

/// Derive the 32-byte idempotency token from a [`Mote`]'s identity.
///
/// Per D38 §1, the broker passes this token through to a remote tool's
/// idempotency header (e.g., `Idempotency-Key: <hex>`) so that a recovery
/// re-dispatch of the same Mote produces the SAME token; the remote API
/// then returns the cached response and no double-effect occurs. The
/// 32-byte form is the raw [`MoteId`][kx_mote::MoteId] bytes; the caller
/// is free to hex-encode them per the remote API's wire format (e.g.,
/// `blake3::Hash::from_bytes(token).to_hex()`).
///
/// # Example
///
/// ```
/// use kx_capability::idempotency_token_for;
/// use kx_mote::{
///     EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
///     MoteDef, NdClass, PromptTemplateHash,
/// };
/// use smallvec::SmallVec;
/// use std::collections::BTreeMap;
///
/// let def = MoteDef {
///     logic_ref: LogicRef::from_bytes([0u8; 32]),
///     model_id: ModelId("m".into()),
///     prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
///     tool_contract: BTreeMap::new(),
///     nd_class: NdClass::Pure,
///     config_subset: BTreeMap::new(),
///     effect_pattern: EffectPattern::IdempotentByConstruction,
///     critic_for: None,
///     is_topology_shaper: false,
///     schema_version: 3,
/// };
/// let mote = Mote::new(
///     def,
///     InputDataId::from_bytes([0u8; 32]),
///     GraphPosition("/root".into()),
///     SmallVec::new(),
/// );
/// let token = idempotency_token_for(&mote);
/// assert_eq!(token.len(), 32);
/// assert_eq!(&token, mote.id.as_bytes());
/// ```
#[inline]
#[must_use]
pub fn idempotency_token_for(mote: &Mote) -> [u8; 32] {
    *mote.id.as_bytes()
}

// ---------------------------------------------------------------------------
// EffectRequest, BrokerHandle, BrokerError
// ---------------------------------------------------------------------------

/// The opaque payload the broker passes through to a capability.
///
/// Per `capability-broker.md` §4, the broker does NOT interpret these
/// bytes — it routes them to the named capability after enforcing the
/// per-call contract. Per-capability typed bindings will live in the
/// Quorum SDK (P4.1) and produce these `EffectRequest`s at workflow
/// submission time.
///
/// The `net_scope` and `fs_scope` fields declare the access this dispatch
/// requires; the broker enforces that they are subsets of the active
/// warrant's corresponding axes (D30 composition).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectRequest {
    /// Opaque payload the named capability understands.
    pub payload: Vec<u8>,
    /// The effect pattern the executor is dispatching this Mote under
    /// (per `MoteDef.effect_pattern`).
    pub pattern: EffectPattern,
    /// For `IdempotencyClass::Token` tools: the 32-byte idempotency token
    /// derived from the Mote's identity. Set by the executor via
    /// [`idempotency_token_for`]. Required for token-class WM tools
    /// (executor predicate R-10); optional for other classes.
    pub idempotency_key: Option<[u8; 32]>,
    /// The network egress this dispatch requires. Must be a subset of
    /// `warrant.net_scope`; otherwise the broker refuses with
    /// [`BrokerError::CapabilityExceedsWarrant`] on
    /// [`WarrantField::NetScope`].
    pub net_scope: NetScope,
    /// The filesystem access this dispatch requires. Must be a subset of
    /// `warrant.fs_scope`; otherwise the broker refuses with
    /// [`BrokerError::CapabilityExceedsWarrant`] on
    /// [`WarrantField::FsScope`].
    pub fs_scope: FsScope,
}

/// The broker's commit-ready artifact for a dispatched effect.
///
/// The executor uses this to assemble the producer Mote's `Committed`
/// journal entry per the dispatch flow in `capability-broker.md` §5:
///
/// - For `IdempotentByConstruction`: `staged_ref` is the response payload
///   (already realized externally and content-addressed by the store).
/// - For `StageThenCommit`: `staged_ref` is the staged ContentRef the
///   executor will commit as `result_ref`.
/// - For `ValidateThenCommit`: `staged_ref` is the staged proposal
///   ContentRef the critic Mote will validate.
///
/// The capability identity is recorded for the audit trail; per the spec
/// it is written into the content store entry's metadata (out of scope
/// for this crate) and NOT into the journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerHandle {
    /// The `ContentRef` where the broker staged the effect's payload.
    pub staged_ref: ContentRef,
    /// The capability that produced this handle (audit trail).
    pub capability: ToolName,
    /// The version of that capability (pinned at dispatch).
    pub capability_version: ToolVersion,
}

/// A capability returning a typed failure reason to the broker.
///
/// The broker wraps these into [`BrokerError::CapabilityFailure`] before
/// surfacing them upward. The executor consults the Mote's `nd_class`
/// retry budget (per `stuck-vs-dead.md`, D21) to decide whether a
/// failed dispatch may be retried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityFailureReason {
    /// Authentication was denied by the external system.
    AuthDenied,
    /// The external system rate-limited this dispatch.
    RateLimited,
    /// The external system was unreachable (network failure, DNS, etc.).
    NetworkUnreachable,
    /// The dispatch exceeded the per-call wall-clock budget.
    Timeout,
    /// The response was malformed or did not match the expected shape.
    InvalidResponse,
    /// Other capability-defined reason; opaque string for diagnostics.
    Other(String),
}

/// The broker's typed refusal vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BrokerError {
    /// The named capability is not in `mote.def.tool_contract` — the
    /// workflow author did not declare it. Per the spec this is a
    /// workflow-author error surfaced as a refused dispatch; the executor
    /// reads this as a `FailureReason::UnsafeWorldMutatingConstruction`
    /// at runtime (R-1 extension per `validate-then-commit.md` §7).
    #[error("capability `{}/{}` not in Mote.tool_contract", name.0, "")]
    UnknownCapability {
        /// The capability whose dispatch was refused.
        name: ToolName,
    },
    /// The capability does not honor the requested
    /// [`EffectPattern`]. For example, dispatching a
    /// `ValidateThenCommit` pattern against a capability whose
    /// `supported_patterns()` is `[IdempotentByConstruction]`.
    #[error(
        "capability `{}/{}` does not honor pattern {:?}",
        capability.0, "", requested
    )]
    UnsupportedPattern {
        /// The capability whose dispatch was refused.
        capability: ToolName,
        /// The pattern the executor asked for.
        requested: EffectPattern,
    },
    /// The dispatch exceeds the active warrant on the named axis (D30
    /// composition). One of:
    ///
    /// - [`WarrantField::ToolGrants`] — capability not in
    ///   `warrant.tool_grants`.
    /// - [`WarrantField::NetScope`] — `request.net_scope` not ⊆
    ///   `warrant.net_scope`.
    /// - [`WarrantField::FsScope`] — `request.fs_scope` not ⊆
    ///   `warrant.fs_scope`.
    #[error("capability dispatch exceeds warrant on axis {axis:?}")]
    CapabilityExceedsWarrant {
        /// The warrant axis the dispatch exceeded.
        axis: WarrantField,
    },
    /// The capability itself returned an error (auth, rate limit,
    /// downstream failure). The executor decides whether retries are
    /// permitted per the Mote's `nd_class` retry budget (D21).
    #[error("capability `{}/{}` failed: {reason:?}", capability.0, "")]
    CapabilityFailure {
        /// The capability whose invocation failed.
        capability: ToolName,
        /// The capability-defined failure reason.
        reason: CapabilityFailureReason,
    },
    /// The sandboxing layer (P5 hardened impl) refused dispatch. The
    /// trivial OSS impl never raises this; the variant exists so the
    /// trait's refusal vocabulary is forward-compatible.
    #[error("sandbox refused dispatch of `{}/{}`: {reason}", capability.0, "")]
    SandboxRefused {
        /// The capability whose dispatch was refused.
        capability: ToolName,
        /// The sandbox-defined reason string.
        reason: String,
    },
    /// The content store rejected the staging write for the response
    /// payload. The dispatch did succeed at the capability, but the
    /// broker could not produce a `BrokerHandle` — surfaced so the
    /// executor can journal the failure rather than silently lose the
    /// effect.
    ///
    /// The diagnostic is carried as a `String` (rather than a typed
    /// `#[source]` chain) so `BrokerError` stays decoupled from the
    /// specific `ContentStore` impl's error type; the wider executor
    /// error hierarchy carries richer context.
    #[error("content-store stage write failed for `{}/{}`: {diagnostic}", capability.0, "")]
    StageWriteFailed {
        /// The capability whose payload could not be staged.
        capability: ToolName,
        /// The string-form description of the underlying store error.
        diagnostic: String,
    },
}

// ---------------------------------------------------------------------------
// Capability trait
// ---------------------------------------------------------------------------

/// A `Capability` is a named, versioned external system a Mote may invoke.
///
/// The set of capabilities a Mote may invoke is its
/// `MoteDef.tool_contract` (per `idempotency.md`); the set the runtime
/// will ever dispatch under any warrant is `warrant.tool_grants` (per
/// `warrant.md`). Both checks live in [`CapabilityBroker::dispatch`].
///
/// Capabilities are registered with a broker via
/// [`LocalCapabilityBroker::register_capability`]. The trait is
/// `Send + Sync` so the broker can hold capabilities behind shared
/// references across threads; per-handle thread-safety is the
/// capability's responsibility (most capabilities front a remote API or
/// an isolated subprocess and can satisfy this trivially).
///
/// The default impl of [`probe`][Capability::probe] returns `Ok(None)`
/// (no readback support). Only capabilities backing `Readback`-class
/// tools (D38 §2a) need to override it.
pub trait Capability: Send + Sync {
    /// The capability's registered name.
    fn name(&self) -> &ToolName;

    /// The capability's pinned version. Two capabilities sharing a name
    /// but differing in version are distinct in the broker's registry;
    /// the broker dispatches to the exact `(name, version)` declared in
    /// the workflow's warrant.
    fn version(&self) -> &ToolVersion;

    /// Which `EffectPattern` values this capability can honor (per
    /// `validate-then-commit.md` §4). A Stripe-style API honors
    /// `IdempotentByConstruction`; a filesystem write honors
    /// `StageThenCommit`; an MCP server call may honor
    /// `ValidateThenCommit`.
    fn supported_patterns(&self) -> &[EffectPattern];

    /// Invoke the capability with the given request, producing the
    /// response bytes that the broker will stage into the content store.
    ///
    /// The bytes returned here are what the executor will read on the
    /// committed `result_ref` after the journal commit lands; they are
    /// the effect's externally-observable result, content-addressed.
    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason>;

    /// Probe whether the effect is already applied (D38 §2a — the
    /// deterministic readback contract). Default returns `Ok(None)`
    /// (capability does not support readback; the broker treats this as
    /// "proceed with dispatch").
    ///
    /// A capability backing a `Readback`-class tool overrides this to
    /// query the world state deterministically. `Ok(Some(bytes))` means
    /// "the effect is already applied, here is the response that proves
    /// it"; the broker then stages those bytes and returns the resulting
    /// `BrokerHandle` so the executor commits without re-dispatching.
    ///
    /// **The probe is a deterministic check** (D20 chain-terminator
    /// rule); **never a model call.** Recovery re-runs the probe and
    /// reaches the same skip-or-dispatch decision.
    fn probe(&self, request: &EffectRequest) -> Result<Option<Vec<u8>>, CapabilityFailureReason> {
        let _ = request;
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// CapabilityBroker trait
// ---------------------------------------------------------------------------

/// The executor's sole interface to effects.
///
/// One implementation per deployment shape:
/// - **P1.8.5 (this crate, OSS):** [`LocalCapabilityBroker`] — trivial
///   in-process pass-through.
/// - **P5 (cloud, `kx-cloud`):** hardened bubblewrap + seccomp +
///   per-tenant isolation behind this **same trait** — never a fork.
///
/// The broker is `Send + Sync` so the executor may share a single
/// instance across worker threads via `Arc<dyn CapabilityBroker>`.
pub trait CapabilityBroker: Send + Sync {
    /// Dispatch an effect a Mote has requested.
    ///
    /// The broker:
    /// 1. Verifies the named capability is in `mote.def.tool_contract`
    ///    (refuses [`BrokerError::UnknownCapability`] otherwise).
    /// 2. Verifies the requested [`EffectPattern`] is in the capability's
    ///    `supported_patterns()`
    ///    (refuses [`BrokerError::UnsupportedPattern`] otherwise).
    /// 3. Verifies the capability is in `warrant.tool_grants`
    ///    (refuses
    ///    [`BrokerError::CapabilityExceedsWarrant`]`{`[`WarrantField::ToolGrants`]`}`
    ///    otherwise).
    /// 4. Verifies `request.net_scope` ⊆ `warrant.net_scope` and
    ///    `request.fs_scope` ⊆ `warrant.fs_scope`
    ///    (refuses
    ///    [`BrokerError::CapabilityExceedsWarrant`]`{`[`WarrantField::NetScope`]`}` /
    ///    `{`[`WarrantField::FsScope`]`}` otherwise).
    /// 5. Routes the request to the named capability via
    ///    [`Capability::invoke`].
    /// 6. Stages the response payload to the content store
    ///    (content-addressed; D17).
    /// 7. Returns a [`BrokerHandle`] the executor uses to assemble the
    ///    journal commit.
    ///
    /// The broker NEVER writes the journal (D14 — the executor owns the
    /// commit txn).
    fn dispatch(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError>;

    /// D38 §2a — deterministic readback probe for
    /// `IdempotencyClass::Readback` tools.
    ///
    /// The broker runs the same per-call contract checks as
    /// [`dispatch`][Self::dispatch] (capability in tool_contract,
    /// capability supports pattern, capability in warrant.tool_grants,
    /// request scopes ⊆ warrant scopes), then invokes the capability's
    /// [`probe`][Capability::probe] method:
    ///
    /// - `Ok(Some(handle))` — the effect is already applied; the
    ///   capability's probe returned bytes that the broker has staged
    ///   into the content store. The executor uses `handle.staged_ref`
    ///   as the `result_ref` and SKIPs the dispatch.
    /// - `Ok(None)` — the effect is not yet applied; the executor
    ///   proceeds to call [`dispatch`][Self::dispatch].
    /// - `Err(_)` — the probe itself failed; surfaced like any other
    ///   broker error.
    ///
    /// **Recovery-state independence (capability-broker.md §3):** the
    /// broker does not consult the journal or projection. Recovery
    /// re-dispatch decisions are the executor's responsibility; the
    /// broker only knows how to run the probe and how to dispatch.
    fn probe_readback(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError>;
}

// ---------------------------------------------------------------------------
// LocalCapabilityBroker — the OSS trivial pass-through
// ---------------------------------------------------------------------------

/// The OSS trivial pass-through `CapabilityBroker` impl.
///
/// In-process dispatch; no sandboxing; single-tenant. The point of
/// `kx-capability` is the seam, not the isolation. P5 swaps in a
/// hardened impl behind the same trait with no executor change.
///
/// Capabilities are registered via [`register_capability`][Self::register_capability]
/// after construction. The broker holds them keyed by [`ToolName`] in a
/// `BTreeMap` under a `RwLock`; registration takes the write lock,
/// dispatch takes the read lock.
///
/// The broker stages response payloads into the supplied [`ContentStore`]
/// via [`ContentStore::put`]; the resulting [`ContentRef`] is the
/// `BrokerHandle.staged_ref`. Two dispatches returning byte-identical
/// payloads share the same `staged_ref` (content-addressing dedupes for
/// free — this is the D17 atomicity contract reused).
pub struct LocalCapabilityBroker<S: ContentStore + Send + Sync> {
    store: S,
    capabilities: RwLock<BTreeMap<ToolName, Box<dyn Capability>>>,
}

impl<S: ContentStore + Send + Sync> std::fmt::Debug for LocalCapabilityBroker<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.capabilities.read().map(|c| c.len()).unwrap_or(0);
        f.debug_struct("LocalCapabilityBroker")
            .field("registered_capabilities", &count)
            .finish_non_exhaustive()
    }
}

impl<S: ContentStore + Send + Sync> LocalCapabilityBroker<S> {
    /// Construct a new broker backed by the supplied content store, with
    /// no capabilities registered.
    pub fn new(store: S) -> Self {
        Self {
            store,
            capabilities: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register a capability with the broker. Replaces any prior
    /// registration under the same [`ToolName`] (the broker holds at
    /// most one impl per name; version disambiguation happens at the
    /// warrant-subset check, not at the registry).
    ///
    /// This is the OSS trivial pass-through's registration model;
    /// hardened cloud impls may register through a richer surface (e.g.,
    /// per-tenant allowlists, signed registrations) behind the same
    /// trait.
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned — which is only
    /// possible if a prior `register_capability` panicked while holding
    /// the write lock. The OSS impl performs no fallible work under the
    /// lock, so poisoning indicates a bug to surface loudly rather than
    /// swallow.
    pub fn register_capability(&self, capability: Box<dyn Capability>) {
        let name = capability.name().clone();
        let mut guard = self
            .capabilities
            .write()
            .expect("RwLock poisoned (prior registration panicked)");
        guard.insert(name, capability);
    }

    /// Number of currently-registered capabilities (useful for tests and
    /// startup diagnostics).
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned (see
    /// [`register_capability`][Self::register_capability]).
    pub fn registered_count(&self) -> usize {
        self.capabilities.read().expect("RwLock poisoned").len()
    }

    /// Internal: run the per-call contract checks shared by
    /// `dispatch` and `probe_readback`. Returns the resolved capability
    /// version on success (so the caller can build a `BrokerHandle`
    /// without re-acquiring the lock).
    ///
    /// Associated function (not `&self`) because the checks read only
    /// from the capabilities map handed in and the request — `self` is
    /// not needed.
    fn precheck<'a>(
        capabilities: &'a BTreeMap<ToolName, Box<dyn Capability>>,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability_name: &ToolName,
        request: &EffectRequest,
    ) -> Result<&'a dyn Capability, BrokerError> {
        // (1) capability declared in MoteDef.tool_contract
        if !mote.def.tool_contract.contains_key(capability_name) {
            return Err(BrokerError::UnknownCapability {
                name: capability_name.clone(),
            });
        }

        // Look up the registered capability impl. If absent, we treat as
        // UnknownCapability — the capability is in the tool_contract but
        // no impl is registered with the broker, which is an
        // operationally-equivalent refusal.
        let Some(capability) = capabilities.get(capability_name) else {
            return Err(BrokerError::UnknownCapability {
                name: capability_name.clone(),
            });
        };

        // (2) capability supports the requested pattern
        if !capability.supported_patterns().contains(&request.pattern) {
            return Err(BrokerError::UnsupportedPattern {
                capability: capability_name.clone(),
                requested: request.pattern,
            });
        }

        // (3) capability ∈ warrant.tool_grants (D30 composition)
        let grant = ToolGrant {
            tool_id: capability_name.clone(),
            tool_version: capability.version().clone(),
        };
        if !warrant.tool_grants.contains(&grant) {
            return Err(BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::ToolGrants,
            });
        }

        // (4) request.net_scope ⊆ warrant.net_scope
        if !request.net_scope.is_subset_of(&warrant.net_scope) {
            return Err(BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::NetScope,
            });
        }

        // (4) request.fs_scope ⊆ warrant.fs_scope
        if !request.fs_scope.is_subset_of(&warrant.fs_scope) {
            return Err(BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::FsScope,
            });
        }

        Ok(&**capability)
    }

    /// Internal: stage response bytes to the content store and build a
    /// `BrokerHandle`. Returns `StageWriteFailed` on store error.
    fn stage(
        &self,
        capability_name: &ToolName,
        capability_version: &ToolVersion,
        bytes: Vec<u8>,
    ) -> Result<BrokerHandle, BrokerError> {
        let staged_ref = self
            .store
            .put(&bytes)
            .map_err(|e| BrokerError::StageWriteFailed {
                capability: capability_name.clone(),
                diagnostic: format!("{e}"),
            })?;
        Ok(BrokerHandle {
            staged_ref,
            capability: capability_name.clone(),
            capability_version: capability_version.clone(),
        })
    }
}

impl<S: ContentStore + Send + Sync> CapabilityBroker for LocalCapabilityBroker<S> {
    #[tracing::instrument(
        level = "debug",
        skip(self, mote, warrant, request),
        fields(
            mote_id = %mote.id,
            capability = %capability.0,
            pattern = ?request.pattern,
            has_idempotency_key = request.idempotency_key.is_some(),
        )
    )]
    fn dispatch(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        let (cap_name, cap_version, invocation) = {
            let guard = self.capabilities.read().expect("RwLock poisoned");
            let cap = Self::precheck(&guard, mote, warrant, capability, &request)?;
            let cap_name = cap.name().clone();
            let cap_version = cap.version().clone();
            let invocation = cap.invoke(&request);
            // Drop the read lock BEFORE staging — staging is I/O.
            (cap_name, cap_version, invocation)
        };
        let bytes = invocation.map_err(|reason| BrokerError::CapabilityFailure {
            capability: cap_name.clone(),
            reason,
        })?;
        self.stage(&cap_name, &cap_version, bytes)
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, mote, warrant, probe),
        fields(
            mote_id = %mote.id,
            capability = %capability.0,
            pattern = ?probe.pattern,
        )
    )]
    fn probe_readback(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        let guard = self.capabilities.read().expect("RwLock poisoned");
        let cap = Self::precheck(&guard, mote, warrant, capability, &probe)?;
        let cap_name = cap.name().clone();
        let cap_version = cap.version().clone();
        let probe_outcome = cap.probe(&probe);
        drop(guard);
        let bytes = match probe_outcome {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(None),
            Err(reason) => {
                return Err(BrokerError::CapabilityFailure {
                    capability: cap_name,
                    reason,
                });
            }
        };
        Ok(Some(self.stage(&cap_name, &cap_version, bytes)?))
    }
}

// ---------------------------------------------------------------------------
// Architectural review (SN-4 v2 #9) — recorded inline, not separate doc.
// ---------------------------------------------------------------------------
//
// 1. The broker has no `kx-journal` or `kx-projection` dependency
//    (verify with `cargo tree`). The recovery-state-independence
//    invariant from capability-broker.md §3 is structurally enforced
//    by the Cargo manifest.
// 2. The trait surface admits a future hosted impl: the trait is
//    object-safe and `Send + Sync`; no signature carries an
//    in-process-only type (e.g., no `Arc<Mutex<...>>` over an
//    in-process queue). A future hardened cloud-side broker can
//    implement `CapabilityBroker` with a remote dispatch protocol
//    behind the same trait, and the executor code is unchanged.
// 3. Every `BrokerError` variant is test-reachable: `UnknownCapability`
//    (CAP-2 fixture), `UnsupportedPattern` (CAP-3 fixture),
//    `CapabilityExceedsWarrant` (CAP-6/7 fixtures on three axes),
//    `CapabilityFailure` (CAP-5 fixture), `SandboxRefused` (CAP-10
//    fixture — fixture broker variant returns it via a fake
//    `CapabilityBroker` impl in tests/), `StageWriteFailed` (CAP-11
//    fixture using a failing content-store impl).
// 4. The single-writer registry (the `RwLock` around the BTreeMap)
//    holds the write lock only for the duration of a `BTreeMap::insert`;
//    invocations and probes hold the READ lock for as briefly as
//    possible (drop before any I/O — see `dispatch` and
//    `probe_readback`). This composes with the workspace's
//    concurrency-test discipline (SN-4 v2 #6) which the integration
//    tests exercise.
//
// ---------------------------------------------------------------------------
// Inline unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
    use kx_mote::{
        EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
        PromptTemplateHash, ToolName, ToolVersion,
    };
    use kx_warrant::{
        ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
        ToolGrant, WarrantSpec,
    };
    use smallvec::SmallVec;

    // -- Fixtures -----------------------------------------------------------

    fn tool_name(name: &str) -> ToolName {
        ToolName(name.into())
    }
    fn tool_version(v: &str) -> ToolVersion {
        ToolVersion(v.into())
    }

    /// A capability that returns `payload.iter().rev().collect()` (the
    /// reverse of the input bytes). Deterministic; useful for asserting
    /// staged_ref values.
    struct ReverseCapability {
        name: ToolName,
        version: ToolVersion,
        patterns: Vec<EffectPattern>,
    }

    impl ReverseCapability {
        fn new(name: &str, version: &str, patterns: Vec<EffectPattern>) -> Self {
            Self {
                name: tool_name(name),
                version: tool_version(version),
                patterns,
            }
        }
    }

    impl Capability for ReverseCapability {
        fn name(&self) -> &ToolName {
            &self.name
        }
        fn version(&self) -> &ToolVersion {
            &self.version
        }
        fn supported_patterns(&self) -> &[EffectPattern] {
            &self.patterns
        }
        fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
            Ok(request.payload.iter().rev().copied().collect())
        }
    }

    /// A capability that always fails with the given reason.
    struct FailingCapability {
        name: ToolName,
        version: ToolVersion,
        patterns: Vec<EffectPattern>,
        reason: CapabilityFailureReason,
    }

    impl Capability for FailingCapability {
        fn name(&self) -> &ToolName {
            &self.name
        }
        fn version(&self) -> &ToolVersion {
            &self.version
        }
        fn supported_patterns(&self) -> &[EffectPattern] {
            &self.patterns
        }
        fn invoke(&self, _: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
            Err(self.reason.clone())
        }
    }

    /// A capability whose probe returns Some(bytes) — for CAP-9 testing.
    struct PrimedReadbackCapability {
        name: ToolName,
        version: ToolVersion,
        patterns: Vec<EffectPattern>,
        prerecorded: Vec<u8>,
    }

    impl Capability for PrimedReadbackCapability {
        fn name(&self) -> &ToolName {
            &self.name
        }
        fn version(&self) -> &ToolVersion {
            &self.version
        }
        fn supported_patterns(&self) -> &[EffectPattern] {
            &self.patterns
        }
        fn invoke(&self, _: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
            // probe path should be taken before invoke; if invoke runs,
            // surface a distinct response so the test fails loudly.
            Ok(b"invoke-was-called-but-probe-should-have-fired".to_vec())
        }
        fn probe(&self, _: &EffectRequest) -> Result<Option<Vec<u8>>, CapabilityFailureReason> {
            Ok(Some(self.prerecorded.clone()))
        }
    }

    fn permissive_warrant_with_grant(grant: ToolGrant) -> WarrantSpec {
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
            tool_grants: BTreeSet::from([grant]),
            model_route: ModelRoute {
                model_id: ModelId("m".into()),
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

    /// Build a Mote whose `tool_contract` includes the given (name, version).
    fn mote_with_tool(name: &ToolName, version: &ToolVersion) -> Mote {
        let mut tool_contract = BTreeMap::new();
        tool_contract.insert(name.clone(), version.clone());
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([0u8; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
            tool_contract,
            nd_class: NdClass::WorldMutating,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            schema_version: 3,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([0u8; 32]),
            GraphPosition(b"/root".to_vec()),
            SmallVec::new(),
        )
    }

    fn empty_request_with_pattern(pattern: EffectPattern, payload: Vec<u8>) -> EffectRequest {
        EffectRequest {
            payload,
            pattern,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
        }
    }

    // -- CAP-1 — dispatch returns content-addressed staged_ref -----------

    #[test]
    fn cap_1_dispatch_returns_content_addressed_handle() {
        let name = tool_name("rev");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "rev",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));

        let req = empty_request_with_pattern(EffectPattern::StageThenCommit, b"hello".to_vec());
        let handle = broker
            .dispatch(&mote, &warrant, &name, req)
            .expect("dispatch ok");

        // The reverse of "hello" is "olleh" — the staged_ref is the hash of
        // those bytes.
        let expected = ContentRef::of(b"olleh");
        assert_eq!(handle.staged_ref, expected);
        assert_eq!(handle.capability, name);
        assert_eq!(handle.capability_version, version);
    }

    // -- CAP-2 — capability not in tool_contract → UnknownCapability -----

    #[test]
    fn cap_2_unknown_capability_when_not_in_tool_contract() {
        let known = tool_name("known");
        let known_ver = tool_version("1");
        let mote = mote_with_tool(&known, &known_ver);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: known.clone(),
            tool_version: known_ver.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        // Register the "known" capability but try to dispatch "other".
        broker.register_capability(Box::new(ReverseCapability::new(
            "known",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));
        broker.register_capability(Box::new(ReverseCapability::new(
            "other",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));

        let req = empty_request_with_pattern(EffectPattern::StageThenCommit, vec![]);
        let other = tool_name("other");
        let err = broker
            .dispatch(&mote, &warrant, &other, req)
            .expect_err("dispatch should refuse");
        assert!(matches!(err, BrokerError::UnknownCapability { name } if name == other));
    }

    // -- CAP-3 — capability doesn't support pattern → UnsupportedPattern -

    #[test]
    fn cap_3_unsupported_pattern_when_capability_pattern_disjoint() {
        let name = tool_name("idem-only");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "idem-only",
            "1",
            vec![EffectPattern::IdempotentByConstruction],
        )));

        let req = empty_request_with_pattern(EffectPattern::ValidateThenCommit, vec![]);
        let err = broker
            .dispatch(&mote, &warrant, &name, req)
            .expect_err("dispatch should refuse");
        match err {
            BrokerError::UnsupportedPattern {
                capability,
                requested,
            } => {
                assert_eq!(capability, name);
                assert_eq!(requested, EffectPattern::ValidateThenCommit);
            }
            other => panic!("expected UnsupportedPattern, got {other:?}"),
        }
    }

    // -- CAP-4 — content-addressing dedupes identical responses ----------

    #[test]
    fn cap_4_identical_responses_dedupe_via_content_addressing() {
        let name = tool_name("rev");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "rev",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));

        let req1 = empty_request_with_pattern(EffectPattern::StageThenCommit, b"abc".to_vec());
        let req2 = empty_request_with_pattern(EffectPattern::StageThenCommit, b"abc".to_vec());
        let h1 = broker.dispatch(&mote, &warrant, &name, req1).unwrap();
        let h2 = broker.dispatch(&mote, &warrant, &name, req2).unwrap();
        // Distinct BrokerHandle structs but identical staged_ref because
        // the responses are byte-identical (ReverseCapability is
        // deterministic) — content-addressing dedupes.
        assert_eq!(h1.staged_ref, h2.staged_ref);
    }

    // -- CAP-5 — capability error produces CapabilityFailure -------------

    #[test]
    fn cap_5_capability_failure_no_content_store_write() {
        let name = tool_name("fail");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let store = InMemoryContentStore::new();
        let broker = LocalCapabilityBroker::new(store);
        broker.register_capability(Box::new(FailingCapability {
            name: name.clone(),
            version,
            patterns: vec![EffectPattern::StageThenCommit],
            reason: CapabilityFailureReason::RateLimited,
        }));

        let req = empty_request_with_pattern(EffectPattern::StageThenCommit, b"x".to_vec());
        let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
        assert!(matches!(
            err,
            BrokerError::CapabilityFailure {
                reason: CapabilityFailureReason::RateLimited,
                ..
            }
        ));
        // No write happened — `list_refs()` is empty.
        assert_eq!(
            broker.store.list_refs().count(),
            0,
            "no content-store write should occur on capability failure"
        );
    }

    // -- CAP-6 — capability not in warrant.tool_grants -------------------

    #[test]
    fn cap_6_capability_exceeds_warrant_on_tool_grants() {
        let name = tool_name("ungranted");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        // Warrant grants a DIFFERENT tool.
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: tool_name("other"),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "ungranted",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));

        let req = empty_request_with_pattern(EffectPattern::StageThenCommit, vec![]);
        let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
        assert!(matches!(
            err,
            BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::ToolGrants
            }
        ));
    }

    // -- CAP-7a — request.net_scope ⊄ warrant.net_scope ------------------

    #[test]
    fn cap_7a_capability_exceeds_warrant_on_net_scope() {
        let name = tool_name("rev");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "rev",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));

        // Request needs egress to a host the warrant doesn't allow.
        let req = EffectRequest {
            payload: vec![],
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host(
                "evil.example.com:443".into(),
            )])),
            fs_scope: FsScope::empty(),
        };
        let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
        assert!(matches!(
            err,
            BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::NetScope
            }
        ));
    }

    // -- CAP-7b — request.fs_scope ⊄ warrant.fs_scope --------------------

    #[test]
    fn cap_7b_capability_exceeds_warrant_on_fs_scope() {
        let name = tool_name("rev");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "rev",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));

        // Request needs write to a path not in warrant's fs_scope.
        let req = EffectRequest {
            payload: vec![],
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope {
                mounts: BTreeMap::from([(PathBuf::from("/etc"), FsMode::ReadWrite)]),
            },
        };
        let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
        assert!(matches!(
            err,
            BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::FsScope
            }
        ));
    }

    // -- CAP-8 — idempotency_token_for returns mote.id bytes -------------

    #[test]
    fn cap_8_idempotency_token_for_returns_mote_id_bytes() {
        let name = tool_name("rev");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let token = idempotency_token_for(&mote);
        assert_eq!(token.len(), 32);
        assert_eq!(&token, mote.id.as_bytes());
    }

    // -- CAP-9 — probe_readback returns Some(handle) when capability has it

    #[test]
    fn cap_9_probe_readback_returns_some_when_capability_primes_a_readback() {
        let name = tool_name("primed");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        let prerecorded = b"already-applied-state".to_vec();
        broker.register_capability(Box::new(PrimedReadbackCapability {
            name: name.clone(),
            version,
            patterns: vec![EffectPattern::IdempotentByConstruction],
            prerecorded: prerecorded.clone(),
        }));

        let probe = empty_request_with_pattern(EffectPattern::IdempotentByConstruction, vec![]);
        let outcome = broker
            .probe_readback(&mote, &warrant, &name, probe)
            .expect("probe ok");
        let handle = outcome.expect("expected Some(handle) — capability primed the probe");
        assert_eq!(handle.staged_ref, ContentRef::of(&prerecorded));
    }

    // -- CAP-9b — default probe (no override) returns None ---------------

    #[test]
    fn cap_9b_default_probe_returns_none() {
        let name = tool_name("rev");
        let version = tool_version("1");
        let mote = mote_with_tool(&name, &version);
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(ReverseCapability::new(
            "rev",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));
        let probe = empty_request_with_pattern(EffectPattern::StageThenCommit, vec![1, 2, 3]);
        let outcome = broker
            .probe_readback(&mote, &warrant, &name, probe)
            .expect("probe ok");
        assert!(
            outcome.is_none(),
            "default probe impl returns None — broker yields None"
        );
    }

    // -- Pattern: registered_count reflects registrations ---------------

    #[test]
    fn registered_count_reflects_register_calls() {
        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        assert_eq!(broker.registered_count(), 0);
        broker.register_capability(Box::new(ReverseCapability::new(
            "a",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));
        assert_eq!(broker.registered_count(), 1);
        broker.register_capability(Box::new(ReverseCapability::new(
            "b",
            "1",
            vec![EffectPattern::StageThenCommit],
        )));
        assert_eq!(broker.registered_count(), 2);
        // Re-register same name → replaces, count unchanged.
        broker.register_capability(Box::new(ReverseCapability::new(
            "a",
            "2",
            vec![EffectPattern::StageThenCommit],
        )));
        assert_eq!(broker.registered_count(), 2);
    }
}
