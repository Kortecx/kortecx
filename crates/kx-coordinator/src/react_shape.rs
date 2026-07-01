//! Coordinator-local ReAct-turn SHAPING primitives (PR-2d-1, react-substrate).
//!
//! These re-implement, BYTE-FOR-BYTE, the run-salted turn builder the
//! `kx-model-harness` exposes as `workflows::react_turn_salted` — re-implemented
//! here (not shared) because the coordinator sits BELOW the dep wall and must not
//! depend on `kx-model-harness` (the `replan_shape` precedent). The equivalence is
//! **load-bearing for R49**: the live coordinator and the harness must derive the
//! SAME turn `MoteId` for a given `(instance_id, turn, instruction, model_id,
//! max_output_tokens)`, or a cold re-fold of a harness-written journal on the live
//! binary (or vice-versa) would diverge. A frozen golden hex pins the equivalence
//! in tests on BOTH sides of the wall, so a drift on either copy fails CI.
//!
//! The turn is RUN-SALTED (`blake3("kx-react-turn" ‖ instance_id ‖ turn)`): the
//! harness drives one journal per run, where the unsalted `‖ turn` material is
//! collision-free — but live serve SHARES one journal across runs, where run B's
//! unsalted turn 0 would dedup-collide with run A's (red-team BLOCKER #1). The
//! salt is the server-assigned `instance_id` (SN-8: never client-controlled),
//! mirroring `kx_journal::run_root_id`.
//!
//! Pure + total + dependency-light: identity material is `blake3` via
//! [`ContentRef::of`] (kx-content), so the coordinator takes no direct `blake3`
//! dependency (D111 — `Cargo.lock` unchanged), exactly like `replan_shape.rs`.

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_journal::INSTANCE_ID_LEN;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId,
    LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName,
    ToolVersion, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY, REACT_TURN_KEY, RERANK_CANDIDATES_KEY,
    RERANK_QUERY_KEY, RERANK_TURN_KEY,
};
use smallvec::SmallVec;

/// The default per-run ReAct turn budget recorded on the turn-0 anchor, and the
/// HARD CEILING both caps are validated against at seed time (a seed-supplied
/// cap above it is refused LOUDLY — `ReactSeedRefused`). 8 is the harness
/// `ReactBudget::default()` turn count, so default-cap serve chains and harness
/// chains are identical-length (the cross-impl equivalence pin, R49). Caps are
/// recorded DURABLY at anchor time so a recovered coordinator enforces the
/// budget the run was admitted under, never a default that drifted across
/// binary versions (red-team BLOCKER #4).
pub const REACT_MAX_TURNS: u32 = 8;
/// The HARD CEILING on a chain's total tool-call (observation) budget
/// (T-MULTI-ELEMENT-TOOLCALLS). A seed cap above it is refused LOUDLY. DECOUPLED
/// from `REACT_MAX_TURNS`: a single model turn can now fire N tools at once (a
/// `ToolBatch`), so the total tool-call budget legitimately exceeds the model-turn
/// budget — the old `max_tool_calls < max_turns` coupling (≤1 tool per turn) no
/// longer holds. Mirrors `kx_journal::MAX_TOOL_BATCH_CALLS` (the per-turn cap) so a
/// single turn can, in the limit, fire up to the full chain budget.
pub const REACT_MAX_TOOL_CALLS: u32 = 20;
/// The DEFAULT per-run tool-call (observation) budget (PR-2d-2; raised 6 → 20 at
/// T-MULTI-ELEMENT-TOOLCALLS for parallel tool calling, user-directed). Now equal to
/// the ceiling: with batched calls a chain may legitimately fire many tools across
/// its model turns. Server-configurable via `GatewayConfig.react_max_tool_calls` /
/// `KX_SERVE_REACT_MAX_TOOL_CALLS` (surfaced read-only in Settings); chains anchored
/// under an older default keep their recorded caps (durable per-anchor).
pub const REACT_DEFAULT_MAX_TOOL_CALLS: u32 = 20;

/// Truncate a refusal reason to [`kx_journal::MAX_REJECTED_REASON_LEN`] chars at a
/// char boundary (deterministic, panic-free, total) before it freezes onto a
/// [`kx_journal::ReactBranch::Rejected`] fact — the journal's `MAX_ENTRY_LEN`
/// guard is the backstop, this is the per-field `DoS`/context-window bound.
#[must_use]
pub(crate) fn bounded_reason(reason: String) -> String {
    if reason.chars().count() <= kx_journal::MAX_REJECTED_REASON_LEN {
        reason
    } else {
        reason
            .chars()
            .take(kx_journal::MAX_REJECTED_REASON_LEN)
            .collect()
    }
}

/// PR-3 (A2 graceful recovery): render the NEXT turn's instruction after the
/// previous turn's tool proposal was REJECTED at the decode/validate authority
/// site. The model reads its own (rejected) proposal via the out-of-band
/// trajectory (`resolve_parent_context` contributes every prior turn's output);
/// this appends the fail-closed `reason` plus a fixed steer so it self-corrects
/// (fix the args, pick a granted tool, or answer directly).
///
/// PURE + total + deterministic: a function of `(base_instruction, reason)` only,
/// both frozen (the base prompt is the anchor's immutable blob; the reason is on
/// the durable `ReactBranch::Rejected` fact). A constant template — no clock, no
/// RNG, no map iteration — so the live drive and a recovery re-fold (and the
/// harness twin) build the byte-identical re-prompt turn. The `reason` is already
/// bounded to `MAX_REJECTED_REASON_LEN` at the fact, so this cannot blow the
/// turn's context window.
#[must_use]
pub(crate) fn render_reprompt(base_instruction: &str, reason: &str) -> String {
    format!(
        "{base_instruction}\n\n[Your previous tool call was REJECTED: {reason}\n\
         Correct it — call a tool you were granted with arguments that match its \
         schema, or answer the question directly if you cannot.]"
    )
}

