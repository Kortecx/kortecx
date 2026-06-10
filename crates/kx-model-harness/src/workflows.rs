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
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
    MOTE_DEF_SCHEMA_VERSION,
};
use kx_runtime::workflow::WorkflowMote;
use kx_runtime::DemoWorkflow;
use kx_warrant::WarrantSpec;
use kx_workflow::CompiledWorkflow;
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

/// **M5.2 — first model-driven tool step.** A single WorldMutating
/// `StageThenCommit` *model* Mote that carries `prompt_text` AND declares the MCP
/// tool `(tool_id, tool_version)` in its `tool_contract` (so the broker's precheck
/// passes). Drive it through [`crate::Harness::drive_with_tool_broker`] with a
/// broker holding an `McpCapability` registered under `tool_id`: the model runs,
/// the runtime decodes its proposed tool call fail-closed, and dispatches it
/// through the warrant gate to the MCP capability. The caller's `warrant` MUST
/// grant `(tool_id, tool_version)`. `stc_crash_target = the Mote` so
/// `--crash-at pre-commit-stc` exercises exactly-once-under-crash on the MCP path.
#[must_use]
pub fn model_tool_call(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    prompt_text: &str,
    tool_id: &ToolName,
    tool_version: &kx_mote::ToolVersion,
) -> DemoWorkflow {
    let mut config_subset = BTreeMap::new();
    prompt::put_prompt(&mut config_subset, prompt_text);
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_id.clone(), tool_version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([0x50; 32]),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes([0x50; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset,
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: greedy(64),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    let m = Mote::new(
        def,
        InputDataId::from_bytes([0x50; 32]),
        GraphPosition(vec![0x50]),
        SmallVec::new(),
    );
    let m_id = m.id;
    // The Mote dispatches under the MCP tool's own name (the capability the broker
    // resolves), not the "kx-model" pseudo-capability — so the OUTER executor's
    // EffectRequest names the MCP tool and the inner broker's tool_contract +
    // tool_grants checks are coherent.
    let motes = vec![WorkflowMote {
        mote: m,
        warrant: warrant.clone(),
        capability: tool_id.clone(),
    }];
    DemoWorkflow {
        motes,
        stc_crash_target: m_id,
        vtc_crash_target: sentinel_shaper(),
        shaper_id: sentinel_shaper(),
    }
}

/// **M6 — the planner step.** A single READ-ONLY-NONDET *model* Mote carrying the
/// `planning_prompt`; the model's output (a structured plan envelope) commits as
/// the Mote's `result_ref` — the plan is a content-addressed FACT (D74). The
/// warrant grants no tools, so the broker's fail-closed `parse_tool_call` returns
/// `None` and the completion bytes (the plan) are committed verbatim. ROND ⇒ the
/// planner re-samples on a *fresh* run by design, but on REPLAY the committed plan
/// is served, never re-sampled (`vtc_crash_target = the planner`, so the
/// `post-commit-vtc` crash test proves a recovered run re-reads the plan).
///
/// Drive it through the same orchestrator as the A–J rows, read the committed plan
/// back via `projection.result_ref_of(&planner_id)` → `store.get`, then
/// `kx_planner::compile_plan` it and run the result via [`from_compiled`].
#[must_use]
pub fn planner_mote(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    planning_prompt: &str,
) -> DemoWorkflow {
    let planner = mote(
        0x60,
        model_id,
        Some(planning_prompt),
        sampled(256, 0x0050_1a2b),
        NdClass::ReadOnlyNondet,
        EffectPattern::IdempotentByConstruction,
        None,
        None,
        &[],
    );
    let planner_id = planner.id;
    // ROND ⇒ a crash AFTER commit must re-READ the plan (served, not re-sampled).
    wrap(vec![planner], warrant, sentinel_shaper(), planner_id)
}

/// **PR-2 (F-4) — the model-driven topology shaper.** A single READ-ONLY-NONDET
/// *model* Mote with `is_topology_shaper = true`, carrying `planning_prompt`. The
/// model's lowered [`kx_mote::TopologyDecision`] commits as the shaper's
/// `result_ref` (a captured fact, D76); the `DefaultTopologyMaterializer` spawns
/// its children, which execute and cold-refold to byte-identical `MoteId`s (R49 —
/// the model's choice is replayed, never re-sampled). `shaper_id` is the shaper
/// itself (NOT the sentinel), so `run_with_seams` takes the topology-materializing
/// path. Drive it through [`crate::topology_provider::run_model_loop`].
///
/// GREEDY params: the committed decision is served (not re-sampled) on recovery
/// regardless, but greedy keeps a live decode reproducible for the campaign.
#[must_use]
pub fn loop_shaper(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    planning_prompt: &str,
    seed: u8,
) -> DemoWorkflow {
    let mut config_subset = BTreeMap::new();
    prompt::put_prompt(&mut config_subset, planning_prompt);
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([seed; 32]),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes([seed; 32]),
        tool_contract: BTreeMap::new(),
        // ROND: the planner samples a plan; the COMMITTED decision is the fact
        // (served, not re-sampled, on replay). R-14: a shaper is never WM.
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: greedy(128),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    let shaper = Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    );
    let shaper_id = shaper.id;
    DemoWorkflow {
        motes: vec![WorkflowMote {
            mote: shaper,
            warrant: warrant.clone(),
            capability: ToolName(CAPABILITY.to_string()),
        }],
        stc_crash_target: sentinel_shaper(),
        vtc_crash_target: sentinel_shaper(),
        shaper_id,
    }
}

/// **PR-3 (AL2) — a re-plan round's topology shaper.** Like [`loop_shaper`], but
/// its identity is derived from the `round` index (a 32-byte blake3 namespace),
/// NOT a single seed byte — so each re-plan round's shaper has a DISTINCT,
/// deterministic, replay-stable `MoteId` that can never collide with the round-0
/// [`loop_shaper`] or a sibling round (the [`crate::run_replan_loop`] crash-safety
/// precondition: a re-plan round's shaper is a pure function of the round, so a
/// cold re-fold reconstructs the SAME chain). `planning_prompt` is the
/// failure-corrected instruction the driver builds from the prior round's
/// dead-lettered step(s); it is carried in `config_subset` (identity-bearing), so
/// the corrective intent is part of the committed shaper fact.
#[must_use]
pub fn replan_shaper(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    planning_prompt: &str,
    round: u32,
) -> DemoWorkflow {
    // Round-namespaced 32-byte identity material — deterministic + distinct per
    // round, and cryptographically distinct from a `loop_shaper` `[seed; 32]`.
    let mut material = b"kx-replan-round".to_vec();
    material.extend_from_slice(&round.to_le_bytes());
    let id_bytes = *blake3::hash(&material).as_bytes();

    let mut config_subset = BTreeMap::new();
    prompt::put_prompt(&mut config_subset, planning_prompt);
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        tool_contract: BTreeMap::new(),
        // ROND + shaper (R-14: a shaper is never WM); the COMMITTED decision is
        // the served fact (R49), greedy so a live decode stays reproducible.
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: greedy(128),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    let shaper = Mote::new(
        def,
        InputDataId::from_bytes(id_bytes),
        GraphPosition(id_bytes.to_vec()),
        SmallVec::new(),
    );
    let shaper_id = shaper.id;
    DemoWorkflow {
        motes: vec![WorkflowMote {
            mote: shaper,
            warrant: warrant.clone(),
            capability: ToolName(CAPABILITY.to_string()),
        }],
        stc_crash_target: sentinel_shaper(),
        vtc_crash_target: sentinel_shaper(),
        shaper_id,
    }
}

/// **PR-4 (M5) — a ReAct loop TURN.** A single READ-ONLY-NONDET *model* Mote that
/// carries `instruction` and (via `prior_trajectory`) a Data edge to EVERY prior
/// turn-output + observation Mote, so [`kx_context_assembler::assemble`] reconstructs
/// the full Reason/Act/Observe transcript into the model window (D78). Its identity
/// is round-namespaced (`blake3("kx-react-turn" || turn)`) — distinct + replay-stable
/// per turn (the [`crate::react::run_react_loop`] crash-safety precondition: a turn is
/// a pure function of its index, so a cold re-fold reconstructs the SAME chain). The
/// model's RAW output (a `{"tool_call": …}` envelope OR a final answer) commits as the
/// turn's `result_ref` — a captured fact (D76) the driver re-decodes via
/// [`crate::toolcall::parse_tool_call`] on replay. NOT a topology shaper (a ReAct turn
/// does not fan out children; the driver chains the next turn). The `warrant` grants
/// the tools (so `assemble` emits the tool menu); the turn carries NO `tool_contract`
/// (it proposes, it does not fire — the separate [`react_tool_mote`] fires).
#[must_use]
pub fn react_turn(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    instruction: &str,
    turn: u32,
    prior_trajectory: &[MoteId],
) -> DemoWorkflow {
    react_turn_salted(model_id, warrant, instruction, turn, prior_trajectory, &[])
}

/// **PR-2d-1 (react-substrate) — the RUN-SALTED [`react_turn`].** Identical to
/// the unsalted builder except the identity material becomes
/// `blake3("kx-react-turn" ‖ salt ‖ turn)` and a non-empty salt additionally
/// writes [`kx_mote::REACT_TURN_KEY`] (value = the salt) into `config_subset`
/// — the routing marker the live gateway's `ModelRouterExecutor` reads.
///
/// The harness drives ONE journal per run, so `blake3("kx-react-turn" ‖ turn)`
/// is collision-free there — but live serve SHARES one journal across runs,
/// where an unsalted turn 0 of run B would dedup-collide with run A's
/// (red-team BLOCKER #1). The salt is the run's registered `instance_id`
/// (server-assigned, unknowable client-side — SN-8), mirroring
/// `kx_journal::run_root_id`'s `blake3("kx-run-root" ‖ instance_id)`.
///
/// An EMPTY salt is byte-identical to the pre-PR-2d-1 builder (same material,
/// no marker key) — pinned by `react_identity_goldens`, so every existing
/// harness golden is unchanged. The coordinator's `react_shape::build_react_turn`
/// re-implements this builder below the dep wall; the two are pinned
/// byte-equivalent by a shared frozen golden (the `replan_shape` precedent).
#[must_use]
pub fn react_turn_salted(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    instruction: &str,
    turn: u32,
    prior_trajectory: &[MoteId],
    salt: &[u8],
) -> DemoWorkflow {
    // Round-namespaced 32-byte identity — deterministic + distinct per turn, and
    // cryptographically distinct from a `loop_shaper`/`replan_shaper` namespace.
    let mut material = b"kx-react-turn".to_vec();
    material.extend_from_slice(salt);
    material.extend_from_slice(&turn.to_le_bytes());
    let id_bytes = *blake3::hash(&material).as_bytes();

    let mut config_subset = BTreeMap::new();
    prompt::put_prompt(&mut config_subset, instruction);
    if !salt.is_empty() {
        config_subset.insert(
            kx_mote::ConfigKey(kx_mote::REACT_TURN_KEY.to_string()),
            kx_mote::ConfigVal(salt.to_vec()),
        );
    }
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        // No tool_contract: the turn PROPOSES; firing is the tool Mote's job.
        tool_contract: BTreeMap::new(),
        // ROND: the model samples; the COMMITTED output is the served fact (R49,
        // never re-sampled on replay). Greedy keeps a live decode reproducible.
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        // Output budget = the warrant's ceiling (never wider — `inference_params_from_mote`
        // refuses a widening, D35). The caller sizes the warrant for the turn's needs.
        inference_params: greedy(warrant.model_route.max_output_tokens),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    // Data edges to the full prior trajectory (turn outputs + observations).
    let parents: SmallVec<[ParentRef; 4]> = prior_trajectory
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect();
    let turn_mote = Mote::new(
        def,
        InputDataId::from_bytes(id_bytes),
        GraphPosition(id_bytes.to_vec()),
        parents,
    );
    DemoWorkflow {
        motes: vec![WorkflowMote {
            mote: turn_mote,
            warrant: warrant.clone(),
            capability: ToolName(CAPABILITY.to_string()),
        }],
        stc_crash_target: sentinel_shaper(),
        vtc_crash_target: sentinel_shaper(),
        shaper_id: sentinel_shaper(),
    }
}

/// **PR-4 (M5) — a ReAct loop OBSERVATION.** The WorldMutating `StageThenCommit` tool
/// Mote that fires the tool the model proposed at `turn` (decoded + warrant-checked by
/// the driver), with a Data edge to its `turn_mote_id` (durable lineage). Its
/// committed `result_ref` is the OBSERVATION the next [`react_turn`] reads back.
/// Round-namespaced identity (`blake3("kx-react-tool" || turn)`) — distinct +
/// replay-stable. It declares `(tool_id, tool_version)` in its `tool_contract` and
/// dispatches under the tool's own name, so [`crate::broker::dispatch_decoded_call`]'s
/// `tool_broker.precheck` (tool_contract / grants / net_scope) is coherent. The
/// `warrant` MUST grant `(tool_id, tool_version)`.
#[must_use]
pub fn react_tool_mote(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    turn_mote_id: MoteId,
) -> WorkflowMote {
    react_tool_mote_salted(
        model_id,
        warrant,
        tool_id,
        tool_version,
        turn,
        turn_mote_id,
        &[],
    )
}

/// **PR-2d-1 (react-substrate) — the RUN-SALTED [`react_tool_mote`].** Identity
/// material becomes `blake3("kx-react-tool" ‖ salt ‖ turn)`; an EMPTY salt is
/// byte-identical to the pre-PR-2d-1 builder (see [`react_turn_salted`] for the
/// shared-journal collision rationale). The observation Mote carries NO marker
/// key — its `config_subset` stays EMPTY (the PR-2d-2 `ToolArgsSink` contract:
/// args travel out-of-band so the observation identity never moves).
#[must_use]
pub fn react_tool_mote_salted(
    model_id: &ModelId,
    warrant: &WarrantSpec,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    turn_mote_id: MoteId,
    salt: &[u8],
) -> WorkflowMote {
    let mut material = b"kx-react-tool".to_vec();
    material.extend_from_slice(salt);
    material.extend_from_slice(&turn.to_le_bytes());
    let id_bytes = *blake3::hash(&material).as_bytes();

    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_id.clone(), tool_version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        tool_contract,
        // The tool effect is world-mutating by default → StageThenCommit (D66),
        // so a crash-recovery re-dispatch is exactly-once (content-addressed +
        // run-scoped idempotency key). No prompt: it fires the decoded call.
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    let tool_mote = Mote::new(
        def,
        InputDataId::from_bytes(id_bytes),
        GraphPosition(id_bytes.to_vec()),
        std::iter::once(ParentRef {
            parent_id: turn_mote_id,
            edge: EdgeMeta::data(),
        })
        .collect::<SmallVec<[ParentRef; 4]>>(),
    );
    WorkflowMote {
        mote: tool_mote,
        warrant: warrant.clone(),
        capability: tool_id.clone(),
    }
}

/// **M6 — run a planner-produced DAG.** Map a `kx_planner::compile_plan` result
/// (`CompiledMote { mote, warrant, capability }`) 1:1 to a flat (shaperless)
/// [`DemoWorkflow`], so it drives through `run_with_seams` with NO new execution
/// mechanism — a planner-produced DAG is "just another built workflow" (pivot §3.6).
#[must_use]
pub fn from_compiled(compiled: &CompiledWorkflow) -> DemoWorkflow {
    let motes = compiled
        .motes
        .iter()
        .map(|cm| WorkflowMote {
            mote: cm.mote.clone(),
            warrant: cm.warrant.clone(),
            capability: cm.capability.clone(),
        })
        .collect();
    DemoWorkflow {
        motes,
        stc_crash_target: sentinel_shaper(),
        vtc_crash_target: sentinel_shaper(),
        shaper_id: sentinel_shaper(),
    }
}

#[cfg(test)]
mod react_identity_tests {
    use super::*;
    use crate::harness_warrant;

    fn mid() -> ModelId {
        ModelId("kx-test:q8:deadbeef".to_string())
    }

    fn hex(id: &MoteId) -> String {
        use std::fmt::Write as _;
        id.as_bytes().iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// PR-2d-1 frozen goldens: (a) the EMPTY-salt builders produce the exact
    /// pre-PR-2d-1 identities (every existing harness golden is byte-unchanged);
    /// (b) the salted identities are the cross-impl contract the coordinator's
    /// `react_shape::build_react_turn` must reproduce byte-for-byte (the
    /// `replan_shape` precedent — the same hex is pinned on BOTH sides of the
    /// dep wall, so silent drift between the two builders fails a test).
    #[test]
    fn react_identity_goldens() {
        let m = mid();
        let w = harness_warrant(&m, 64, 5_000);

        // (a) empty salt == the pre-change builder (material has no salt bytes,
        //     no marker key) — the unsalted wrapper IS the salted fn with [].
        let unsalted = react_turn(&m, &w, "list the files", 0, &[]);
        let via_salted = react_turn_salted(&m, &w, "list the files", 0, &[], &[]);
        assert_eq!(unsalted.motes[0].mote.id, via_salted.motes[0].mote.id);
        assert!(
            !unsalted.motes[0]
                .mote
                .def
                .config_subset
                .contains_key(&kx_mote::ConfigKey(kx_mote::REACT_TURN_KEY.to_string())),
            "an unsalted turn must NOT carry the react marker"
        );
        assert_eq!(
            hex(&unsalted.motes[0].mote.id),
            "9aa916e81e26e54f6f35f0cd009e4dbf901d6d8da0f93aa7e548d7cc27c16257",
            "the empty-salt react_turn identity moved — every harness golden breaks"
        );

        // (b) the salted identity (the harness↔serve byte-equivalence contract).
        let salt = [0x4d_u8; 16];
        let salted = react_turn_salted(&m, &w, "list the files", 0, &[], &salt);
        assert_ne!(salted.motes[0].mote.id, unsalted.motes[0].mote.id);
        assert_eq!(
            salted.motes[0]
                .mote
                .def
                .config_subset
                .get(&kx_mote::ConfigKey(kx_mote::REACT_TURN_KEY.to_string()))
                .map(|v| v.0.clone()),
            Some(salt.to_vec()),
            "a salted turn carries the marker key with the salt as value"
        );
        assert_eq!(
            hex(&salted.motes[0].mote.id),
            "f2e465451f434a861090109d336c39a8307e5d539963fd48b3470df84458a5cb",
            "the salted react_turn identity moved — the coordinator react_shape \
             golden (pinned to the same hex) must move in lock-step"
        );

        // Cross-run isolation: distinct salts ⇒ distinct identities; same salt
        // ⇒ the same identity (replay-stable).
        let other = react_turn_salted(&m, &w, "list the files", 0, &[], &[0x4e; 16]);
        assert_ne!(other.motes[0].mote.id, salted.motes[0].mote.id);
        let again = react_turn_salted(&m, &w, "list the files", 0, &[], &salt);
        assert_eq!(again.motes[0].mote.id, salted.motes[0].mote.id);

        // The tool-observation builder mirrors all of the above (no marker key —
        // its config_subset stays EMPTY per the ToolArgsSink contract).
        let turn_id = salted.motes[0].mote.id;
        let tool = ToolName("mcp-echo".to_string());
        let ver = ToolVersion("1".to_string());
        let obs_unsalted = react_tool_mote(&m, &w, &tool, &ver, 0, turn_id);
        let obs_salted = react_tool_mote_salted(&m, &w, &tool, &ver, 0, turn_id, &salt);
        assert_ne!(obs_unsalted.mote.id, obs_salted.mote.id);
        assert!(obs_salted.mote.def.config_subset.is_empty());
        assert_eq!(
            hex(&obs_unsalted.mote.id),
            "3837994781ee3a5e9254a634adc55e5eafbe354fcaafb9487445f1537840a59f",
            "the empty-salt react_tool_mote identity moved"
        );
        assert_eq!(
            hex(&obs_salted.mote.id),
            "0797b93286b999344db0ba9a458a83105c6a6b55c29760e510311ae45ff68048",
            "the salted react_tool_mote identity moved"
        );
    }
}
