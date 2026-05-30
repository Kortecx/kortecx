//! Ergonomic builders for the common Morphic step kinds, plus the concrete
//! **data-synthesis recipe** ([`synthesis_pipeline`]) the P4.1 spec calls out:
//! generator → transform → deterministic critic → content-addressed corpus.
//!
//! The builders pick the `nd_class` / `effect_pattern` combination that the
//! executor's refusal predicates accept for each role, so authors don't have to
//! memorize them (e.g. R-14 forbids a WORLD-MUTATING topology shaper — the
//! shaper builder is READ-ONLY-NONDET). Returned [`StepDef`]s have public
//! fields, so anything the builder defaults can be overridden afterwards.

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_critic_types::CheckSpec;
use kx_mote::{
    EdgeMeta, EffectPattern, InferenceParams, LogicRef, ModelId, NdClass, PromptTemplateHash,
    ToolName,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

use crate::def::{StepDef, StepRef, StepRole, WorkflowDef};
use crate::error::CompileError;

/// A permissive warrant suitable for local, single-process development and
/// tests — every axis is wide open. The broker/executor seams still enforce
/// structurally; this warrant simply narrows nothing.
///
/// Model-route limits are deliberately positive: the topology materializer
/// narrows a shaper's warrant against each child role via `kx_warrant::intersect`,
/// which rejects a zeroed model route as invalid.
#[must_use]
pub fn permissive_warrant(model_id: ModelId) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id,
            max_input_tokens: 4096,
            max_output_tokens: 4096,
            max_calls: 16,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

/// Build a [`StepDef`] for the given role with sensible defaults (zeroed prompt
/// template, empty tool contract + config, greedy inference params). Callers
/// override any public field on the result as needed.
pub(crate) fn step(
    logic_ref: LogicRef,
    model_id: ModelId,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    role: StepRole,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    StepDef {
        logic_ref,
        model_id,
        prompt_template_hash: PromptTemplateHash::from_bytes([0; 32]),
        tool_contract: BTreeMap::new(),
        nd_class,
        config_subset: BTreeMap::new(),
        effect_pattern,
        inference_params: InferenceParams::default(),
        role,
        warrant,
        capability,
    }
}

/// A generator step: samples new content (READ-ONLY-NONDET, `StageThenCommit`).
/// Its committed `result_ref` IS the generated payload.
#[must_use]
pub fn generator(
    logic_ref: LogicRef,
    model_id: ModelId,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    step(
        logic_ref,
        model_id,
        NdClass::ReadOnlyNondet,
        EffectPattern::StageThenCommit,
        StepRole::Plain,
        warrant,
        capability,
    )
}

/// A transform step: a deterministic function of its inputs (PURE,
/// `IdempotentByConstruction`).
#[must_use]
pub fn transform(
    logic_ref: LogicRef,
    model_id: ModelId,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    step(
        logic_ref,
        model_id,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        StepRole::Plain,
        warrant,
        capability,
    )
}

/// A critic step validating `producer`: a deterministic check (PURE,
/// `IdempotentByConstruction`) — schema / dedup / stat-bounds / PII-leakage.
/// Declare a dependency edge from `producer` to this step so the producer
/// precedes it in the DAG.
#[must_use]
pub fn critic(
    producer: StepRef,
    logic_ref: LogicRef,
    model_id: ModelId,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    step(
        logic_ref,
        model_id,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        StepRole::Critic { producer },
        warrant,
        capability,
    )
}

/// A **deterministic critic** validating `producer` (D60 / P4.2-2): a PURE,
/// `IdempotentByConstruction` step carrying a [`CheckSpec`] (schema / dedup /
/// stat-bounds / PII-leakage). The check folds into the critic's `MoteId`, and
/// at runtime the executor evaluates it in-process against the producer's
/// committed bytes (`kx_executor::run_native_critic_mote`) and commits a
/// `CriticVerdict` — no model, decorrelated from the producer. Declare a
/// dependency edge from `producer` to this step so the producer precedes it in
/// the DAG. The compiled `MoteDef` satisfies executor refusal R-15 by
/// construction (PURE + `critic_for=Some` + `!is_topology_shaper`).
#[must_use]
pub fn deterministic_critic(
    producer: StepRef,
    check: CheckSpec,
    logic_ref: LogicRef,
    model_id: ModelId,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    step(
        logic_ref,
        model_id,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        StepRole::DeterministicCritic { producer, check },
        warrant,
        capability,
    )
}

/// A topology shaper (READ-ONLY-NONDET; R-14 forbids WORLD-MUTATING shapers).
/// At runtime it commits a `TopologyDecision` from which the projection
/// materializes children deterministically — its dynamic fan-out is not static
/// compile output.
#[must_use]
pub fn topology_shaper(
    logic_ref: LogicRef,
    model_id: ModelId,
    warrant: WarrantSpec,
    capability: ToolName,
) -> StepDef {
    step(
        logic_ref,
        model_id,
        NdClass::ReadOnlyNondet,
        EffectPattern::IdempotentByConstruction,
        StepRole::TopologyShaper,
        warrant,
        capability,
    )
}

/// The canonical data-synthesis recipe wired as a [`WorkflowDef`]:
///
/// ```text
/// generator (ROND) ──data──> transform (PURE) ──data──> critic (PURE)
/// ```
///
/// Reproducible by construction: pin the workflow `seed` (folded into the
/// generator's entrypoint `input_data_id`) plus each step's model + inference
/// params (folded into its `mote_def_hash`, D50) and the whole corpus
/// regenerates byte-identically. The three `logic_ref`s name the generator,
/// transform, and critic logic respectively.
///
/// # Errors
///
/// Propagates [`CompileError`] from edge declaration (it never fails for this
/// fixed three-step shape, but the signature keeps edge wiring honest).
pub fn synthesis_pipeline(
    seed: u32,
    model_id: ModelId,
    capability: ToolName,
    gen_logic: LogicRef,
    transform_logic: LogicRef,
    critic_logic: LogicRef,
) -> Result<WorkflowDef, CompileError> {
    let warrant = permissive_warrant(model_id.clone());
    let mut wf = WorkflowDef::new(seed);

    let g = wf.add_step(generator(
        gen_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    ));
    let t = wf.add_step(transform(
        transform_logic,
        model_id.clone(),
        warrant.clone(),
        capability.clone(),
    ));
    let c = wf.add_step(critic(t, critic_logic, model_id, warrant, capability));

    wf.add_edge(g, t, EdgeMeta::data())?;
    wf.add_edge(t, c, EdgeMeta::data())?;
    Ok(wf)
}