/// W2 (settle-nudge): render the LAST useful tool-firing turn's instruction when a
/// chain is one round from exhausting its budget on a `Tool` tail. The model has
/// already gathered tool observations (served out-of-band via the F-7 trajectory)
/// but keeps proposing more tool calls; this appends a fixed steer instructing it
/// to STOP calling tools and answer directly from what it has observed, so the
/// chain settles on an `Answer` instead of quiescing answerless (the W2 finding —
/// a tool-looping model that never settles and dead-letters / `agent run` exit-1).
///
/// PURE + total + deterministic: a function of `base_instruction` ALONE (the
/// anchor's immutable base prompt). A CONSTANT suffix — no clock, RNG, reason
/// interpolation, or map iteration — so the live drive and a recovery re-fold build
/// the byte-identical nudged turn (and thus the byte-identical turn `MoteId`, since
/// the instruction rides `config_subset[PROMPT_KEY]`). The nudge needs NO durable
/// state: the decision is re-derived from the frozen `(tool_calls, turns_used,
/// caps, prev branch)` on every pass, exactly like the A2 [`render_reprompt`]. The
/// suffix is shorter than the worst-case re-prompt (no reason), so it can never
/// blow the turn's context window beyond what a non-nudged turn already carries.
#[must_use]
pub(crate) fn render_settle_nudge(base_instruction: &str) -> String {
    format!(
        "{base_instruction}\n\n[You have already gathered tool results, and your \
         tool-call budget is nearly exhausted. Do NOT call another tool. Using the \
         observations you already have, give your FINAL answer to the question now, \
         directly and in prose.]"
    )
}

/// The run-salted 32-byte identity material for a ReAct turn:
/// `blake3(b"kx-react-turn" ‖ instance_id ‖ turn.to_le_bytes())`. Deterministic +
/// distinct per `(run, turn)`, and cryptographically distinct from the
/// `loop_shaper`/`replan_shaper`/unsalted-harness namespaces. Mirrors
/// `kx_model_harness::workflows::react_turn_salted` (which uses `blake3::hash`
/// directly; [`ContentRef::of`] IS blake3-of-bytes, so the bytes are identical
/// without a direct `blake3` dependency).
#[must_use]
pub(crate) fn react_turn_id_material(instance_id: &[u8; INSTANCE_ID_LEN], turn: u32) -> [u8; 32] {
    let mut material = b"kx-react-turn".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(&turn.to_le_bytes());
    *ContentRef::of(&material).as_bytes()
}

/// Re-derive a ReAct turn's `Mote` from `(model_id, instruction, turn,
/// instance_id, max_output_tokens)` — byte-for-byte identical to the harness
/// `workflows::react_turn_salted` builder, so the derived `MoteId` matches the
/// harness oracle (R49) and the in-flight-turn identity the recovery pass checks.
///
/// The turn is ROND (the model samples; the COMMITTED output is the served fact,
/// never re-sampled on replay), greedy-decoded at `max_output_tokens` (the
/// warrant's output ceiling — the harness uses
/// `greedy(warrant.model_route.max_output_tokens)`), carries the instruction in
/// `config_subset[PROMPT_KEY]` plus the [`REACT_TURN_KEY`] routing marker
/// (value = the salt) — both identity-bearing — and is **EDGE-FREE** (empty
/// parents): the trajectory is served out-of-band via the F-7 react special-case
/// in `resolve_parent_context`, so a turn never moves the canonical digest via
/// `encode_state` edges. NOT a topology shaper (a turn does not fan out children;
/// the settle pass chains the next turn).
#[must_use]
pub(crate) fn build_react_turn(
    model_id: &ModelId,
    instruction: &str,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    max_output_tokens: u32,
) -> Mote {
    let id_bytes = react_turn_id_material(instance_id, turn);

    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(instruction.as_bytes().to_vec()),
    );
    config_subset.insert(
        ConfigKey(REACT_TURN_KEY.to_string()),
        ConfigVal(instance_id.to_vec()),
    );
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        // No tool_contract: the turn PROPOSES; the OBSERVATION Mote fires.
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams {
            max_output_tokens,
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

/// The run-salted 32-byte identity material for a ReAct OBSERVATION (the tool
/// Mote that fires the model's frozen decision at `turn`):
/// `blake3(b"kx-react-tool" ‖ instance_id ‖ turn.to_le_bytes() ‖ [call_index if >0])`.
/// The TOOL identity is deliberately NOT in the material — it enters the `MoteId`
/// via `tool_contract` (def-hash), exactly like the harness
/// `kx_model_harness::workflows::react_tool_mote_salted`.
///
/// T-MULTI-ELEMENT-TOOLCALLS: a `ToolBatch` turn fires N observations; `call_index`
/// (the position of the call within the turn's frozen output) disambiguates them so
/// two calls to the SAME tool at the same turn never collide on one `MoteId` (the
/// red-team BLOCKER #1 dedup-collision class). The index is appended ONLY when `> 0`,
/// so a single-call turn (index 0, the `Tool` branch) is BYTE-IDENTICAL to every
/// pre-v13 chain — `SALTED_TOOL0_GOLDEN` holds and no shipped observation moves.
#[must_use]
pub(crate) fn react_tool_id_material(
    instance_id: &[u8; INSTANCE_ID_LEN],
    turn: u32,
    call_index: u32,
) -> [u8; 32] {
    let mut material = b"kx-react-tool".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(&turn.to_le_bytes());
    if call_index > 0 {
        material.extend_from_slice(&call_index.to_le_bytes());
    }
    *ContentRef::of(&material).as_bytes()
}

/// Re-derive a ReAct OBSERVATION `Mote` from the frozen `Tool` branch fact —
/// byte-for-byte identical (FULL Mote equality, parents included) to the harness
/// `workflows::react_tool_mote_salted`, so the observation the live coordinator
/// materializes is the observation the harness oracle derives (R49; the
/// cross-impl golden below pins it on both sides of the dep wall).
///
/// The observation is WorldMutating `StageThenCommit` (D66 — a crash-recovery
/// re-dispatch is exactly-once via the content-addressed stage + the run-scoped
/// idempotency token), declares `(tool_id, tool_version)` in its `tool_contract`
/// (the broker's `precheck` re-verifies it against `warrant.tool_grants` at
/// dispatch — SN-8), carries ONE Data edge to its proposing turn (durable
/// lineage; the ready-set releases it when the turn commits), and keeps its
/// `config_subset` EMPTY — the PR-2d-2 args contract: the model-proposed args
/// travel OUT-OF-BAND (`WorkItem.tool_args`, re-derived at lease time from the
/// committed turn output), so the observation's identity never moves with the
/// args. Everything here is a pure function of `(model_id, tool, turn,
/// instance_id, turn_mote_id)` — recovery re-derives the SAME Mote from the
/// frozen fact, which is why no "materialized" marker needs to be durable.
#[must_use]
pub(crate) fn build_react_tool(
    model_id: &ModelId,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    call_index: u32,
    turn_mote_id: MoteId,
) -> Mote {
    let id_bytes = react_tool_id_material(instance_id, turn, call_index);

    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_id.clone(), tool_version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        // EMPTY — the out-of-band args contract (see the fn doc).
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes(id_bytes),
        GraphPosition(id_bytes.to_vec()),
        std::iter::once(ParentRef {
            parent_id: turn_mote_id,
            edge: EdgeMeta::data(),
        })
        .collect::<SmallVec<[ParentRef; 4]>>(),
    )
}

