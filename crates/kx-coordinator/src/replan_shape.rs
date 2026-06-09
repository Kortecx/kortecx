//! Coordinator-local re-plan-round SHAPING primitives (PR-2c-2, re-plan-live).
//!
//! These re-implement, BYTE-FOR-BYTE, the three identity-bearing pieces the
//! `kx-model-harness` driver uses to drive a re-plan round — the failure→token
//! mapping, the corrected planning prompt, and the round-namespaced shaper Mote.
//! They are re-implemented here (not shared) because the coordinator sits BELOW
//! the dep wall and must not depend on `kx-model-harness` (the live shaper loop
//! reuses public `kx-planner`/`kx-mote` primitives coordinator-locally, exactly as
//! PR-2b did). The equivalence is **load-bearing for R49**: the live coordinator
//! and the harness must derive the SAME shaper `MoteId` for a given
//! `(round, corrected_prompt, model_id)`, or a cold re-fold of a harness-written
//! journal on the live binary (or vice-versa) would diverge. A frozen golden
//! string + a frozen token mapping pin the equivalence in tests; a matching golden
//! is asserted harness-side, so a drift on either copy fails CI.
//!
//! Pure + total + dependency-light: identity material is `blake3` via
//! [`ContentRef::of`] (kx-content), so the coordinator takes no direct `blake3`
//! dependency (D111 — `Cargo.lock` unchanged), exactly like `materialize.rs`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kx_content::ContentRef;
use kx_journal::FailureReason;
use kx_mote::{
    ConfigKey, ConfigVal, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef,
    ModelId, Mote, MoteDef, MoteId, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY,
};
use smallvec::SmallVec;

/// The greedy-decode output-token cap a re-plan shaper runs under — mirrors
/// `kx_model_harness::workflows::greedy(128)` (a greedy/deterministic decode so a
/// live decode stays reproducible).
const SHAPER_MAX_OUTPUT_TOKENS: u32 = 128;

/// A STABLE, rename-and-reorder-proof token for a [`FailureReason`] — keyed off the
/// canonical `as_u8()` `#[repr(u8)]` discriminant (the journal's own on-disk reason
/// encoding), NEVER `Debug`. **Load-bearing for R49**: this token is threaded into
/// the re-plan shaper's [`corrected_prompt`], which is identity-bearing
/// (`config_subset` → `MoteDef::hash` → the shaper `MoteId`). A byte-for-byte copy
/// of `kx_model_harness::topology_provider::failure_reason_token` (pinned by
/// [`tests::failure_reason_token_is_frozen`] + a matching harness golden).
#[must_use]
pub(crate) fn failure_reason_token(reason: Option<FailureReason>) -> &'static str {
    match reason.map(FailureReason::as_u8) {
        Some(0) => "timed-out",
        Some(1) => "executor-refused",
        Some(2) => "validator-rejected",
        Some(3) => "worker-crashed",
        Some(4) => "upstream-repudiated",
        Some(5) => "unsafe-world-mutating-construction",
        Some(6) => "compensated-at-least-once",
        Some(7) => "quarantined-at-least-once",
        Some(8) => "dead-lettered",
        _ => "transient-or-unknown",
    }
}

/// Build a re-plan round's failure-corrected planning instruction — deterministic
/// (`failures` MUST be pre-sorted by `mote_id.as_bytes()`, each reason rendered via
/// the stable [`failure_reason_token`]), so a cold re-fold reconstructs the SAME
/// prompt ⇒ the SAME shaper `MoteId` (R49). The reasons are the low-entropy
/// [`FailureReason`] enum only (never result bytes / secrets — SN-8). A byte-for-byte
/// copy of `kx_model_harness::topology_provider::corrected_prompt`.
#[must_use]
pub(crate) fn corrected_prompt(base: &str, failures: &[(MoteId, Option<FailureReason>)]) -> String {
    let mut s = String::from(base);
    s.push_str(
        "\n\nThe previous attempt left failed step(s). Respond with a `replan` envelope: \
         either `next_steps` that retry or replace them (corrected context / a role whose \
         authority fits), or `flag_human` with a reason if you cannot fix it within your \
         authority. Failed step(s):",
    );
    for (id, reason) in failures {
        let label = failure_reason_token(*reason);
        let _ = write!(s, "\n- step {id} failed (reason: {label})");
    }
    s
}

/// The round-namespaced 32-byte identity material for a re-plan round's shaper:
/// `blake3(b"kx-replan-round" ‖ round.to_le_bytes())`. Deterministic + distinct per
/// round, and cryptographically distinct from any `loop_shaper` `[seed; 32]`.
/// Mirrors `kx_model_harness::workflows::replan_shaper` (which uses `blake3::hash`
/// directly; [`ContentRef::of`] IS blake3-of-bytes, so the bytes are identical
/// without a direct `blake3` dependency).
#[must_use]
pub(crate) fn replan_shaper_id_material(round: u32) -> [u8; 32] {
    let mut material = b"kx-replan-round".to_vec();
    material.extend_from_slice(&round.to_le_bytes());
    *ContentRef::of(&material).as_bytes()
}

