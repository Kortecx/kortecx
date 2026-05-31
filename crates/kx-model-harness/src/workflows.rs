//! Per-row workflow builders for the A–J matrix, expressed as flat (shaperless)
//! `kx_runtime::DemoWorkflow` values driven through `run_with_seams`.
//!
//! Identity is fixed from per-Mote seed bytes (so a workflow rebuilds to the
//! same `MoteId`s — the replay precondition), plus the prompt carried in
//! `config_subset` and the params/model_id in `MoteDef` (all identity-bearing).

use std::collections::BTreeMap;

use kx_critic_types::CheckSpec;
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, MOTE_DEF_SCHEMA_VERSION,
};
use kx_runtime::workflow::WorkflowMote;
use kx_runtime::DemoWorkflow;
use kx_warrant::WarrantSpec;
use smallvec::SmallVec;

use crate::prompt;

/// The capability every harness dispatch routes through (the broker keys on
/// prompt presence, not the name).
pub const CAPABILITY: &str = "kx-model";

/// Greedy (deterministic) decoding params capped at `max_output_tokens`.
#[must_use]
pub fn greedy(max_output_tokens: u32) -> InferenceParams {
    InferenceParams {
        max_output_tokens,
        ..InferenceParams::default()
    }
}

/// Sampled (stochastic) decoding params: temperature 0.8, top-p 0.95, top-k 40,
/// pinned `seed`. A pinned seed makes even a sampled decode reproducible given
/// the same (model, prompt, params).
#[must_use]
pub fn sampled(max_output_tokens: u32, seed: u32) -> InferenceParams {
    InferenceParams {
        max_output_tokens,
        temperature_bps: 8_000,
        top_p_bps: 9_500,
        top_k: 40,
        seed,
        ..InferenceParams::default()
    }
}

/// A `MoteId` that never matches a workflow Mote — used as the shaperless
/// sentinel `shaper_id` so `run_with_seams` takes the flat-DAG path.
#[must_use]
pub fn sentinel_shaper() -> MoteId {
    MoteId::from_bytes([0xFF; 32])
}

/// Build one Mote. `prompt = Some` makes it a model Mote (the executor/broker
/// runs inference); `None` makes it a deterministic non-model Mote.
#[allow(clippy::too_many_arguments)]
fn mote(
    seed: u8,
    model_id: &ModelId,
    prompt_text: Option<&str>,
    params: InferenceParams,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    critic_check: Option<CheckSpec>,
    parents: &[ParentRef],
) -> Mote {
    let mut config_subset = BTreeMap::new();
    if let Some(p) = prompt_text {
        prompt::put_prompt(&mut config_subset, p);
    }
    let def = MoteDef {
        critic_check,
        logic_ref: LogicRef::from_bytes([seed; 32]),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes([seed; 32]),
        tool_contract: BTreeMap::new(),
        nd_class,
        config_subset,
        effect_pattern,
        critic_for,
        is_topology_shaper: false,
        inference_params: params,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents
            .iter()
            .copied()
            .collect::<SmallVec<[ParentRef; 4]>>(),
    )
}

fn data_parent(parent: &Mote) -> ParentRef {
    ParentRef {
        parent_id: parent.id,
        edge: EdgeMeta::data(),
    }
}

fn wrap(motes: Vec<Mote>, warrant: &WarrantSpec, stc: MoteId, vtc: MoteId) -> DemoWorkflow {
    let cap = ToolName(CAPABILITY.to_string());
    let motes = motes
        .into_iter()
        .map(|m| WorkflowMote {
            mote: m,
            warrant: warrant.clone(),
            capability: cap.clone(),
        })
        .collect();
    DemoWorkflow {
        motes,
        stc_crash_target: stc,
        vtc_crash_target: vtc,
        shaper_id: sentinel_shaper(),
    }
}

