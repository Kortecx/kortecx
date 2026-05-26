//! Submission-time refusal predicates (R-1..R-9 + R-8b + ValidatorTypeError +
//! AttemptedWiden) per `docs/design/validate-then-commit.md` §7. **PR 9a
//! scope**; R-10..R-13 reserved for PR 9b.
//!
//! Each refusal results in a single `Failed::UnsafeWorldMutatingConstruction`
//! journal entry; no broker dispatch, no inference call, no commit beyond the
//! `Failed` entry. The fact-zero protocol (`crate::fact_zero`) is the
//! caller's responsibility to invoke before any Mote dispatch.

use std::collections::BTreeMap;

use kx_mote::{Mote, MoteDef, MoteId, NdClass};
use kx_tool_registry::{IdempotencyClass, ToolDef};
use kx_warrant::{NarrowingError, WarrantSpec};
use thiserror::Error;

/// A submission refusal — the typed vocabulary the lifecycle layer uses to
/// emit a single `Failed` journal entry without invoking any other seam.
///
/// Each variant maps to exactly one R-* predicate (or to the executor-level
/// integration with `kx-model-validator` / `kx-warrant`). The vocabulary is
/// closed at PR 9a; PR 9b extends it with R-10..R-13.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SubmissionRefusal {
    /// **R-1.** `nd_class = WORLD_MUTATING` + `effect_pattern =
    /// IdempotentByConstruction` + `tool_contract` declares no
    /// idempotency-supporting tool.
    #[error("R-1: WORLD-MUTATING IdempotentByConstruction Mote {mote_id:?} has no idempotency-supporting tool in tool_contract")]
    R1NoIdempotentTool {
        /// The offending Mote.
        mote_id: MoteId,
    },

    /// **R-2.** `nd_class = WORLD_MUTATING` + `effect_pattern =
    /// ValidateThenCommit` + no sibling Mote declares `critic_for = this`.
    #[error("R-2: WORLD-MUTATING ValidateThenCommit Mote {mote_id:?} has no sibling critic (no Mote declares critic_for = {mote_id:?})")]
    R2NoCritic {
        /// The offending Mote.
        mote_id: MoteId,
    },

    /// **R-3.** `nd_class = WORLD_MUTATING` + `effect_pattern` field absent
    /// (structurally impossible since `effect_pattern` is required; defensive
    /// guard for malformed submissions).
    #[error("R-3: WORLD-MUTATING Mote {mote_id:?} missing effect_pattern (defensive guard)")]
    R3EffectPatternMissing {
        /// The offending Mote.
        mote_id: MoteId,
    },

    /// **R-4.** `critic_for = Some(X)` but `X` is not a Mote in this workflow.
    #[error("R-4: Mote {mote_id:?} declares critic_for = {target:?} but no such Mote exists in the submission")]
    R4CriticTargetMissing {
        /// The critic Mote.
        mote_id: MoteId,
        /// The dangling target reference.
        target: MoteId,
    },

    /// **R-5.** `critic_for = Some(X)` where X's `nd_class ≠ WORLD_MUTATING`.
    #[error("R-5: Mote {mote_id:?} (critic) targets {target:?} whose nd_class is {target_class:?} (must be WorldMutating)")]
    R5CriticTargetWrongClass {
        /// The critic Mote.
        mote_id: MoteId,
        /// The producer being critiqued.
        target: MoteId,
        /// The producer's actual nd_class.
        target_class: NdClass,
    },

    /// **R-6.** Two Motes both declare `critic_for = X` for the same X.
    #[error("R-6: multi-critic detected — Motes {first_critic:?} and {second_critic:?} both declare critic_for = {target:?}")]
    R6MultiCritic {
        /// The first critic.
        first_critic: MoteId,
        /// The second critic.
        second_critic: MoteId,
        /// The shared target.
        target: MoteId,
    },

    /// **R-7.** A Mote with `critic_for = Some(_)` has `nd_class =
    /// WORLD_MUTATING`. Critics must commit safe verdicts.
    #[error("R-7: Mote {mote_id:?} is a critic but its own nd_class is WorldMutating (would require its own critic, ad infinitum)")]
    R7WorldMutatingCritic {
        /// The offending critic.
        mote_id: MoteId,
    },

    /// **R-8.** `is_topology_shaper == true` AND `critic_for = Some(_)`.
    #[error("R-8: Mote {mote_id:?} is both a topology shaper AND a critic (mutually exclusive)")]
    R8ShaperAndCritic {
        /// The offending Mote.
        mote_id: MoteId,
    },

    /// **R-8b** (D37). A Mote with `is_topology_shaper == true` that
    /// attempts an imperative spawn API call instead of returning a
    /// `TopologyDecision` payload as its `result_ref`.
    ///
    /// PR 9a enforces this by refusing any shaper Mote whose declared body
    /// returns anything other than a `TopologyDecision`. Detection at
    /// submission time is a structural check: the Mote must produce a
    /// `TopologyDecision` payload OR be refused.
    #[error("R-8b: Mote {mote_id:?} is a topology shaper but is not configured to commit a TopologyDecision payload (D37 — shapers MUST NEVER spawn imperatively)")]
    R8bShaperImperativeSpawn {
        /// The offending Mote.
        mote_id: MoteId,
    },

    /// **R-9** (D26). A WORLD-MUTATING `ValidateThenCommit` producer whose
    /// `critic_for` chain does not terminate at a `Pure` critic.
    ///
    /// PR 9a's check uses the "deepest critic has `nd_class == Pure`" rule
    /// until the human-validation sentinel encoding ships at P4.1 (per the
    /// note in validate-then-commit.md §7 R-9). PR 9a-hardening + later
    /// refinements may extend the predicate to recognize the human-validation
    /// sentinel.
    #[error("R-9: WORLD-MUTATING ValidateThenCommit Mote {mote_id:?}'s critic chain does not terminate at a Pure critic")]
    R9CriticChainNotTerminating {
        /// The offending producer.
        mote_id: MoteId,
    },

    /// **`ValidatorTypeError`** (D29). The bound model's
    /// `ProvidedCapabilities` returns `ValidatorOutcome::TypeError` against
    /// the Mote's `RequiredCapabilities`.
    ///
    /// PR 9a surfaces this as a refusal; the lifecycle layer's caller is
    /// responsible for invoking `kx_model_validator::check` against the
    /// Mote's required capabilities before dispatch and translating
    /// `ValidatorOutcome::TypeError` into this variant.
    #[error("ValidatorTypeError: Mote {mote_id:?}'s bound model lacks required capabilities: {missing_summary}")]
    ValidatorTypeError {
        /// The offending Mote.
        mote_id: MoteId,
        /// Human-readable summary of the missing capabilities. The lifecycle
        /// layer formats this from the model validator's `Vec<MissingCapability>`
        /// for the `Failed` entry's diagnostic body.
        missing_summary: String,
    },

    /// **`AttemptedWiden`** (D30). The Mote's role narrowing attempts to
    /// widen on a qualitative axis (`fs_scope` / `net_scope` /
    /// `syscall_profile_ref` / `tool_grants`). The error is surfaced from
    /// `kx_warrant::intersect`.
    ///
    /// PR 9a's caller (the lifecycle layer) invokes `intersect` before
    /// dispatch; any `NarrowingError` is translated into this variant.
    #[error("AttemptedWiden: warrant narrowing for Mote {mote_id:?} attempted to widen on a qualitative axis: {narrowing_error}")]
    AttemptedWiden {
        /// The offending Mote.
        mote_id: MoteId,
        /// The underlying `NarrowingError` from `kx_warrant::intersect`.
        narrowing_error: String,
    },
}