// ===========================================================================
// RC4c-2b — the live LLM RERANK turn builder.
//
// A rerank turn is a coordinator-materialized, OFF-DAG, OFF-BUDGET Mote that
// reorders a RAG retrieval's candidate passages by relevance (a permutation).
// It mirrors the ReAct-turn SHAPE (ROND, edge-free, worker-sampled →
// coordinator-decoded) but is a SINGLE bounded turn per retrieval (no successor
// chain, no `max_turns`/`max_tool_calls` consumption). Its identity namespace is
// cryptographically distinct (`b"kx-rerank-turn"`), salted by the content-addressed
// base results + query so each distinct retrieval reranks under a distinct identity.
// ===========================================================================

/// The 32-byte identity material for a live LLM RERANK turn (RC4c-2b):
/// `blake3(b"kx-rerank-turn" ‖ instance_id ‖ base_results_ref ‖ query_ref)`.
/// Cryptographically distinct from every react/agentic/shaper namespace (different
/// domain tag). Salted by the (content-addressed) base results + query, so each
/// distinct retrieval reranks under a distinct identity and two byte-identical
/// retrievals in one run dedup to ONE rerank (idempotent — identical inputs yield the
/// identical order). Both salts are recorded on the `ReRankRound` anchor, so recovery
/// re-derives this material byte-identically (R49).
#[must_use]
pub(crate) fn rerank_turn_id_material(
    instance_id: &[u8; INSTANCE_ID_LEN],
    base_results_ref: &ContentRef,
    query_ref: &ContentRef,
) -> [u8; 32] {
    let mut material = b"kx-rerank-turn".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(base_results_ref.as_bytes());
    material.extend_from_slice(query_ref.as_bytes());
    *ContentRef::of(&material).as_bytes()
}

