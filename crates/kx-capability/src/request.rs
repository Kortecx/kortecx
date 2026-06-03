//! [`EffectRequest`] — the opaque-payload bundle the broker routes to a
//! capability after the per-call contract checks pass. [`BrokerHandle`] —
//! the commit-ready artifact the broker returns on a successful dispatch.

use kx_content::ContentRef;
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_warrant::{FsScope, NetScope, SecretScope};
use serde::{Deserialize, Serialize};

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
    /// [`crate::idempotency_token_for`]. Required for token-class WM tools
    /// (executor predicate R-10); optional for other classes.
    pub idempotency_key: Option<[u8; 32]>,
    /// The network egress this dispatch requires. Must be a subset of
    /// `warrant.net_scope`; otherwise the broker refuses with
    /// [`crate::BrokerError::CapabilityExceedsWarrant`] on
    /// [`kx_warrant::WarrantField::NetScope`].
    pub net_scope: NetScope,
    /// The filesystem access this dispatch requires. Must be a subset of
    /// `warrant.fs_scope`; otherwise the broker refuses with
    /// [`crate::BrokerError::CapabilityExceedsWarrant`] on
    /// [`kx_warrant::WarrantField::FsScope`].
    pub fs_scope: FsScope,
    /// The secret references this dispatch requires resolution of (D110.3).
    /// Must be a subset of `warrant.secret_scope`; otherwise the broker refuses
    /// with [`crate::BrokerError::CapabilityExceedsWarrant`] on
    /// [`kx_warrant::WarrantField::SecretScope`]. Defaults to
    /// [`SecretScope::None`] (a dispatch that needs no secret).
    pub secret_scope: SecretScope,
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
