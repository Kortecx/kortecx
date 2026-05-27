// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! D50 — identity-distinguishing tests for `MoteDef.inference_params`.
//!
//! The pre-D50 latent bug: `MoteDef::hash` excluded decoding parameters,
//! so two NONDET Motes differing only in `temperature_bps` (or any
//! other decoding field) produced the same `mote_def_hash` and the
//! same `MoteId`. `kx-memoizer::lookup` keys on `MoteId` alone — a
//! cache hit could serve greedy output to a temp=0.7 caller.
//!
//! D50 fixes this by making `inference_params` a first-class
//! identity-bearing field of `MoteDef` (per D4 — identity-bearing types
//! live with the substrate). These tests assert the fix at the identity
//! layer; `kx-memoizer/tests/d50_decoding_params_partition.rs` exercises
//! the downstream memoizer behaviour.

use std::collections::BTreeMap;

use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use proptest::prelude::*;
use smallvec::SmallVec;

fn nondet_def(params: InferenceParams) -> MoteDef {
    MoteDef {
        logic_ref: LogicRef::from_bytes([0x11; 32]),
        model_id: ModelId("llama-3-8b:q4".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0x22; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: params,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn mote_for(def: MoteDef) -> Mote {
    Mote::new(
        def,
        InputDataId::from_bytes([0x33; 32]),
        GraphPosition(b"root".to_vec()),
        SmallVec::new(),
    )
}

#[test]
fn temperature_differs_produces_different_mote_def_hash() {
    let greedy = InferenceParams::default();
    let warm = InferenceParams {
        temperature_bps: 5_000,
        ..InferenceParams::default()
    };
    let a = nondet_def(greedy);
    let b = nondet_def(warm);
    assert_ne!(
        a.hash(),
        b.hash(),
        "MoteDef::hash MUST distinguish two MoteDefs differing only in \
         inference_params.temperature_bps — D50 identity invariant. This is \
         the assertion that would have caught the pre-D50 latent bug."
    );
}

#[test]
fn temperature_differs_produces_different_mote_id() {
    let a = mote_for(nondet_def(InferenceParams::default()));
    let b = mote_for(nondet_def(InferenceParams {
        temperature_bps: 7_500,
        ..InferenceParams::default()
    }));
    assert_ne!(
        a.id, b.id,
        "MoteId MUST distinguish two Motes whose MoteDefs differ only in \
         inference_params.temperature_bps."
    );
}

#[test]
fn seed_differs_produces_different_mote_id() {
    let a = mote_for(nondet_def(InferenceParams {
        temperature_bps: 5_000,
        seed: 42,
        ..InferenceParams::default()
    }));
    let b = mote_for(nondet_def(InferenceParams {
        temperature_bps: 5_000,
        seed: 1337,
        ..InferenceParams::default()
    }));
    assert_ne!(
        a.id, b.id,
        "MoteId MUST distinguish two Motes whose MoteDefs differ only in \
         inference_params.seed — required for reproducible-stochastic \
         identity semantics."
    );
}

#[test]
fn default_inference_params_preserve_greedy_identity_across_schema_bump() {
    // The greedy-default identity is the workspace's lowest-common-denominator
    // — every test fixture that omitted `inference_params` pre-D50 now uses
    // `InferenceParams::default()`. This test pins the values so a future
    // accidental drift (e.g., changing top_p_bps default) would surface as
    // an assertion failure.
    let p = InferenceParams::default();
    assert_eq!(p.max_output_tokens, 512);
    assert_eq!(p.temperature_bps, 0);
    assert_eq!(p.top_p_bps, 10_000);
    assert_eq!(p.top_k, 0);
    assert_eq!(p.seed, 0);
    assert!(p.stop_tokens.is_empty());
    assert!(p.grammar.is_none());
}

proptest! {
    /// SN-4 v2 #5 (property test): any pair of MoteDefs differing in any
    /// single decoding-param field MUST produce a different `mote_def_hash`.
    /// Closes the input-space-of-bugs that hand-picked tests cannot.
    #[test]
    fn prop_any_inference_params_change_changes_mote_def_hash(
        max_output_tokens in 1u32..4096,
        temperature_bps in 0u32..10_000,
        top_p_bps in 1u32..=10_000,
        top_k in 0u32..256,
        seed in 0u32..u32::MAX,
    ) {
        let baseline = nondet_def(InferenceParams::default());
        let baseline_hash = baseline.hash();

        // Each variant changes exactly one field away from default. The hash
        // MUST differ for every variant that genuinely differs in any field.
        let variants = [
            ("max_output_tokens", InferenceParams { max_output_tokens, ..InferenceParams::default() }),
            ("temperature_bps", InferenceParams { temperature_bps, ..InferenceParams::default() }),
            ("top_p_bps", InferenceParams { top_p_bps, ..InferenceParams::default() }),
            ("top_k", InferenceParams { top_k, ..InferenceParams::default() }),
            ("seed", InferenceParams { seed, ..InferenceParams::default() }),
        ];
        for (field, p) in variants {
            let default = InferenceParams::default();
            // Determine whether this variant actually differs from default on its
            // axis; if not, skip (the strategy may generate the default value).
            let differs = match field {
                "max_output_tokens" => p.max_output_tokens != default.max_output_tokens,
                "temperature_bps" => p.temperature_bps != default.temperature_bps,
                "top_p_bps" => p.top_p_bps != default.top_p_bps,
                "top_k" => p.top_k != default.top_k,
                "seed" => p.seed != default.seed,
                _ => false,
            };
            if !differs {
                continue;
            }
            let variant_hash = nondet_def(p).hash();
            prop_assert_ne!(
                variant_hash,
                baseline_hash,
                "inference_params.{} change MUST alter mote_def_hash",
                field
            );
        }
    }
}
