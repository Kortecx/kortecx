//! Broker refusal vocabulary: [`CapabilityFailureReason`] (returned by a
//! capability when its invocation fails) + [`BrokerError`] (the broker's
//! typed refusal at dispatch / probe).

use kx_mote::{EffectPattern, ToolName};
use kx_warrant::WarrantField;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
