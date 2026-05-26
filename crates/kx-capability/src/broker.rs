//! [`CapabilityBroker`] trait — the D24 seam between the executor and the
//! effect surface. OSS ships [`crate::LocalCapabilityBroker`]; cloud ships a
//! hardened impl behind the same trait per D28.

use kx_mote::ToolName;
use kx_warrant::WarrantSpec;

use crate::errors::BrokerError;
use crate::request::{BrokerHandle, EffectRequest};

/// The executor's sole interface to effects.
///
/// One implementation per deployment shape:
/// - **P1.8.5 (this crate, OSS):** [`crate::LocalCapabilityBroker`] — trivial
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
    /// 2. Verifies the requested [`kx_mote::EffectPattern`] is in the
    ///    capability's `supported_patterns()`
    ///    (refuses [`BrokerError::UnsupportedPattern`] otherwise).
    /// 3. Verifies the capability is in `warrant.tool_grants`
    ///    (refuses
    ///    [`BrokerError::CapabilityExceedsWarrant`]`{`[`kx_warrant::WarrantField::ToolGrants`]`}`
    ///    otherwise).
    /// 4. Verifies `request.net_scope` ⊆ `warrant.net_scope` and
    ///    `request.fs_scope` ⊆ `warrant.fs_scope`
    ///    (refuses
    ///    [`BrokerError::CapabilityExceedsWarrant`]`{`[`kx_warrant::WarrantField::NetScope`]`}` /
    ///    `{`[`kx_warrant::WarrantField::FsScope`]`}` otherwise).
    /// 5. Routes the request to the named capability via
    ///    [`crate::Capability::invoke`].
    /// 6. Stages the response payload to the content store
    ///    (content-addressed; D17).
    /// 7. Returns a [`BrokerHandle`] the executor uses to assemble the
    ///    journal commit.
    ///
    /// The broker NEVER writes the journal (D14 — the executor owns the
    /// commit txn).
    fn dispatch(
        &self,
        mote: &kx_mote::Mote,
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
    /// [`probe`][crate::Capability::probe] method:
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
        mote: &kx_mote::Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError>;
}
