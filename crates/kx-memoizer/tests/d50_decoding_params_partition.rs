// Integration-test file: compiled as a separate crate from the host lib.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! D50 (citation-admissibility freeze §2.51) — memoizer-collision-closed test.
//!
//! Pre-D50 the memoizer would return a `CacheHit` for a NONDET Mote whose
//! `inference_params.temperature_bps` differed from a previously-committed
//! Mote with the same other fields, because `MoteDef::hash` excluded the
//! decoding params and `derive_mote_id` collapsed the two to the same
//! `MoteId`. `kx_memoizer::lookup` keyed (and still keys) on `mote.id`
//! alone — the partition is restored *upstream* by making
//! `inference_params` identity-bearing in `MoteDef`. This test proves the
//! partition closed end-to-end.

use std::collections::BTreeMap;

use kx_content::{ContentRef, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_memoizer::lookup;
use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    MoteDefHash, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::Projection;
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

fn make_mote(def: MoteDef) -> Mote {
    Mote::new(
        def,
        InputDataId::from_bytes([0x33; 32]),
        GraphPosition(b"root".to_vec()),
        SmallVec::new(),
    )
}

fn committed_entry(mote: &Mote, result_ref: ContentRef) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: mote.id,
        idempotency_key: mote.id.0,
        seq: 0,
        nondeterminism: mote.def.nd_class,
        result_ref,
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash(mote.def.hash().0),
    }
}

#[test]
fn nondet_motes_differing_only_in_temperature_do_not_collide_on_memoizer_lookup() {
    // ARRANGE — commit the greedy Mote.
    let greedy_mote = make_mote(nondet_def(InferenceParams::default()));
    let warm_mote = make_mote(nondet_def(InferenceParams {
        temperature_bps: 7_500,
        ..InferenceParams::default()
    }));

    // Sanity: the two Motes' identities are genuinely distinct post-D50.
    assert_ne!(
        greedy_mote.id, warm_mote.id,
        "Pre-condition: two NONDET Motes with different temperature_bps \
         MUST have different MoteId; if they collide the D50 identity fix \
         has regressed."
    );

    let journal = InMemoryJournal::new();
    let result_ref = ContentRef::from_bytes([0xbb; 32]);
    journal
        .append(committed_entry(&greedy_mote, result_ref))
        .expect("commit greedy_mote succeeds");
    let _store = InMemoryContentStore::new();
    let projection = Projection::from_journal(&journal).expect("fold journal succeeds");
    let snapshot = projection.snapshot();

    // ACT — look up the warm Mote against the snapshot of a journal that
    // only ever committed the greedy Mote.
    let hit = lookup(&warm_mote, &snapshot);

    // ASSERT — the lookup MUST miss. Pre-D50 it would have returned
    // `Some(CacheHit)` carrying greedy_mote's result_ref to the
    // temperature=0.75 caller (the identity-substrate vulnerability).
    assert!(
        hit.is_none(),
        "Memoizer lookup MUST miss for a Mote with different \
         inference_params than the committed Mote. A `Some(CacheHit)` here \
         is the pre-D50 silent-corruption surface."
    );
}

#[test]
fn nondet_motes_differing_only_in_seed_do_not_collide_on_memoizer_lookup() {
    let mote_a = make_mote(nondet_def(InferenceParams {
        temperature_bps: 5_000,
        seed: 42,
        ..InferenceParams::default()
    }));
    let mote_b = make_mote(nondet_def(InferenceParams {
        temperature_bps: 5_000,
        seed: 1337,
        ..InferenceParams::default()
    }));
    assert_ne!(mote_a.id, mote_b.id);

    let journal = InMemoryJournal::new();
    journal
        .append(committed_entry(&mote_a, ContentRef::from_bytes([0xcc; 32])))
        .expect("commit mote_a succeeds");
    let projection = Projection::from_journal(&journal).expect("fold succeeds");
    let snapshot = projection.snapshot();

    let hit = lookup(&mote_b, &snapshot);
    assert!(
        hit.is_none(),
        "Memoizer MUST miss when only `seed` differs — reproducible-stochastic \
         identity demands the partition."
    );
}

#[test]
fn identical_inference_params_still_collide_as_expected() {
    // The complement: two structurally-identical Motes (same MoteDef, same
    // input_data_id, same graph_position) MUST still produce a CacheHit. This
    // pins the memoizer's positive-path correctness so the D50 partition isn't
    // mistaken for a global cache invalidation.
    let mote_a = make_mote(nondet_def(InferenceParams::default()));
    let mote_b = make_mote(nondet_def(InferenceParams::default()));
    assert_eq!(
        mote_a.id, mote_b.id,
        "Two Motes with identical inputs MUST share a MoteId — content-addressed identity."
    );

    let journal = InMemoryJournal::new();
    journal
        .append(committed_entry(&mote_a, ContentRef::from_bytes([0xdd; 32])))
        .expect("commit mote_a succeeds");
    let projection = Projection::from_journal(&journal).expect("fold succeeds");
    let snapshot = projection.snapshot();

    let hit = lookup(&mote_b, &snapshot);
    assert!(
        hit.is_some(),
        "Memoizer MUST hit on identical inputs — D50 only partitions on \
         differing identity, it does not invalidate the cache."
    );
}