/// Build a live LLM RERANK turn `Mote` (RC4c-2b) — the coordinator-materialized,
/// off-DAG Mote that reorders a RAG retrieval's candidate passages by relevance.
///
/// Shape mirrors [`build_react_turn`]: ROND (the model samples a permutation; the
/// COMMITTED output is the served fact, re-decoded — never re-sampled — on replay),
/// **EDGE-FREE** (empty parents — the candidates travel via `config_subset` refs, so
/// the turn never moves the canonical digest via `encode_state` edges), NOT a topology
/// shaper, no `tool_contract` (a rerank fires nothing; it PROPOSES an order the
/// coordinator settle enforces). Three identity-bearing config values: the
/// [`RERANK_TURN_KEY`] routing marker (value = `instance_id`, mirroring
/// [`REACT_TURN_KEY`]), the [`RERANK_QUERY_KEY`] query ref, and the
/// [`RERANK_CANDIDATES_KEY`] base-results ref. The worker's `run_rerank_turn` resolves
/// both refs, renders the shared rerank prompt, arms `Grammar::Permutation(n)`
/// OFF-MoteDef (digest-neutral, like the RC2 react grammar), and commits the RAW
/// permutation; the coordinator's `settle_rerank_rounds` is the sole
/// `parse_permutation` authority.
///
/// `max_output_tokens` is the warrant's output ceiling (identity-bearing, mirrors
/// [`build_react_turn`]); the worker additionally bounds the decode by
/// `kx_context_assembler::rerank_output_cap(n)` at dispatch.
#[must_use]
pub(crate) fn build_rerank_turn(
    model_id: &ModelId,
    instance_id: &[u8; INSTANCE_ID_LEN],
    base_results_ref: &ContentRef,
    query_ref: &ContentRef,
    max_output_tokens: u32,
) -> Mote {
    let id_bytes = rerank_turn_id_material(instance_id, base_results_ref, query_ref);

    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(RERANK_TURN_KEY.to_string()),
        ConfigVal(instance_id.to_vec()),
    );
    config_subset.insert(
        ConfigKey(RERANK_QUERY_KEY.to_string()),
        ConfigVal(query_ref.as_bytes().to_vec()),
    );
    config_subset.insert(
        ConfigKey(RERANK_CANDIDATES_KEY.to_string()),
        ConfigVal(base_results_ref.as_bytes().to_vec()),
    );
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        // No tool_contract: the rerank PROPOSES an order; it fires nothing.
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams {
            max_output_tokens,
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

// ===========================================================================
// PR-9b-2b — the SALT-2 builders for a DETERMINISTIC-AGENTIC STEP.
//
// A deterministic-agentic step is a frozen-DAG MODEL step that becomes ready
// MID-RUN and runs its OWN bounded reason→tool→observe loop (vs the run-level
// react chain swapped in at submit). Its turn/observation Motes are salted by an
// ADDITIONAL 32-byte `step_salt` (= the launch step's `MoteId`) on top of the run
// `instance_id`, so multiple agentic steps in one run — and the run-level react
// chain — never collide on `(instance_id, turn)`. The domain tags are DISTINCT
// from the salt-1 namespaces (`b"kx-agentic-*"` vs `b"kx-react-*"`), and the
// byte-frozen salt-1 builders above are deliberately UNTOUCHED (their cross-impl
// goldens stay pinned). A NEW golden pins the salt-2 derivation below.
// ===========================================================================

/// The salt-2 identity material for an agentic-step turn:
/// `blake3(b"kx-agentic-turn" ‖ instance_id ‖ step_salt ‖ turn.to_le_bytes())`.
/// Deterministic + distinct per `(run, step, turn)` and cryptographically
/// distinct from EVERY salt-1 namespace (different domain tag) — so an agentic
/// step's chain can never dedup-collide with the run-level react chain.
#[must_use]
pub(crate) fn react_turn_id_material2(
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: &[u8; 32],
    turn: u32,
) -> [u8; 32] {
    let mut material = b"kx-agentic-turn".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(step_salt);
    material.extend_from_slice(&turn.to_le_bytes());
    *ContentRef::of(&material).as_bytes()
}

/// Re-derive an agentic-step turn `Mote` — the salt-2 twin of [`build_react_turn`].
/// Identical SHAPE (ROND, edge-free, instruction in `config_subset[PROMPT_KEY]`,
/// greedy at `max_output_tokens`) EXCEPT: (a) the id is salt-2 derived, and
/// (b) the [`REACT_TURN_KEY`] routing marker carries `instance_id ‖ step_salt`
/// (48 bytes) so the coordinator's `resolve_parent_context` reconstructs the
/// compound `(instance_id, step_salt)` chain key (a 16-byte marker = run-level).
#[must_use]
pub(crate) fn build_agentic_turn(
    model_id: &ModelId,
    instruction: &str,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: &[u8; 32],
    max_output_tokens: u32,
) -> Mote {
    let id_bytes = react_turn_id_material2(instance_id, step_salt, turn);

    let mut marker = instance_id.to_vec();
    marker.extend_from_slice(step_salt);

    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(instruction.as_bytes().to_vec()),
    );
    config_subset.insert(ConfigKey(REACT_TURN_KEY.to_string()), ConfigVal(marker));
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
        is_topology_shaper: false,
        inference_params: InferenceParams {
            max_output_tokens,
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

/// The salt-2 identity material for an agentic-step OBSERVATION:
/// `blake3(b"kx-agentic-tool" ‖ instance_id ‖ step_salt ‖ turn.to_le_bytes() ‖
/// [call_index if >0])`. T-MULTI-ELEMENT-TOOLCALLS: `call_index` disambiguates the N
/// observations of a `ToolBatch` turn, appended ONLY when `> 0` so a single-call
/// agentic step (index 0) is byte-identical to every pre-v13 chain (`AGENTIC_TOOL0_GOLDEN`).
#[must_use]
pub(crate) fn react_tool_id_material2(
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: &[u8; 32],
    turn: u32,
    call_index: u32,
) -> [u8; 32] {
    let mut material = b"kx-agentic-tool".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(step_salt);
    material.extend_from_slice(&turn.to_le_bytes());
    if call_index > 0 {
        material.extend_from_slice(&call_index.to_le_bytes());
    }
    *ContentRef::of(&material).as_bytes()
}

/// Re-derive an agentic-step OBSERVATION `Mote` — the salt-2 twin of
/// [`build_react_tool`]. Identical SHAPE (WM `StageThenCommit`, one Data edge to
/// its proposing turn, EMPTY config = the out-of-band args contract, declared
/// `(tool_id, tool_version)` in `tool_contract`) EXCEPT the id is salt-2 derived
/// and the parent is the agentic turn.
#[must_use]
#[allow(clippy::too_many_arguments)] // identity inputs: model/tool/turn/instance/step_salt/call_index/parent
pub(crate) fn build_agentic_tool(
    model_id: &ModelId,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: &[u8; 32],
    call_index: u32,
    turn_mote_id: MoteId,
) -> Mote {
    let id_bytes = react_tool_id_material2(instance_id, step_salt, turn, call_index);

    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_id.clone(), tool_version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(id_bytes),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes(id_bytes),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes(id_bytes),
        GraphPosition(id_bytes.to_vec()),
        std::iter::once(ParentRef {
            parent_id: turn_mote_id,
            edge: EdgeMeta::data(),
        })
        .collect::<SmallVec<[ParentRef; 4]>>(),
    )
}

/// Re-derive a chain TURN `Mote` keyed by the chain's `step_salt` (PR-9b-2b): the
/// run-level react chain (`None`) uses the salt-1 [`build_react_turn`]; an agentic
/// step's private chain (`Some(launch MoteId)`) uses the salt-2 [`build_agentic_turn`].
/// One dispatch point so the coordinator's settle/recover/advance code is chain-shape
/// agnostic and the two namespaces can never be confused.
#[must_use]
pub(crate) fn build_chain_turn(
    model_id: &ModelId,
    instruction: &str,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
    max_output_tokens: u32,
) -> Mote {
    match step_salt {
        Some(salt) => build_agentic_turn(
            model_id,
            instruction,
            turn,
            instance_id,
            &salt,
            max_output_tokens,
        ),
        None => build_react_turn(model_id, instruction, turn, instance_id, max_output_tokens),
    }
}

/// Re-derive a chain OBSERVATION `Mote` keyed by the chain's `step_salt` (PR-9b-2b):
/// the salt-1 [`build_react_tool`] for the run-level chain, the salt-2
/// [`build_agentic_tool`] for an agentic step's chain. The twin of [`build_chain_turn`].
#[must_use]
#[allow(clippy::too_many_arguments)] // identity inputs: model/tool/turn/instance/step_salt/call_index/parent
pub(crate) fn build_chain_tool(
    model_id: &ModelId,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
    call_index: u32,
    turn_mote_id: MoteId,
) -> Mote {
    match step_salt {
        Some(salt) => build_agentic_tool(
            model_id,
            tool_id,
            tool_version,
            turn,
            instance_id,
            &salt,
            call_index,
            turn_mote_id,
        ),
        None => build_react_tool(
            model_id,
            tool_id,
            tool_version,
            turn,
            instance_id,
            call_index,
            turn_mote_id,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8; 32]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// The CROSS-IMPL frozen golden (R49): the EXACT salted turn-0 `MoteId` the
    /// harness `react_turn_salted` derives for the same inputs — the same hex is
    /// pinned in `kx-model-harness::workflows::react_identity_tests`, so a drift
    /// on EITHER copy fails CI. Inputs: model `kx-test:q8:deadbeef`, instruction
    /// "list the files", turn 0, salt `[0x4d; 16]`, max_output_tokens 64.
    const SALTED_TURN0_GOLDEN: &str =
        "f2e465451f434a861090109d336c39a8307e5d539963fd48b3470df84458a5cb";

    #[test]
    fn salted_turn_matches_the_harness_golden() {
        let model = ModelId("kx-test:q8:deadbeef".to_string());
        let turn = build_react_turn(&model, "list the files", 0, &[0x4d; 16], 64);
        assert_eq!(hex(turn.id.as_bytes()), SALTED_TURN0_GOLDEN);
        assert!(turn.parents.is_empty(), "a react turn MUST be edge-free");
        assert!(!turn.def.is_topology_shaper);
        assert_eq!(turn.def.nd_class, NdClass::ReadOnlyNondet);
        // The routing marker carries the salt (mechanism A).
        assert_eq!(
            turn.def
                .config_subset
                .get(&ConfigKey(REACT_TURN_KEY.to_string()))
                .map(|v| v.0.clone()),
            Some(vec![0x4d; 16])
        );
    }

    /// The CROSS-IMPL frozen golden for the OBSERVATION builder (PR-2d-2): the
    /// EXACT salted `MoteId` the harness `react_tool_mote_salted` derives for
    /// the same inputs — the same hex is pinned in
    /// `kx-model-harness::workflows::react_identity_tests`, so a drift on
    /// EITHER copy fails CI. Inputs: model `kx-test:q8:deadbeef`, tool
    /// `mcp-echo@1`, turn 0, salt `[0x4d; 16]`, turn_mote_id = the salted
    /// turn-0 golden.
    const SALTED_TOOL0_GOLDEN: &str =
        "0797b93286b999344db0ba9a458a83105c6a6b55c29760e510311ae45ff68048";

    #[test]
    fn salted_observation_matches_the_harness_golden() {
        let model = ModelId("kx-test:q8:deadbeef".to_string());
        let salt = [0x4d_u8; 16];
        let turn = build_react_turn(&model, "list the files", 0, &salt, 64);
        let obs = build_react_tool(
            &model,
            &ToolName("mcp-echo".to_string()),
            &ToolVersion("1".to_string()),
            0,
            &salt,
            0, // call_index 0 (single-call turn) ⇒ byte-identical to pre-v13
            turn.id,
        );
        assert_eq!(hex(obs.id.as_bytes()), SALTED_TOOL0_GOLDEN);
        // FULL-Mote contract (not just the id): one Data edge to the turn,
        // empty config (out-of-band args), WM + StageThenCommit, the declared
        // tool contract — byte-for-byte the harness observation.
        assert_eq!(obs.parents.len(), 1);
        assert_eq!(obs.parents[0].parent_id, turn.id);
        assert!(obs.def.config_subset.is_empty());
        assert_eq!(obs.def.nd_class, NdClass::WorldMutating);
        assert_eq!(obs.def.effect_pattern, EffectPattern::StageThenCommit);
        assert_eq!(
            obs.def
                .tool_contract
                .get(&ToolName("mcp-echo".to_string()))
                .map(|v| v.0.clone()),
            Some("1".to_string())
        );
    }

    #[test]
    fn react_tool_id_is_deterministic_and_run_isolated() {
        let a = react_tool_id_material(&[1; 16], 0, 0);
        assert_eq!(a, react_tool_id_material(&[1; 16], 0, 0));
        assert_ne!(a, react_tool_id_material(&[1; 16], 1, 0));
        assert_ne!(a, react_tool_id_material(&[2; 16], 0, 0));
        // Distinct from the TURN namespace at the same coordinates.
        assert_ne!(a, react_turn_id_material(&[1; 16], 0));
        // T-MULTI-ELEMENT-TOOLCALLS: call_index 0 is byte-identical to the no-index
        // material (the "append only if >0" rule), while >0 is distinct + deterministic.
        assert_eq!(a, react_tool_id_material(&[1; 16], 0, 0));
        let c1 = react_tool_id_material(&[1; 16], 0, 1);
        assert_ne!(a, c1, "call_index 1 must differ from call_index 0");
        assert_eq!(c1, react_tool_id_material(&[1; 16], 0, 1), "deterministic");
        assert_ne!(c1, react_tool_id_material(&[1; 16], 0, 2));
        // A multi-call turn's index-1 obs must NOT collide with the NEXT turn's index-0.
        assert_ne!(c1, react_tool_id_material(&[1; 16], 1, 0));
    }

    #[test]
    fn react_turn_id_is_deterministic_and_run_isolated() {
        let a = react_turn_id_material(&[1; 16], 0);
        // Deterministic.
        assert_eq!(a, react_turn_id_material(&[1; 16], 0));
        // Distinct per turn.
        assert_ne!(a, react_turn_id_material(&[1; 16], 1));
        // Distinct per RUN (the shared-journal collision the salt closes).
        assert_ne!(a, react_turn_id_material(&[2; 16], 0));
        // The built Mote is a pure function of its inputs.
        let model = ModelId("qwen".into());
        let x = build_react_turn(&model, "p", 1, &[1; 16], 64);
        let y = build_react_turn(&model, "p", 1, &[1; 16], 64);
        assert_eq!(x.id, y.id);
        assert_ne!(build_react_turn(&model, "p2", 1, &[1; 16], 64).id, x.id);
        assert_ne!(build_react_turn(&model, "p", 2, &[1; 16], 64).id, x.id);
        assert_ne!(build_react_turn(&model, "p", 1, &[1; 16], 65).id, x.id);
    }

    // -----------------------------------------------------------------------
    // RC4c-2b — live LLM rerank turn builder.
    // -----------------------------------------------------------------------

    /// The frozen golden `MoteId` for a rerank turn built from fixed inputs — a
    /// regression pin (the rerank turn is serve-only, so there is NO harness twin;
    /// this guards against accidental identity drift). Inputs: model
    /// `kx-test:q8:deadbeef`, salt `[0x4d; 16]`, `base_results_ref =
    /// ContentRef::of(b"base")`, `query_ref = ContentRef::of(b"query")`,
    /// `max_output_tokens` 512.
    const RERANK_TURN0_GOLDEN: &str =
        "c2bc06a8f71290b6325a3214a68f44092f3aec25bb8c8ed6e29d5949bfac7565";

    #[test]
    fn rerank_turn_is_deterministic_edge_free_and_isolated() {
        let model = ModelId("kx-test:q8:deadbeef".to_string());
        let salt = [0x4d_u8; 16];
        let base = ContentRef::of(b"base");
        let query = ContentRef::of(b"query");
        let turn = build_rerank_turn(&model, &salt, &base, &query, 512);
        // Frozen identity.
        assert_eq!(hex(turn.id.as_bytes()), RERANK_TURN0_GOLDEN);
        // Off-DAG contract: edge-free, ROND, not a shaper, no tool_contract (fires nothing).
        assert!(turn.parents.is_empty(), "a rerank turn MUST be edge-free");
        assert!(!turn.def.is_topology_shaper);
        assert!(turn.def.tool_contract.is_empty());
        assert_eq!(turn.def.nd_class, NdClass::ReadOnlyNondet);
        // The routing marker + the two candidate/query refs.
        assert_eq!(
            turn.def
                .config_subset
                .get(&ConfigKey(RERANK_TURN_KEY.to_string()))
                .map(|v| v.0.clone()),
            Some(salt.to_vec())
        );
        assert_eq!(
            turn.def
                .config_subset
                .get(&ConfigKey(RERANK_QUERY_KEY.to_string()))
                .map(|v| v.0.clone()),
            Some(query.as_bytes().to_vec())
        );
        assert_eq!(
            turn.def
                .config_subset
                .get(&ConfigKey(RERANK_CANDIDATES_KEY.to_string()))
                .map(|v| v.0.clone()),
            Some(base.as_bytes().to_vec())
        );
        // Deterministic + salted by (run, base_results, query).
        assert_eq!(
            build_rerank_turn(&model, &salt, &base, &query, 512).id,
            turn.id
        );
        assert_ne!(
            build_rerank_turn(&model, &[0x01; 16], &base, &query, 512).id,
            turn.id,
            "run-isolated"
        );
        assert_ne!(
            build_rerank_turn(&model, &salt, &ContentRef::of(b"other"), &query, 512).id,
            turn.id,
            "base-results-salted"
        );
        assert_ne!(
            build_rerank_turn(&model, &salt, &base, &ContentRef::of(b"other"), 512).id,
            turn.id,
            "query-salted"
        );
        // Cryptographically distinct namespace from the react turn at the same run-salt.
        assert_ne!(
            rerank_turn_id_material(&salt, &base, &query),
            react_turn_id_material(&salt, 0)
        );
    }

    // -----------------------------------------------------------------------
    // PR-9b-2b — salt-2 (deterministic-agentic step) builder goldens.
    // -----------------------------------------------------------------------

    /// The frozen golden for the salt-2 turn-0 `MoteId`. Inputs: model
    /// `kx-test:q8:deadbeef`, instruction "list the files", turn 0, salt
    /// `[0x4d; 16]`, step_salt `[0x9a; 32]`, max_output_tokens 64. Coordinator-
    /// local (no harness twin — agentic steps are a serve-only construct); pins
    /// the salt-2 derivation so a drift in the domain tag / material order fails
    /// CI. MUST differ from `SALTED_TURN0_GOLDEN` (distinct domain namespaces).
    const AGENTIC_TURN0_GOLDEN: &str =
        "8bed4369abcfd6da5f334ea1e2e28358773a83596d58d2e16ea12a84b0312dc2";

    #[test]
    fn agentic_turn_matches_its_golden_and_is_namespace_distinct() {
        let model = ModelId("kx-test:q8:deadbeef".to_string());
        let salt = [0x4d_u8; 16];
        let step_salt = [0x9a_u8; 32];
        let turn = build_agentic_turn(&model, "list the files", 0, &salt, &step_salt, 64);
        // Property contract (the golden hex is bootstrapped by `just`-running this
        // once; pinned below). Shape mirrors a salt-1 react turn.
        assert!(turn.parents.is_empty(), "an agentic turn MUST be edge-free");
        assert!(!turn.def.is_topology_shaper);
        assert_eq!(turn.def.nd_class, NdClass::ReadOnlyNondet);
        // The marker carries instance_id ‖ step_salt (48 bytes).
        let marker = turn
            .def
            .config_subset
            .get(&ConfigKey(REACT_TURN_KEY.to_string()))
            .map(|v| v.0.clone())
            .expect("marker present");
        assert_eq!(marker.len(), INSTANCE_ID_LEN + 32);
        assert_eq!(&marker[..INSTANCE_ID_LEN], &salt[..]);
        assert_eq!(&marker[INSTANCE_ID_LEN..], &step_salt[..]);
        // CRYPTOGRAPHICALLY distinct from the salt-1 react turn at the same coords.
        let react = build_react_turn(&model, "list the files", 0, &salt, 64);
        assert_ne!(turn.id, react.id, "salt-2 must not collide with salt-1");
        assert_eq!(hex(turn.id.as_bytes()), AGENTIC_TURN0_GOLDEN);
    }

    /// The frozen golden for the salt-2 observation-0 `MoteId`. Same inputs +
    /// tool `mcp-echo@1`, parent = the salt-2 turn-0.
    const AGENTIC_TOOL0_GOLDEN: &str =
        "95b763ae1384952b004b5e16d0ee47ce02c08b403b18ebc0a629e65db91b8b98";

    #[test]
    fn agentic_tool_matches_its_golden_and_is_namespace_distinct() {
        let model = ModelId("kx-test:q8:deadbeef".to_string());
        let salt = [0x4d_u8; 16];
        let step_salt = [0x9a_u8; 32];
        let turn = build_agentic_turn(&model, "list the files", 0, &salt, &step_salt, 64);
        let obs = build_agentic_tool(
            &model,
            &ToolName("mcp-echo".to_string()),
            &ToolVersion("1".to_string()),
            0,
            &salt,
            &step_salt,
            0, // call_index 0 (single-call turn) ⇒ byte-identical to pre-v13
            turn.id,
        );
        // Shape: one Data edge to the turn, empty config (out-of-band args), WM +
        // StageThenCommit, the declared tool contract.
        assert_eq!(obs.parents.len(), 1);
        assert_eq!(obs.parents[0].parent_id, turn.id);
        assert!(obs.def.config_subset.is_empty());
        assert_eq!(obs.def.nd_class, NdClass::WorldMutating);
        assert_eq!(obs.def.effect_pattern, EffectPattern::StageThenCommit);
        // Distinct from the salt-1 observation at the same coords.
        let react_obs = build_react_tool(
            &model,
            &ToolName("mcp-echo".to_string()),
            &ToolVersion("1".to_string()),
            0,
            &salt,
            0,
            turn.id,
        );
        assert_ne!(obs.id, react_obs.id);
        assert_eq!(hex(obs.id.as_bytes()), AGENTIC_TOOL0_GOLDEN);
    }

    /// T-MULTI-ELEMENT-TOOLCALLS — the CROSS-IMPL golden for a multi-call turn's
    /// SECOND observation (`call_index = 1`). Same inputs as `SALTED_TOOL0_GOLDEN`
    /// (model `kx-test:q8:deadbeef`, tool `mcp-echo@1`, turn 0, salt `[0x4d; 16]`,
    /// turn_mote_id = the salted turn-0 golden) EXCEPT `call_index = 1`. The SAME
    /// hex is pinned in the harness `react_identity_tests`, so a drift on EITHER copy
    /// of the call-indexed material fails CI — the R49 equivalence the live drain and
    /// the harness oracle depend on. MUST differ from `SALTED_TOOL0_GOLDEN`.
    const SALTED_TOOL1_GOLDEN: &str =
        "6288b25bc8514c933719bfafddb4b065f02ed8a4ff54ff7ab4ca059d180d62b3";

    #[test]
    fn salted_observation_call1_matches_the_harness_golden() {
        let model = ModelId("kx-test:q8:deadbeef".to_string());
        let salt = [0x4d_u8; 16];
        let turn = build_react_turn(&model, "list the files", 0, &salt, 64);
        // call_index 0 reproduces the EXISTING golden byte-for-byte (the
        // "append only if >0" byte-preservation rule).
        let obs0 = build_react_tool(
            &model,
            &ToolName("mcp-echo".to_string()),
            &ToolVersion("1".to_string()),
            0,
            &salt,
            0,
            turn.id,
        );
        assert_eq!(hex(obs0.id.as_bytes()), SALTED_TOOL0_GOLDEN);
        // call_index 1 is a DISTINCT, deterministic id pinned by the cross-impl golden.
        let obs1 = build_react_tool(
            &model,
            &ToolName("mcp-echo".to_string()),
            &ToolVersion("1".to_string()),
            0,
            &salt,
            1,
            turn.id,
        );
        assert_ne!(
            obs0.id, obs1.id,
            "two calls at one turn must have distinct observation ids"
        );
        assert_eq!(hex(obs1.id.as_bytes()), SALTED_TOOL1_GOLDEN);
        // Both still declare the SAME tool contract + parent (only the id differs).
        assert_eq!(obs1.parents[0].parent_id, turn.id);
        assert!(obs1.def.config_subset.is_empty());
    }

    #[test]
    fn render_reprompt_is_deterministic_and_embeds_the_reason() {
        let a = render_reprompt("list the files", "tool `x@1` is not granted to this run");
        // PURE: same inputs → byte-identical output (the recovery/replay law).
        assert_eq!(
            a,
            render_reprompt("list the files", "tool `x@1` is not granted to this run")
        );
        // CROSS-IMPL pin (R49): the EXACT bytes the harness twin
        // `kx_model_harness::react_reason::render_reprompt` must also produce — pinned
        // as a literal on BOTH sides (the `SALTED_TURN0_GOLDEN` convention) so a drift
        // on either copy fails CI and a re-prompted turn's MoteId stays identical
        // across the dep wall. Keep this literal in sync with the harness test
        // `reprompt_text_matches_the_coordinator`.
        assert_eq!(
            render_reprompt("list the files", "tool `x@1` is not granted to this run"),
            "list the files\n\n[Your previous tool call was REJECTED: \
             tool `x@1` is not granted to this run\nCorrect it — call a tool you \
             were granted with arguments that match its schema, or answer the \
             question directly if you cannot.]"
        );
        // A different reason → a different re-prompt (the model sees what changed).
        assert_ne!(
            a,
            render_reprompt("list the files", "args do not match schema")
        );
    }

    #[test]
    fn render_settle_nudge_is_deterministic_and_keeps_the_base() {
        let a = render_settle_nudge("list the files");
        // PURE: same input → byte-identical output (the recovery/replay law).
        assert_eq!(a, render_settle_nudge("list the files"));
        assert!(
            a.starts_with("list the files"),
            "the base instruction leads"
        );
        assert!(
            a.contains("Do NOT call another tool"),
            "carries the stop-calling-tools steer"
        );
        assert!(
            a.contains("give your FINAL answer"),
            "carries the answer-now steer"
        );
        // A different base → a different nudge.
        assert_ne!(a, render_settle_nudge("summarize the doc"));
        // The nudge is strictly the base + a CONSTANT suffix (no reason), so it is
        // shorter than the worst-case A2 re-prompt for the same base.
        let reprompt = render_reprompt("list the files", &"x".repeat(512));
        assert!(
            a.len() < reprompt.len(),
            "the nudge has no unbounded reason"
        );
    }

    #[test]
    fn bounded_reason_truncates_at_a_char_boundary_total() {
        // Under the cap: identity.
        let short = "short reason".to_string();
        assert_eq!(bounded_reason(short.clone()), short);
        // Over the cap (multi-byte chars): truncated to exactly the cap, never
        // panics on a char boundary, and is idempotent.
        let long: String = "é".repeat(kx_journal::MAX_REJECTED_REASON_LEN + 50);
        let bounded = bounded_reason(long);
        assert_eq!(bounded.chars().count(), kx_journal::MAX_REJECTED_REASON_LEN);
        assert_eq!(bounded_reason(bounded.clone()), bounded, "idempotent");
    }

    #[test]
    fn agentic_ids_are_deterministic_and_step_isolated() {
        let a = react_turn_id_material2(&[1; 16], &[2; 32], 0);
        assert_eq!(a, react_turn_id_material2(&[1; 16], &[2; 32], 0));
        assert_ne!(a, react_turn_id_material2(&[1; 16], &[2; 32], 1)); // per turn
        assert_ne!(a, react_turn_id_material2(&[1; 16], &[3; 32], 0)); // per step
        assert_ne!(a, react_turn_id_material2(&[9; 16], &[2; 32], 0)); // per run
                                                                       // Distinct from the agentic-tool namespace at the same coords.
        assert_ne!(a, react_tool_id_material2(&[1; 16], &[2; 32], 0, 0));
        // Distinct from BOTH salt-1 namespaces.
        assert_ne!(a, react_turn_id_material(&[1; 16], 0));
        assert_ne!(a, react_tool_id_material(&[1; 16], 0, 0));
        // T-MULTI-ELEMENT-TOOLCALLS: salt-2 call_index 0 is byte-identical to the
        // no-index material, while >0 is distinct + deterministic.
        let t0 = react_tool_id_material2(&[1; 16], &[2; 32], 0, 0);
        assert_eq!(t0, react_tool_id_material2(&[1; 16], &[2; 32], 0, 0));
        let t1 = react_tool_id_material2(&[1; 16], &[2; 32], 0, 1);
        assert_ne!(t0, t1, "agentic call_index 1 must differ from 0");
        assert_ne!(t1, react_tool_id_material2(&[1; 16], &[2; 32], 1, 0));
    }
}
