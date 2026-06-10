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
    ConfigKey, ConfigVal, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef,
    ModelId, Mote, MoteDef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY,
    REACT_TURN_KEY,
};
use smallvec::SmallVec;

/// The durable per-run ReAct budget caps recorded on the turn-0 anchor — a
/// byte-mirror of the harness `ReactBudget::default()` (8 turns / 8 tool calls),
/// so the live coordinator and the harness drive identical-length chains (the
/// cross-impl equivalence pin, R49). Recorded DURABLY at anchor time so a
/// recovered coordinator enforces the budget the run was admitted under, never a
/// default that drifted across binary versions (red-team BLOCKER #4).
pub(crate) const REACT_MAX_TURNS: u32 = 8;
/// See [`REACT_MAX_TURNS`].
pub(crate) const REACT_MAX_TOOL_CALLS: u32 = 8;

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
        // No tool_contract: the turn PROPOSES; firing is PR-2d-2's tool Mote.
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
}
