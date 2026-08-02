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
    /// A credential this dispatch NAMES did not resolve on this host — the local secret
    /// store has no such name and neither does the environment. Carries the name so the
    /// operator is pointed at their own store rather than at the remote system's
    /// settings. Distinct from [`Self::AuthDenied`], which means the far end evaluated a
    /// credential and rejected it; here nothing was ever sent.
    CredentialUnresolved(String),
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

impl BrokerError {
    /// The part of this refusal that may be shown to the MODEL — the DOWNSTREAM
    /// system's own diagnostic, and nothing the runtime knows about itself.
    ///
    /// **Why this is a narrow allowlist and not `format!("{self}")`.** The rendered
    /// detail travels onto a durable `Failed` entry and from there into the next ReAct
    /// turn's instruction, which is the widest audience any of these strings has. Most
    /// `BrokerError` variants describe the RUNTIME's own state and carry host-shaped
    /// text — [`Self::StageWriteFailed`]'s store diagnostic can name a filesystem path,
    /// [`Self::SandboxRefused`]'s reason describes local confinement. None of that
    /// belongs in a model prompt, and a blanket stringify would put it there the moment
    /// a new variant appeared.
    ///
    /// Only [`Self::CapabilityFailure`] qualifies, and only through
    /// [`CapabilityFailureReason`] — a CLOSED enum whose single free-text arm the MCP
    /// layer populates with the server's own `"MCP error {code}: {message}"`. That
    /// boundary was already drawn deliberately: `TransportError::Unreachable(_)` DROPS
    /// its diagnostic to [`CapabilityFailureReason::NetworkUnreachable`] rather than
    /// echo an endpoint. This method respects that line instead of routing around it.
    ///
    /// Every other variant returns `""`, which renders as the unchanged class-derived
    /// steer — the pre-existing behaviour, preserved by default rather than by
    /// omission. There is no wildcard arm: a new `BrokerError` variant fails
    /// `cargo check` here and must be classified deliberately.
    #[must_use]
    pub fn model_facing_detail(&self) -> String {
        match self {
            Self::CapabilityFailure { reason, .. } => match reason {
                // The downstream system's own words — the whole point of the method.
                CapabilityFailureReason::Other(detail) => detail.clone(),
                // The closed arms are already a vocabulary the model can act on, and
                // they carry no free text, so rendering them is safe AND useful: "the
                // endpoint refused your credentials" is actionable where "it failed to
                // run" is not.
                CapabilityFailureReason::AuthDenied => {
                    "the external system denied authentication".to_string()
                }
                CapabilityFailureReason::RateLimited => {
                    "the external system rate-limited this call".to_string()
                }
                CapabilityFailureReason::NetworkUnreachable => {
                    "the external system was unreachable".to_string()
                }
                CapabilityFailureReason::Timeout => "the call exceeded its time budget".to_string(),
                CapabilityFailureReason::InvalidResponse => {
                    "the external system's response did not match the expected shape".to_string()
                }
                CapabilityFailureReason::CredentialUnresolved(name) => format!(
                    "the credential {name:?} this call needs is not stored on this host, so \
                     no request was sent; an operator must store it"
                ),
            },
            // Runtime-side refusals. The model is told THAT the call did not complete
            // (via the class-derived steer) and not how this machine is configured.
            Self::UnknownCapability { .. }
            | Self::UnsupportedPattern { .. }
            | Self::CapabilityExceedsWarrant { .. }
            | Self::SandboxRefused { .. }
            | Self::StageWriteFailed { .. } => String::new(),
        }
    }

