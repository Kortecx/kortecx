//! [`Capability`] trait — a named, versioned external system a Mote may
//! invoke. The seam between the broker and the actual remote/local
//! integration.

use kx_mote::{EffectPattern, ToolName, ToolVersion};

use crate::errors::CapabilityFailureReason;
use crate::request::EffectRequest;

/// A `Capability` is a named, versioned external system a Mote may invoke.
///
/// The set of capabilities a Mote may invoke is its
/// `MoteDef.tool_contract` (per `idempotency.md`); the set the runtime
/// will ever dispatch under any warrant is `warrant.tool_grants` (per
/// `warrant.md`). Both checks live in [`crate::CapabilityBroker::dispatch`].
///
/// Capabilities are registered with a broker via
/// [`crate::LocalCapabilityBroker::register_capability`]. The trait is
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

    /// Compensate (undo) an effect that may have partially applied (D65 —
    /// the recovery seam, M2.3). Default returns `Ok(None)` (capability does
    /// not support compensation; recovery quarantines rather than risking a
    /// double-fire).
    ///
    /// A capability whose effect cannot be made idempotent (no token, no
    /// readback) and whose double-application is harmful overrides this to
    /// reverse the effect deterministically, keyed on `MoteId`. `Ok(Some(bytes))`
    /// proves the compensating action ran (the bytes are the undo's
    /// externally-observable result); `Ok(None)` means "compensation
    /// unsupported"; `Err` means the compensation itself failed.
    ///
    /// **Like [`probe`][Capability::probe] this is a deterministic action**,
    /// **never a model call.** Reserved for the M2.3b class-aware
    /// `Compensate`-vs-`Redispatch` recovery decision; the M2.3a probe path
    /// does not invoke it.
    fn compensate(
        &self,
        request: &EffectRequest,
    ) -> Result<Option<Vec<u8>>, CapabilityFailureReason> {
        let _ = request;
        Ok(None)
    }
}