/// **Row A — exit gate.** A WorldMutating (`ValidateThenCommit`) model producer
/// `P` whose committed output is gated by a native deterministic critic carrying
/// `check`; a PURE consumer `C` reads `P`. The producer commit schedules the
/// critic (R-2 sibling). If the critic returns `Valid`, `P` is `Promoted` and
/// `C` runs; if `Invalid`, `P` is `Unpromoted` and `C` is withheld (fail-closed,
/// the run stalls with `C` uncommitted).
#[must_use]
pub fn exit_gate(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    prompt_text: &str,
    check: CheckSpec,
) -> DemoWorkflow {
    let producer = mote(
        0x10,
        model_id,
        Some(prompt_text),
        greedy(48),
        NdClass::WorldMutating,
        EffectPattern::ValidateThenCommit,
        None,
        None,
        &[],
    );
    let critic = mote(
        0x11,
        model_id,
        None,
        InferenceParams::default(),
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        Some(check),
        &[data_parent(&producer)],
    );
    let consumer = mote(
        0x12,
        model_id,
        None,
        InferenceParams::default(),
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        None,
        &[data_parent(&producer)],
    );
    wrap(
        vec![producer, critic, consumer],
        warrant,
        sentinel_shaper(),
        sentinel_shaper(),
    )
}

/// **Row C — serve-not-re-sample.** A ReadOnlyNondet model producer `P`
/// (stochastic: re-running would re-sample) and a PURE consumer `C` reading `P`.
/// `vtc_crash_target = P` so `--crash-at post-commit-vtc` aborts the instant
/// `P`'s `Committed` is durable. Recovery must RE-READ `P` (served, never
/// re-sampled) and then run `C`.
#[must_use]
pub fn serve_chain(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    prompt_text: &str,
    seed: u32,
) -> DemoWorkflow {
    let producer = mote(
        0x20,
        model_id,
        Some(prompt_text),
        sampled(48, seed),
        NdClass::ReadOnlyNondet,
        EffectPattern::IdempotentByConstruction,
        None,
        None,
        &[],
    );
    let consumer = mote(
        0x21,
        model_id,
        None,
        InferenceParams::default(),
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        None,
        &[data_parent(&producer)],
    );
    let producer_id = producer.id;
    wrap(
        vec![producer, consumer],
        warrant,
        sentinel_shaper(),
        producer_id,
    )
}

/// **Row D — reproducibility.** A single model producer + PURE consumer, with
/// caller-chosen `params`/`nd_class`: greedy+PURE for byte-reproducibility, or
/// sampled+ROND for the "digest differs but guarantees hold" contrast.
#[must_use]
pub fn model_chain(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    prompt_text: &str,
    params: InferenceParams,
    nd_class: NdClass,
) -> DemoWorkflow {
    let producer = mote(
        0x30,
        model_id,
        Some(prompt_text),
        params,
        nd_class,
        EffectPattern::IdempotentByConstruction,
        None,
        None,
        &[],
    );
    let consumer = mote(
        0x31,
        model_id,
        None,
        InferenceParams::default(),
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        None,
        &[data_parent(&producer)],
    );
    wrap(
        vec![producer, consumer],
        warrant,
        sentinel_shaper(),
        sentinel_shaper(),
    )
}

/// **Row G — tool / MCP-shaped WM dispatch.** A WorldMutating `StageThenCommit`
/// tool Mote (no prompt → the broker stages a deterministic, content-addressed
/// response). `stc_crash_target = tool` so `--crash-at pre-commit-stc` aborts
/// after the effect is staged but before `Committed`; recovery re-dispatches and
/// the content-addressed dedup makes the effect exactly-once.
#[must_use]
pub fn tool_stage(model_id: &ModelId, warrant: &WarrantSpec) -> DemoWorkflow {
    let tool = mote(
        0x40,
        model_id,
        None,
        InferenceParams::default(),
        NdClass::WorldMutating,
        EffectPattern::StageThenCommit,
        None,
        None,
        &[],
    );
    let tool_id = tool.id;
    wrap(vec![tool], warrant, tool_id, sentinel_shaper())
}