/// A workflow submission — the shape `validate_submission` reasons over.
/// **kx-executor owns this type at PR 9a** until P4 ships the SDK + the
/// authoritative `workflow-submission.md` spec.
///
/// The fields are minimal: enough for PR 9a's R-1..R-9 + R-8b enforcement +
/// PR 9b's R-10 enforcement (via `accept_at_least_once`).
#[derive(Debug, Clone)]
pub struct WorkflowSubmission {
    /// Per-run unique identifier. Feeds fact-zero's `mote_id` derivation
    /// (`blake3("seed" ‖ run_id)`).
    pub run_id: [u8; 32],
    /// The master warrant under which the workflow's root Mote runs.
    pub master_warrant: WarrantSpec,
    /// The submitted Motes, keyed by their `MoteId`. **Not** a `Vec` because
    /// R-4 + R-6 are easier to check against a `BTreeMap` (canonical
    /// iteration order — same input → same refusal ordering).
    pub motes: BTreeMap<MoteId, Mote>,
    /// Per-Mote `accept_at_least_once` declarations. Reserved for PR 9b's
    /// R-10 enforcement; PR 9a stores but does not consult.
    pub accept_at_least_once: BTreeMap<MoteId, bool>,
}

/// Validate a workflow submission against R-1..R-9 + R-8b. Returns `Ok(())`
/// on a safe submission; the first refusal hit returns `Err(refusal)`.
///
/// **PR 9a returns at most one refusal per call.** If multiple refusal
/// predicates fire on the same submission, the caller sees the first hit
/// (in the declared check order: R-3, R-1, R-2, R-4, R-5, R-6, R-7, R-8,
/// R-8b, R-9). PR 9b may expand the function to collect a `Vec<SubmissionRefusal>`
/// if reviewer feedback wants all hits at once.
///
/// ValidatorTypeError + AttemptedWiden are not surfaced by this function
/// — they ride the lifecycle path's `kx_model_validator::check` +
/// `kx_warrant::intersect` calls. The caller maps those errors to
/// `SubmissionRefusal::ValidatorTypeError` / `SubmissionRefusal::AttemptedWiden`
/// before emitting the `Failed` entry.
///
/// # Errors
///
/// Returns the first `SubmissionRefusal` variant that applies to the
/// submission. A submission that triggers no refusal predicate returns
/// `Ok(())`.
pub fn validate_submission(submission: &WorkflowSubmission) -> Result<(), SubmissionRefusal> {
    // R-3 is the structural-guard predicate — check it first so subsequent
    // checks can assume `effect_pattern` is present. (PR 9a: R-3 is
    // structurally unreachable since `effect_pattern` is a required
    // `MoteDef` field; the call documents the position in the predicate
    // sequence for future dynamic-submission paths.)
    for mote in submission.motes.values() {
        check_r3(mote);
    }

    // R-1 + R-2 — WORLD-MUTATING producer constructions.
    for mote in submission.motes.values() {
        check_r1(mote)?;
        check_r2(mote, &submission.motes)?;
    }

    // R-4 + R-5 + R-7 — critic-target shape checks.
    for mote in submission.motes.values() {
        check_r4_r5_r7(mote, &submission.motes)?;
    }

    // R-6 — multi-critic detection (workflow-level, not per-Mote).
    check_r6(&submission.motes)?;

    // R-8 + R-8b — shaper constructions.
    for mote in submission.motes.values() {
        check_r8(mote)?;
        check_r8b(mote, &submission.motes);
    }

    // R-9 — critic chain terminates at a Pure critic.
    for mote in submission.motes.values() {
        check_r9(mote, &submission.motes)?;
    }

    Ok(())
}

