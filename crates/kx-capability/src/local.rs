//! [`LocalCapabilityBroker`] — the OSS trivial pass-through impl of
//! [`crate::CapabilityBroker`]. In-process; no sandboxing; single-tenant.

use std::collections::BTreeMap;
use std::sync::RwLock;

use kx_content::ContentStore;
use kx_mote::{Mote, ToolName};
use kx_warrant::{ToolGrant, WarrantField, WarrantSpec};

use crate::broker::CapabilityBroker;
use crate::capability::Capability;
use crate::errors::BrokerError;
use crate::request::{BrokerHandle, EffectRequest};

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
/// via [`ContentStore::put`]; the resulting [`kx_content::ContentRef`] is
/// the `BrokerHandle.staged_ref`. Two dispatches returning byte-identical
/// payloads share the same `staged_ref` (content-addressing dedupes for
/// free — this is the D17 atomicity contract reused).
///
/// # Architectural review (SN-4 v2 #9) — recorded inline.
///
/// 1. The broker has no `kx-journal` or `kx-projection` dependency
///    (verify with `cargo tree`). The recovery-state-independence
///    invariant from capability-broker.md §3 is structurally enforced
///    by the Cargo manifest.
/// 2. The trait surface admits a future hosted impl: the trait is
///    object-safe and `Send + Sync`; no signature carries an
///    in-process-only type (e.g., no `Arc<Mutex<...>>` over an
///    in-process queue). A future hardened cloud-side broker can
///    implement `CapabilityBroker` with a remote dispatch protocol
///    behind the same trait, and the executor code is unchanged.
/// 3. Every `BrokerError` variant is test-reachable: `UnknownCapability`
///    (CAP-2 fixture), `UnsupportedPattern` (CAP-3 fixture),
///    `CapabilityExceedsWarrant` (CAP-6/7 fixtures on three axes),
///    `CapabilityFailure` (CAP-5 fixture), `SandboxRefused` (CAP-10
///    fixture — fixture broker variant returns it via a fake
///    `CapabilityBroker` impl in tests/), `StageWriteFailed` (CAP-11
///    fixture using a failing content-store impl).
/// 4. The single-writer registry (the `RwLock` around the BTreeMap)
///    holds the write lock only for the duration of a `BTreeMap::insert`;
///    invocations and probes hold the READ lock for as briefly as
///    possible (drop before any I/O — see `dispatch` and
///    `probe_readback`). This composes with the workspace's
///    concurrency-test discipline (SN-4 v2 #6) which the integration
///    tests exercise.
pub struct LocalCapabilityBroker<S: ContentStore + Send + Sync> {
    pub(crate) store: S,
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
        capability_version: &kx_mote::ToolVersion,
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

    #[tracing::instrument(
        level = "debug",
        skip(self, mote, warrant, request),
        fields(
            mote_id = %mote.id,
            capability = %capability.0,
            pattern = ?request.pattern,
        )
    )]
    fn compensate(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        request: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        // SAME per-call contract gate as dispatch/probe_readback — compensation
        // is a world-mutating undo and must not bypass the warrant (D65 / M2.3b).
        let guard = self.capabilities.read().expect("RwLock poisoned");
        let cap = Self::precheck(&guard, mote, warrant, capability, &request)?;
        let cap_name = cap.name().clone();
        let cap_version = cap.version().clone();
        let outcome = cap.compensate(&request);
        // Drop the read lock BEFORE staging — staging is I/O.
        drop(guard);
        let bytes = match outcome {
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