    /// Is this refusal PERMANENT — i.e. would an identical re-dispatch fail identically?
    ///
    /// **Why this exists.** A dispatch failure is classified `TransientInfra` and retried
    /// on a bounded budget. That is right for a blip and wrong for a verdict: a warrant
    /// axis refusal, an undeclared capability, or a denied credential is a DECISION, and
    /// retrying a decision three times burns latency, triples the load on whatever
    /// refused, and buries the cause under two identical failures. This was measured
    /// twice in successive subsystems — a permanent warrant refusal retried 3× as
    /// transient, then a permanent capability refusal retried 3× as transient — which is
    /// the argument for classifying the CLASS here rather than the instance downstream.
    ///
    /// **Why it lives on the error and is read outside the kernel.** Same split as
    /// [`Self::model_facing_detail`]: the answer is only knowable while the error is
    /// still TYPED. `crates/kx-executor/src` renders it to `format!("{e:?}")`, and a
    /// string cannot be re-classified without re-parsing a debug dump — which would
    /// silently start guessing the moment a variant's `Debug` changed.
    ///
    /// **The honest boundary, stated rather than approximated.**
    /// [`CapabilityFailureReason::Other`] is opaque free text, so a permanent downstream
    /// refusal that arrives through it (an HTTP 501 from a backend that cannot serve the
    /// endpoint at all) is NOT recognised here and stays retryable. Sniffing a status
    /// code out of a diagnostic string would be a guess dressed as a classification, and
    /// the same anti-pattern — a fallback that looks graceful and removes the diagnosis —
    /// is what produced that 501 in the first place. The cure for that case is to refuse
    /// the impossible configuration at RESOLVE time, which is fixed separately; this
    /// method deliberately classifies only what the TYPE can answer.
    ///
    /// Exhaustive with no wildcard: a new variant fails `cargo check` here and must be
    /// classified deliberately, rather than defaulting into a retry loop.
    #[must_use]
    // `Other(_)` deliberately keeps its own arm despite sharing a body with the closed
    // transient set. The two say different things: those variants are KNOWN retryable,
    // while `Other` is UNKNOWN and treated as retryable because the arm carries free text
    // this method refuses to parse. Merging them would erase the boundary at the exact
    // site a reader needs it — and the boundary is load-bearing, because the failure
    // measured on the live serve arrives through `Other` and is therefore NOT covered.
    #[allow(clippy::match_same_arms)]
    pub fn is_permanent(&self) -> bool {
        match self {
            // Verdicts. The declaration, the warrant, the pattern and the confinement are
            // all fixed for the life of the dispatch — a retry re-asks a settled question.
            Self::UnknownCapability { .. }
            | Self::UnsupportedPattern { .. }
            | Self::CapabilityExceedsWarrant { .. }
            | Self::SandboxRefused { .. } => true,
            // The content store rejected the staging write. Disk pressure and transient
            // IO are exactly what a bounded retry is for.
            Self::StageWriteFailed { .. } => false,
            Self::CapabilityFailure { reason, .. } => match reason {
                // The external system evaluated the credential and said no. The same
                // credential produces the same answer; only an operator changes it.
                CapabilityFailureReason::AuthDenied => true,
                // A credential that is absent from this host is absent on the next
                // attempt too. Only an operator storing it changes the answer, so a retry
                // re-asks a settled question — and each one is another outbound call.
                CapabilityFailureReason::CredentialUnresolved(_) => true,
                // Genuinely retryable: a limit that lifts, a network that returns, a call
                // that may fit next time. `InvalidResponse` is included deliberately — a
                // server that garbles one response may serve the next, and treating a
                // single bad frame as permanent would strand a working connector.
                CapabilityFailureReason::RateLimited
                | CapabilityFailureReason::NetworkUnreachable
                | CapabilityFailureReason::Timeout
                | CapabilityFailureReason::InvalidResponse => false,
                // See the boundary above: opaque, so retryable by default.
                CapabilityFailureReason::Other(_) => false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap() -> ToolName {
        ToolName("fleet/get".to_string())
    }

    /// ★ The reason this method exists. An MCP server that says WHICH argument is
    /// wrong must have those words survive to the model, because the class-derived
    /// alternative ("it failed to run — do not call it again with the same
    /// arguments") is not vague but backwards: changing one argument IS the fix.
    #[test]
    fn a_downstream_systems_own_error_is_model_facing() {
        let e = BrokerError::CapabilityFailure {
            capability: cap(),
            reason: CapabilityFailureReason::Other(
                r#"MCP error -32004: no such vessel "x""#.to_string(),
            ),
        };
        assert_eq!(
            e.model_facing_detail(),
            r#"MCP error -32004: no such vessel "x""#
        );
    }

    /// The REFUSING control, varying exactly one thing: who authored the text. A
    /// store diagnostic can name a filesystem path on this machine, and a prompt is
    /// the widest audience any of these strings has.
    #[test]
    fn a_runtime_side_diagnostic_is_never_model_facing() {
        let e = BrokerError::StageWriteFailed {
            capability: cap(),
            diagnostic: "/Users/someone/.kx/blobs: permission denied".to_string(),
        };
        assert_eq!(
            e.model_facing_detail(),
            "",
            "a host path must never reach a model prompt"
        );
        let sandbox = BrokerError::SandboxRefused {
            capability: cap(),
            reason: "bwrap: /usr/bin/dash exited 71".to_string(),
        };
        assert_eq!(sandbox.model_facing_detail(), "");
    }

    /// The closed [`CapabilityFailureReason`] arms carry no free text, so they are
    /// safe to render AND more actionable than the generic steer — but they must
    /// still say something, or the arm is indistinguishable from a refusal.
    #[test]
    fn the_closed_reason_arms_are_rendered_and_non_empty() {
        for reason in [
            CapabilityFailureReason::AuthDenied,
            CapabilityFailureReason::RateLimited,
            CapabilityFailureReason::NetworkUnreachable,
            CapabilityFailureReason::Timeout,
            CapabilityFailureReason::InvalidResponse,
        ] {
            let rendered = BrokerError::CapabilityFailure {
                capability: cap(),
                reason: reason.clone(),
            }
            .model_facing_detail();
            assert!(
                !rendered.is_empty(),
                "{reason:?} must render something the model can act on"
            );
            assert!(
                !rendered.contains('/') || reason == CapabilityFailureReason::InvalidResponse,
                "{reason:?} rendered {rendered:?} — no path-shaped text in a closed arm"
            );
        }
    }

    /// The detail is bounded at the WRITE site, but a hostile server controls its
    /// length, so pin that this method does not itself promise a bound — the caller
    /// must apply `kx_journal::bounded_failure_detail`. Stated as a test so the
    /// obligation is discoverable from here rather than only from the call site.
    #[test]
    fn an_unbounded_server_message_is_returned_verbatim_for_the_caller_to_bound() {
        let huge = "x".repeat(10_000);
        let e = BrokerError::CapabilityFailure {
            capability: cap(),
            reason: CapabilityFailureReason::Other(huge.clone()),
        };
        assert_eq!(e.model_facing_detail().len(), huge.len());
    }

    /// ★ The measured incident this method exists for: a warrant-axis refusal was
    /// classified transient and re-dispatched THREE TIMES. A warrant is fixed for the
    /// life of the dispatch, so every retry re-asked a settled question and the two extra
    /// failures buried the cause. The accepting control is one variable away — a rate
    /// limit, where a bounded retry is exactly right — so this cannot pass by returning
    /// `true` for everything.
    #[test]
    fn a_warrant_refusal_is_permanent_and_a_rate_limit_is_not() {
        let refused = BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::ToolGrants,
        };
        assert!(
            refused.is_permanent(),
            "a warrant-axis refusal is a VERDICT — retrying it re-asks a settled question"
        );
        let limited = BrokerError::CapabilityFailure {
            capability: cap(),
            reason: CapabilityFailureReason::RateLimited,
        };
        assert!(
            !limited.is_permanent(),
            "a rate limit lifts — a bounded retry is the correct response"
        );
    }

    /// The rest of the vocabulary, pinned so a future variant cannot drift its
    /// classification silently. Denied credentials are a verdict; a network that was
    /// unreachable, a call that timed out, a response that did not parse and a staging
    /// write that failed are all things a retry may legitimately survive.
    #[test]
    fn the_permanence_of_every_variant_is_pinned() {
        let permanent: Vec<BrokerError> = vec![
            BrokerError::UnknownCapability { name: cap() },
            BrokerError::UnsupportedPattern {
                capability: cap(),
                requested: EffectPattern::IdempotentByConstruction,
            },
            BrokerError::CapabilityExceedsWarrant {
                axis: WarrantField::NetScope,
            },
            BrokerError::SandboxRefused {
                capability: cap(),
                reason: "confined".to_string(),
            },
            BrokerError::CapabilityFailure {
                capability: cap(),
                reason: CapabilityFailureReason::AuthDenied,
            },
            // A credential absent from this host is absent on the retry too, and each
            // retry is another outbound call carrying no credential.
            BrokerError::CapabilityFailure {
                capability: cap(),
                reason: CapabilityFailureReason::CredentialUnresolved("TOKEN".to_string()),
            },
        ];
        for e in &permanent {
            assert!(e.is_permanent(), "{e:?} must be permanent");
        }
        let transient: Vec<BrokerError> = vec![
            BrokerError::StageWriteFailed {
                capability: cap(),
                diagnostic: "disk".to_string(),
            },
            BrokerError::CapabilityFailure {
                capability: cap(),
                reason: CapabilityFailureReason::RateLimited,
            },
            BrokerError::CapabilityFailure {
                capability: cap(),
                reason: CapabilityFailureReason::NetworkUnreachable,
            },
            BrokerError::CapabilityFailure {
                capability: cap(),
                reason: CapabilityFailureReason::Timeout,
            },
            BrokerError::CapabilityFailure {
                capability: cap(),
                reason: CapabilityFailureReason::InvalidResponse,
            },
        ];
        for e in &transient {
            assert!(!e.is_permanent(), "{e:?} must be retryable");
        }
    }

    /// THE STATED GAP, pinned as a test so it is discoverable here rather than only in
    /// prose. A permanent downstream refusal that arrives through the opaque `Other` arm
    /// — an HTTP 501 from a backend that cannot serve the endpoint AT ALL — is NOT
    /// recognised as permanent, because the arm carries free text and reading a status
    /// code out of it would be a guess dressed as a classification. The cure is to refuse
    /// the impossible configuration at resolve time, not to sniff the string here. If a
    /// future change makes this decidable, this test is the one that must change.
    #[test]
    fn a_permanent_failure_arriving_as_opaque_text_is_not_recognised_and_that_is_stated() {
        let five_oh_one = BrokerError::CapabilityFailure {
            capability: cap(),
            reason: CapabilityFailureReason::Other(
                "retrieve: embedding: backend `kx-ollama` failed: ollama http status 501"
                    .to_string(),
            ),
        };
        assert!(
            !five_oh_one.is_permanent(),
            "the Other arm is opaque by construction; this classifies TYPES, never strings"
        );
    }
}