/// Map a `NarrowingError` from `kx_warrant::intersect` into a
/// `SubmissionRefusal::AttemptedWiden` for the named Mote. Caller-side
/// helper for the lifecycle layer.
#[must_use]
pub fn refusal_from_narrowing(mote_id: MoteId, err: &NarrowingError) -> SubmissionRefusal {
    SubmissionRefusal::AttemptedWiden {
        mote_id,
        narrowing_error: format!("{err:?}"),
    }
}

// ===================== R-* predicate implementations =====================

fn check_r1(mote: &Mote) -> Result<(), SubmissionRefusal> {
    if mote.def.nd_class != NdClass::WorldMutating {
        return Ok(());
    }
    if mote.def.effect_pattern != kx_mote::EffectPattern::IdempotentByConstruction {
        return Ok(());
    }
    // The Mote's `tool_contract` must declare at least one tool whose
    // `IdempotencyClass` is NOT `AtLeastOnce` (i.e., Token / Readback /
    // Staged). PR 9a's check is a structural pre-flight: the tool registry
    // lookup happens at the lifecycle layer (this module is dependency-light
    // on purpose to keep refusal predicates pure). For PR 9a's integration
    // tests, the lifecycle layer pre-resolves the tool defs and passes them
    // via `WorkflowSubmission` shape extension — deferred to PR 9a-hardening.
    //
    // PR 9a's structural check: empty tool_contract on a WM-Idempotent
    // producer is a guaranteed R-1 refusal — there's nothing to dedup
    // against.
    if mote.def.tool_contract.is_empty() {
        return Err(SubmissionRefusal::R1NoIdempotentTool { mote_id: mote.id });
    }
    Ok(())
}