/// Re-derive a re-plan round's shaper `Mote` from its `(round, model_id,
/// planning_prompt)` — byte-for-byte identical to
/// `kx_model_harness::workflows::replan_shaper`, so the derived `MoteId` matches the
/// harness oracle (R49) and the in-flight-round identity the recovery pass checks.
///
/// The shaper is a ROND topology shaper (R-14: never WORLD-MUTATING), greedy-decoded
/// (the committed decision is the served fact, R49), with the planning prompt carried
/// in `config_subset[PROMPT_KEY]` (identity-bearing) and EMPTY parents (edge-free —
/// the digest-invariance + planner-prompt-injection guard: an inherited Data edge
/// would both move the digest via `encode_state` and feed raw upstream bytes into the
/// planner via F-7 `parent_results`).
#[must_use]
pub(crate) fn build_replan_shaper(model_id: &ModelId, planning_prompt: &str, round: u32) -> Mote {
    let id_bytes = replan_shaper_id_material(round);

    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(planning_prompt.as_bytes().to_vec()),
    );
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: InferenceParams {
            max_output_tokens: SHAPER_MAX_OUTPUT_TOKENS,
            ..InferenceParams::default()
        },
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes(id_bytes),
        GraphPosition(id_bytes.to_vec()),
        SmallVec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The frozen golden: the EXACT corrected-prompt suffix the harness emits. A
    // matching literal is asserted in the harness so a drift on either copy fails
    // CI (R49 cross-impl equivalence — D2).
    const CORRECTED_SUFFIX: &str =
        "\n\nThe previous attempt left failed step(s). Respond with a `replan` envelope: \
         either `next_steps` that retry or replace them (corrected context / a role whose \
         authority fits), or `flag_human` with a reason if you cannot fix it within your \
         authority. Failed step(s):";

    #[test]
    fn failure_reason_token_is_frozen() {
        // Pin the canonical as_u8()-keyed mapping (any reorder/renumber that would
        // shift identity bytes fails here).
        assert_eq!(failure_reason_token(Some(FailureReason::TimedOut)), "timed-out");
        assert_eq!(
            failure_reason_token(Some(FailureReason::ExecutorRefused)),
            "executor-refused"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::ValidatorRejected)),
            "validator-rejected"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::WorkerCrashed)),
            "worker-crashed"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::UpstreamRepudiated)),
            "upstream-repudiated"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::UnsafeWorldMutatingConstruction)),
            "unsafe-world-mutating-construction"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::CompensatedAtLeastOnce)),
            "compensated-at-least-once"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::QuarantinedAtLeastOnce)),
            "quarantined-at-least-once"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::DeadLettered)),
            "dead-lettered"
        );
        assert_eq!(failure_reason_token(None), "transient-or-unknown");
    }

    #[test]
    fn corrected_prompt_is_deterministic_and_frozen() {
        let base = "Plan the run.";
        let id = MoteId::from_bytes([3u8; 32]);
        let out = corrected_prompt(&base, &[(id, Some(FailureReason::DeadLettered))]);
        // Stable across calls (no clock / no RNG).
        assert_eq!(out, corrected_prompt(&base, &[(id, Some(FailureReason::DeadLettered))]));
        assert!(out.starts_with(base));
        assert!(out.contains(CORRECTED_SUFFIX));
        // The low-entropy token is rendered, never raw bytes / secrets (SN-8).
        assert!(out.contains("failed (reason: dead-lettered)"));
    }

    #[test]
    fn replan_shaper_id_is_deterministic_and_distinct_per_round() {
        // Deterministic per round.
        assert_eq!(replan_shaper_id_material(1), replan_shaper_id_material(1));
        // Distinct across rounds.
        assert_ne!(replan_shaper_id_material(1), replan_shaper_id_material(2));
        // The built shaper's MoteId is a pure function of (round, model_id, prompt).
        let model = ModelId("qwen".into());
        let a = build_replan_shaper(&model, "p", 1);
        let b = build_replan_shaper(&model, "p", 1);
        assert_eq!(a.id, b.id);
        assert!(a.def.is_topology_shaper);
        assert!(a.parents.is_empty(), "re-plan shaper MUST be edge-free");
        // A different prompt / round is genuinely distinct work.
        assert_ne!(build_replan_shaper(&model, "p2", 1).id, a.id);
        assert_ne!(build_replan_shaper(&model, "p", 2).id, a.id);
    }
}
