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
    ToolVersion, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY, REACT_TURN_KEY,
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
pub(crate) const REACT_MAX_TURNS: u32 = 8;
/// The default per-run tool-call (observation) budget (PR-2d-2). Deliberately
/// `< REACT_MAX_TURNS`: the harness `ReactBudget` docs pin that a useful loop
/// leaves at least one turn to READ the last observation and answer — an 8/8
/// budget is degenerate (the harness default predates live tool firing). Seed
/// caps are validated `0 < max_tool_calls < max_turns ≤ 8`; chains anchored
/// under the old 8/8 default keep their recorded caps (durable per-anchor).
pub(crate) const REACT_DEFAULT_MAX_TOOL_CALLS: u32 = 6;

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
/// Mote that fires the model's frozen `Tool` decision at `turn`):
/// `blake3(b"kx-react-tool" ‖ instance_id ‖ turn.to_le_bytes())`. The TOOL
/// identity is deliberately NOT in the material — it enters the `MoteId` via
/// `tool_contract` (def-hash), exactly like the harness
/// `kx_model_harness::workflows::react_tool_mote_salted`.
#[must_use]
pub(crate) fn react_tool_id_material(instance_id: &[u8; INSTANCE_ID_LEN], turn: u32) -> [u8; 32] {
    let mut material = b"kx-react-tool".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(&turn.to_le_bytes());
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
    turn_mote_id: MoteId,
) -> Mote {
    let id_bytes = react_tool_id_material(instance_id, turn);

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
/// `blake3(b"kx-agentic-tool" ‖ instance_id ‖ step_salt ‖ turn.to_le_bytes())`.
#[must_use]
pub(crate) fn react_tool_id_material2(
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: &[u8; 32],
    turn: u32,
) -> [u8; 32] {
    let mut material = b"kx-agentic-tool".to_vec();
    material.extend_from_slice(instance_id);
    material.extend_from_slice(step_salt);
    material.extend_from_slice(&turn.to_le_bytes());
    *ContentRef::of(&material).as_bytes()
}

/// Re-derive an agentic-step OBSERVATION `Mote` — the salt-2 twin of
/// [`build_react_tool`]. Identical SHAPE (WM `StageThenCommit`, one Data edge to
/// its proposing turn, EMPTY config = the out-of-band args contract, declared
/// `(tool_id, tool_version)` in `tool_contract`) EXCEPT the id is salt-2 derived
/// and the parent is the agentic turn.
#[must_use]
pub(crate) fn build_agentic_tool(
    model_id: &ModelId,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: &[u8; 32],
    turn_mote_id: MoteId,
) -> Mote {
    let id_bytes = react_tool_id_material2(instance_id, step_salt, turn);

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
pub(crate) fn build_chain_tool(
    model_id: &ModelId,
    tool_id: &ToolName,
    tool_version: &ToolVersion,
    turn: u32,
    instance_id: &[u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
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
            turn_mote_id,
        ),
        None => build_react_tool(
            model_id,
            tool_id,
            tool_version,
            turn,
            instance_id,
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
        let a = react_tool_id_material(&[1; 16], 0);
        assert_eq!(a, react_tool_id_material(&[1; 16], 0));
        assert_ne!(a, react_tool_id_material(&[1; 16], 1));
        assert_ne!(a, react_tool_id_material(&[2; 16], 0));
        // Distinct from the TURN namespace at the same coordinates.
        assert_ne!(a, react_turn_id_material(&[1; 16], 0));
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
            turn.id,
        );
        assert_ne!(obs.id, react_obs.id);
        assert_eq!(hex(obs.id.as_bytes()), AGENTIC_TOOL0_GOLDEN);
    }

    #[test]
    fn agentic_ids_are_deterministic_and_step_isolated() {
        let a = react_turn_id_material2(&[1; 16], &[2; 32], 0);
        assert_eq!(a, react_turn_id_material2(&[1; 16], &[2; 32], 0));
        assert_ne!(a, react_turn_id_material2(&[1; 16], &[2; 32], 1)); // per turn
        assert_ne!(a, react_turn_id_material2(&[1; 16], &[3; 32], 0)); // per step
        assert_ne!(a, react_turn_id_material2(&[9; 16], &[2; 32], 0)); // per run
                                                                       // Distinct from the agentic-tool namespace at the same coords.
        assert_ne!(a, react_tool_id_material2(&[1; 16], &[2; 32], 0));
        // Distinct from BOTH salt-1 namespaces.
        assert_ne!(a, react_turn_id_material(&[1; 16], 0));
        assert_ne!(a, react_tool_id_material(&[1; 16], 0));
    }
}