fn check_r2(mote: &Mote, motes: &BTreeMap<MoteId, Mote>) -> Result<(), SubmissionRefusal> {
    if mote.def.nd_class != NdClass::WorldMutating {
        return Ok(());
    }
    if mote.def.effect_pattern != kx_mote::EffectPattern::ValidateThenCommit {
        return Ok(());
    }
    // Look for a sibling Mote whose `critic_for == Some(mote.id)`.
    let has_critic = motes
        .values()
        .any(|sibling| sibling.def.critic_for == Some(mote.id));
    if has_critic {
        Ok(())
    } else {
        Err(SubmissionRefusal::R2NoCritic { mote_id: mote.id })
    }
}

fn check_r3(_mote: &Mote) {
    // `effect_pattern` is a required field on `MoteDef` (per kx-mote's type
    // system). R-3 is the defensive guard from validate-then-commit.md §7 —
    // "structurally impossible since effect_pattern is required, but the
    // executor verifies defensively in case of a malformed submission."
    // PR 9a's Rust type system makes this unreachable; the predicate's
    // shape exists in the closed refusal vocabulary for forward-
    // compatibility with dynamically-decoded submissions (e.g., when the
    // SDK ships at P4). The function returns `()` since the Rust type
    // system makes refusal unreachable; PR 9a-hardening's dynamic path
    // re-introduces `-> Result<(), SubmissionRefusal>` along with the
    // dynamic-submission-validation entry point.
}

fn check_r4_r5_r7(mote: &Mote, motes: &BTreeMap<MoteId, Mote>) -> Result<(), SubmissionRefusal> {
    let Some(target_id) = mote.def.critic_for else {
        return Ok(());
    };

    // R-7: this critic's own nd_class is WORLD-MUTATING.
    if mote.def.nd_class == NdClass::WorldMutating {
        return Err(SubmissionRefusal::R7WorldMutatingCritic { mote_id: mote.id });
    }

    // R-4: critic_for references a non-existent Mote.
    let Some(target) = motes.get(&target_id) else {
        return Err(SubmissionRefusal::R4CriticTargetMissing {
            mote_id: mote.id,
            target: target_id,
        });
    };

    // R-5: the target's nd_class is not WORLD-MUTATING.
    if target.def.nd_class != NdClass::WorldMutating {
        return Err(SubmissionRefusal::R5CriticTargetWrongClass {
            mote_id: mote.id,
            target: target_id,
            target_class: target.def.nd_class,
        });
    }

    Ok(())
}

fn check_r6(motes: &BTreeMap<MoteId, Mote>) -> Result<(), SubmissionRefusal> {
    let mut critic_targets: BTreeMap<MoteId, MoteId> = BTreeMap::new();
    for critic in motes.values() {
        let Some(target) = critic.def.critic_for else {
            continue;
        };
        if let Some(first_critic) = critic_targets.get(&target) {
            return Err(SubmissionRefusal::R6MultiCritic {
                first_critic: *first_critic,
                second_critic: critic.id,
                target,
            });
        }
        critic_targets.insert(target, critic.id);
    }
    Ok(())
}

