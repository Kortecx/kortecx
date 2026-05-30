//! Inline unit tests for kx-mote. Extracted per Rule 3 with bodies
//! unchanged.

use std::collections::BTreeMap;

use smallvec::SmallVec;

use super::*;

fn sample_def() -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1u8; 32]),
        model_id: ModelId("test-model:v1:q4".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

#[test]
fn nd_class_u8_repr_is_stable() {
    assert_eq!(NdClass::Pure.as_u8(), 0);
    assert_eq!(NdClass::ReadOnlyNondet.as_u8(), 1);
    assert_eq!(NdClass::WorldMutating.as_u8(), 2);
}

#[test]
fn edge_kind_u8_repr_is_stable() {
    assert_eq!(EdgeKind::Data.as_u8(), 0);
    assert_eq!(EdgeKind::Control.as_u8(), 1);
}

#[test]
fn edge_meta_constructors_uphold_invariants() {
    assert_eq!(
        EdgeMeta::data(),
        EdgeMeta {
            kind: EdgeKind::Data,
            non_cascade: false
        }
    );
    assert_eq!(
        EdgeMeta::control(),
        EdgeMeta {
            kind: EdgeKind::Control,
            non_cascade: false
        }
    );
    assert_eq!(
        EdgeMeta::control_non_cascading(),
        EdgeMeta {
            kind: EdgeKind::Control,
            non_cascade: true
        }
    );
}

#[test]
fn mote_def_hash_is_deterministic_across_calls() {
    let def = sample_def();
    let h1 = def.hash();
    let h2 = def.hash();
    assert_eq!(h1, h2);
}

#[test]
fn schema_version_is_v5() {
    assert_eq!(MOTE_DEF_SCHEMA_VERSION, 5);
    assert_eq!(sample_def().schema_version, 5);
}

#[test]
fn derive_mote_id_is_pure() {
    let def_hash = MoteDefHash::from_bytes([7u8; 32]);
    let input = InputDataId::from_bytes([8u8; 32]);
    let pos = GraphPosition(vec![9, 9, 9]);
    let a = derive_mote_id(&def_hash, &input, &pos);
    let b = derive_mote_id(&def_hash, &input, &pos);
    assert_eq!(a, b);
}

#[test]
fn derive_mote_id_differs_on_any_component_change() {
    let def_hash = MoteDefHash::from_bytes([7u8; 32]);
    let input = InputDataId::from_bytes([8u8; 32]);
    let pos = GraphPosition(vec![9, 9, 9]);
    let base = derive_mote_id(&def_hash, &input, &pos);

    let diff_def = derive_mote_id(&MoteDefHash::from_bytes([6u8; 32]), &input, &pos);
    let diff_input = derive_mote_id(&def_hash, &InputDataId::from_bytes([9u8; 32]), &pos);
    let diff_pos = derive_mote_id(&def_hash, &input, &GraphPosition(vec![1]));

    assert_ne!(base, diff_def);
    assert_ne!(base, diff_input);
    assert_ne!(base, diff_pos);
    assert_ne!(diff_def, diff_input);
}

#[test]
fn mote_id_display_is_64_hex_chars() {
    let id = MoteId::from_bytes([0xab; 32]);
    let s = format!("{id}");
    assert_eq!(s.len(), 64);
    assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn legal_transitions_are_accepted() {
    use AttemptState::*;
    assert!(transition(Pending, Scheduled).is_ok());
    assert!(transition(Scheduled, Running).is_ok());
    assert!(transition(Running, Committed).is_ok());
    assert!(transition(Running, Failed).is_ok());
    assert!(transition(Committed, Repudiated).is_ok());
}

#[test]
fn exhaustive_transition_matrix() {
    use AttemptState::*;
    let legal: std::collections::HashSet<(AttemptState, AttemptState)> = [
        (Pending, Scheduled),
        (Scheduled, Running),
        (Running, Committed),
        (Running, Failed),
        (Committed, Repudiated),
    ]
    .into_iter()
    .collect();

    let mut legal_count = 0usize;
    let mut illegal_count = 0usize;
    for &from in &ALL_ATTEMPT_STATES {
        for &to in &ALL_ATTEMPT_STATES {
            let result = transition(from, to);
            if legal.contains(&(from, to)) {
                assert!(
                    result.is_ok(),
                    "expected legal transition: {from:?} → {to:?}"
                );
                legal_count += 1;
            } else {
                assert!(
                    result.is_err(),
                    "expected illegal transition: {from:?} → {to:?}"
                );
                illegal_count += 1;
            }
        }
    }
    assert_eq!(legal_count, 5);
    assert_eq!(illegal_count, 36 - 5);
}

#[test]
fn failed_is_terminal() {
    use AttemptState::*;
    for &to in &ALL_ATTEMPT_STATES {
        assert!(
            transition(Failed, to).is_err(),
            "Failed → {to:?} must be illegal (terminal)"
        );
    }
}

#[test]
fn repudiated_is_terminal() {
    use AttemptState::*;
    for &to in &ALL_ATTEMPT_STATES {
        assert!(
            transition(Repudiated, to).is_err(),
            "Repudiated → {to:?} must be illegal (terminal)"
        );
    }
}

#[test]
fn self_loops_are_illegal() {
    for &s in &ALL_ATTEMPT_STATES {
        assert!(
            transition(s, s).is_err(),
            "{s:?} → {s:?} must be illegal (no self-loops)"
        );
    }
}

#[test]
fn committed_does_not_demote_to_failed() {
    use AttemptState::*;
    assert!(transition(Committed, Failed).is_err());
}

#[test]
fn mote_new_derives_id_correctly() {
    let def = sample_def();
    let input = InputDataId::from_bytes([5u8; 32]);
    let pos = GraphPosition(vec![1, 2, 3]);
    let mote = Mote::new(def.clone(), input, pos.clone(), SmallVec::new());
    let expected = derive_mote_id(&def.hash(), &input, &pos);
    assert_eq!(mote.id, expected);
}

#[test]
fn mote_graph_round_trips_a_single_mote() {
    let def = sample_def();
    let mote = Mote::new(
        def,
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition::default(),
        SmallVec::new(),
    );
    let id = mote.id;
    let mut g = MoteGraph::new();
    g.insert(mote.clone());
    assert_eq!(g.len(), 1);
    assert!(!g.is_empty());
    assert_eq!(g.get(&id), Some(&mote));
    assert_eq!(g.parents_of(&id).map(|v| v.len()), Some(0));
}

#[test]
fn illegal_transition_error_carries_states() {
    use AttemptState::*;
    let err = transition(Pending, Committed).unwrap_err();
    assert_eq!(err.from, Pending);
    assert_eq!(err.to, Committed);
    let s = format!("{err}");
    assert!(s.contains("Pending"));
    assert!(s.contains("Committed"));
}
