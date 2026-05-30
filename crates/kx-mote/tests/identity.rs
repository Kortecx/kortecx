// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! P1.2 identity-derivation tests.
//!
//! Two-pronged coverage per the `idempotency.md` test obligations:
//!
//! - **Insensitivity to non-behavioral change**: reordering `BTreeMap` insertions
//!   (`config_subset`, `tool_contract`) must not change the canonical bytes or
//!   the resulting `MoteDefHash` / `MoteId`.
//! - **Sensitivity to behavioral change**: changing any field that affects what
//!   the Mote commits must change the canonical bytes and so the hash.
//!
//! Determinism is property-tested via `proptest` so the guarantee holds across
//! the entire input space, not just a few hand-picked cases.

use std::collections::BTreeMap;

use kx_mote::{
    canonical_config, derive_mote_id, ConfigKey, ConfigVal, EffectPattern, GraphPosition,
    InputDataId, LogicRef, ModelId, MoteDef, MoteDefHash, NdClass, PromptTemplateHash, ToolName,
    ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn base_def() -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([0xaa; 32]),
        model_id: ModelId("claude-opus-4-7:1m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0xbb; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

// ---------------------------------------------------------------------------
// Insensitivity to non-behavioral change
// ---------------------------------------------------------------------------

#[test]
fn config_subset_insertion_order_does_not_affect_hash() {
    let mut a = base_def();
    a.config_subset
        .insert(ConfigKey("temperature".into()), ConfigVal(vec![0]));
    a.config_subset
        .insert(ConfigKey("max_tokens".into()), ConfigVal(vec![100]));
    a.config_subset
        .insert(ConfigKey("seed".into()), ConfigVal(vec![42]));

    let mut b = base_def();
    b.config_subset
        .insert(ConfigKey("seed".into()), ConfigVal(vec![42]));
    b.config_subset
        .insert(ConfigKey("max_tokens".into()), ConfigVal(vec![100]));
    b.config_subset
        .insert(ConfigKey("temperature".into()), ConfigVal(vec![0]));

    assert_eq!(a.hash(), b.hash());

    let bytes_a = bincode::serde::encode_to_vec(&a, canonical_config()).unwrap();
    let bytes_b = bincode::serde::encode_to_vec(&b, canonical_config()).unwrap();
    assert_eq!(bytes_a, bytes_b);
}

#[test]
fn tool_contract_insertion_order_does_not_affect_hash() {
    let mut a = base_def();
    a.tool_contract
        .insert(ToolName("stripe".into()), ToolVersion("v2".into()));
    a.tool_contract
        .insert(ToolName("filesystem".into()), ToolVersion("v1".into()));
    a.tool_contract
        .insert(ToolName("github".into()), ToolVersion("v3".into()));

    let mut b = base_def();
    b.tool_contract
        .insert(ToolName("github".into()), ToolVersion("v3".into()));
    b.tool_contract
        .insert(ToolName("stripe".into()), ToolVersion("v2".into()));
    b.tool_contract
        .insert(ToolName("filesystem".into()), ToolVersion("v1".into()));

    assert_eq!(a.hash(), b.hash());
}

// ---------------------------------------------------------------------------
// Sensitivity to behavioral change (per `idempotency.md` test obligations)
// ---------------------------------------------------------------------------

#[test]
fn changing_logic_ref_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.logic_ref = LogicRef::from_bytes([0xab; 32]);
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_model_id_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.model_id = ModelId("claude-sonnet-4-6".into());
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_prompt_template_hash_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.prompt_template_hash = PromptTemplateHash::from_bytes([0xbc; 32]);
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_a_tool_version_changes_hash() {
    let mut a = base_def();
    a.tool_contract
        .insert(ToolName("stripe".into()), ToolVersion("v2".into()));
    let mut b = base_def();
    b.tool_contract
        .insert(ToolName("stripe".into()), ToolVersion("v3".into()));
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_nd_class_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.nd_class = NdClass::WorldMutating;
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_an_included_config_key_changes_hash() {
    let mut a = base_def();
    a.config_subset
        .insert(ConfigKey("temperature".into()), ConfigVal(vec![0]));
    let mut b = base_def();
    b.config_subset
        .insert(ConfigKey("temperature".into()), ConfigVal(vec![1]));
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_effect_pattern_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.effect_pattern = EffectPattern::ValidateThenCommit;
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_critic_for_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.critic_for = Some(kx_mote::MoteId::from_bytes([1u8; 32]));
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_is_topology_shaper_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.is_topology_shaper = true;
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn changing_schema_version_changes_hash() {
    let a = base_def();
    let mut b = base_def();
    b.schema_version = MOTE_DEF_SCHEMA_VERSION.saturating_sub(1);
    assert_ne!(a.hash(), b.hash());
}

// ---------------------------------------------------------------------------
// MoteId composition: any component change → different id
// ---------------------------------------------------------------------------

#[test]
fn entrypoint_motes_with_same_seed_collide_only_when_def_and_position_match() {
    // Two entrypoint Motes in the same run with the SAME workflow-input seed,
    // the SAME MoteDef, and the SAME graph_position derive the same MoteId.
    // Distinct graph_positions diverge them, even though the seed matches.
    let def_hash = MoteDefHash::from_bytes([0x55; 32]);
    let seed = InputDataId::from_bytes([0x11; 32]);
    let pos_a = GraphPosition(b"root/a".to_vec());
    let pos_b = GraphPosition(b"root/b".to_vec());

    let id_a = derive_mote_id(&def_hash, &seed, &pos_a);
    let id_b = derive_mote_id(&def_hash, &seed, &pos_b);
    let id_a2 = derive_mote_id(&def_hash, &seed, &pos_a);

    assert_ne!(id_a, id_b, "different graph_position must diverge");
    assert_eq!(
        id_a, id_a2,
        "identical components must produce identical id"
    );
}

// ---------------------------------------------------------------------------
// Canonical encoding: byte-determinism property tests
// ---------------------------------------------------------------------------

proptest! {
    /// Two `MoteDef`s identical in every behavioral field, regardless of how
    /// their `config_subset` was inserted, serialize to byte-identical output.
    /// This is the workhorse property: it covers an unbounded space of
    /// insertion orderings against a single sorted canonical form.
    ///
    /// Input is generated as a `BTreeMap` so keys are unique-by-construction;
    /// otherwise duplicate-key entries would produce different final maps in
    /// forward vs reverse insertion order (the LAST insertion wins) — that is
    /// a property of `BTreeMap::insert`, not a hash-stability issue.
    #[test]
    fn config_subset_canonical_bytes_invariant_under_permutation(
        // 0–6 unique-key entries, ASCII to keep proptest output legible.
        entries in prop::collection::btree_map(
            r"[a-z_][a-z0-9_]{0,8}",
            prop::collection::vec(any::<u8>(), 0..8),
            0..6,
        ),
    ) {
        // Insert the same unique-key entries in two opposite orders. With
        // unique keys, forward and reverse produce the same final BTreeMap.
        let pairs: Vec<_> = entries.into_iter().collect();
        let mut forward = BTreeMap::new();
        for (k, v) in pairs.iter() {
            forward.insert(ConfigKey(k.clone()), ConfigVal(v.clone()));
        }
        let mut reverse = BTreeMap::new();
        for (k, v) in pairs.iter().rev() {
            reverse.insert(ConfigKey(k.clone()), ConfigVal(v.clone()));
        }

        let mut a = base_def();
        a.config_subset = forward;
        let mut b = base_def();
        b.config_subset = reverse;

        let bytes_a = bincode::serde::encode_to_vec(&a, canonical_config()).unwrap();
        let bytes_b = bincode::serde::encode_to_vec(&b, canonical_config()).unwrap();
        prop_assert_eq!(bytes_a, bytes_b);
        prop_assert_eq!(a.hash(), b.hash());
    }

    /// Encoding is total (never panics, never errors) across arbitrary
    /// MoteDef payloads — including empty maps, large hash bytes, and any
    /// schema_version. The hash is a pure function of the bytes.
    #[test]
    fn mote_def_hash_is_total_and_pure(
        logic in any::<[u8; 32]>(),
        model in r"[a-z_][a-z0-9_:.-]{0,32}",
        nd in 0u8..3,
        eff in 0u8..3,
        shaper in any::<bool>(),
        version in any::<u16>(),
    ) {
        let mut d = base_def();
        d.logic_ref = LogicRef::from_bytes(logic);
        d.model_id = ModelId(model);
        d.nd_class = match nd {
            0 => NdClass::Pure,
            1 => NdClass::ReadOnlyNondet,
            _ => NdClass::WorldMutating,
        };
        d.effect_pattern = match eff {
            0 => EffectPattern::IdempotentByConstruction,
            1 => EffectPattern::StageThenCommit,
            _ => EffectPattern::ValidateThenCommit,
        };
        d.is_topology_shaper = shaper;
        d.schema_version = version;

        let h1 = d.hash();
        let h2 = d.hash();
        prop_assert_eq!(h1, h2);

        // The bytes that produced the hash must round-trip the same way.
        let bytes_a = bincode::serde::encode_to_vec(&d, canonical_config()).unwrap();
        let bytes_b = bincode::serde::encode_to_vec(&d, canonical_config()).unwrap();
        prop_assert_eq!(bytes_a, bytes_b);
    }

    /// `derive_mote_id` is a deterministic pure function across its entire
    /// 32+32+variable-length input space.
    #[test]
    fn derive_mote_id_is_deterministic(
        def_hash in any::<[u8; 32]>(),
        input in any::<[u8; 32]>(),
        pos in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let dh = MoteDefHash::from_bytes(def_hash);
        let id = InputDataId::from_bytes(input);
        let gp = GraphPosition(pos);
        let a = derive_mote_id(&dh, &id, &gp);
        let b = derive_mote_id(&dh, &id, &gp);
        prop_assert_eq!(a, b);
    }
}