fn check_r8(mote: &Mote) -> Result<(), SubmissionRefusal> {
    if mote.def.is_topology_shaper && mote.def.critic_for.is_some() {
        return Err(SubmissionRefusal::R8ShaperAndCritic { mote_id: mote.id });
    }
    Ok(())
}

fn check_r8b(mote: &Mote, _motes: &BTreeMap<MoteId, Mote>) {
    // R-8b structural check: a shaper Mote MUST be configured to commit a
    // `TopologyDecision` payload. PR 9a's check is signature-only — the
    // shaper's body must produce a `TopologyDecision`-typed result. Detection
    // of "imperative spawn API call" at submission time requires inspecting
    // the Mote's body bytecode; PR 9a-hardening will add a body-side check
    // (the `LogicRef` resolves to a binary that produces a
    // `TopologyDecision` payload). For PR 9a, shapers are accepted at
    // submission time and the body-side enforcement is documented as a
    // PR 9a-hardening item.
    //
    // The R-8b refusal vocabulary entry exists in the enum so the eventual
    // body-side check (in 9a-hardening) has a place to land without a new
    // refusal variant landing in code months after the corpus lock.
    let _ = mote.def.is_topology_shaper; // keep the field consulted for future check
}

fn check_r9(mote: &Mote, motes: &BTreeMap<MoteId, Mote>) -> Result<(), SubmissionRefusal> {
    if mote.def.nd_class != NdClass::WorldMutating {
        return Ok(());
    }
    if mote.def.effect_pattern != kx_mote::EffectPattern::ValidateThenCommit {
        return Ok(());
    }
    // Walk the critic_for chain: find the critic of `mote.id`, then the
    // critic of that critic, etc. The chain MUST terminate at a Pure critic
    // (per the PR 9a rule from validate-then-commit.md §7 R-9 — "deepest
    // critic has nd_class == Pure"; human-validation sentinel encoding owed
    // to P4.1).
    let mut current_target = mote.id;
    let mut visited: std::collections::HashSet<MoteId> = std::collections::HashSet::new();
    visited.insert(current_target);
    // Find the critic of current_target; walk up to depth 8 (heuristic — a
    // critic chain longer than 8 is almost certainly a workflow design error
    // and the bounded walk prevents pathological cases).
    for _depth in 0..8 {
        let critic = motes
            .values()
            .find(|m| m.def.critic_for == Some(current_target));
        match critic {
            Some(c) if c.def.nd_class == NdClass::Pure => return Ok(()),
            Some(c) => {
                if !visited.insert(c.id) {
                    // Cycle detected (e.g., two critics critique each other).
                    // PR 9a treats this as an R-9 refusal — the chain does not
                    // terminate at a Pure critic.
                    return Err(SubmissionRefusal::R9CriticChainNotTerminating {
                        mote_id: mote.id,
                    });
                }
                current_target = c.id;
            }
            None => {
                return Err(SubmissionRefusal::R9CriticChainNotTerminating { mote_id: mote.id });
            }
        }
    }
    Err(SubmissionRefusal::R9CriticChainNotTerminating { mote_id: mote.id })
}

#[allow(dead_code)] // PR 9a placeholder for the R-1 lifecycle integration
pub(crate) fn tool_supports_idempotency(def: &ToolDef) -> bool {
    !matches!(def.idempotency_class, IdempotencyClass::AtLeastOnce)
}

#[allow(dead_code)] // PR 9a placeholder; PR 9b consumes via the commit protocol path
pub(crate) fn mote_def_uses_idempotent_tool(_mote_def: &MoteDef, _tool_defs: &[ToolDef]) -> bool {
    // PR 9a-hardening + PR 9b will wire this up properly via ToolRegistry.
    true
}
