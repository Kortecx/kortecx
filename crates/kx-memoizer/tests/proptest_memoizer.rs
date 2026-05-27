// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `kx-memoizer` (SN-4 v2 #6 — pinned per D33 + `validate-then-commit.md` §10.5).
//!
//! Properties:
//!
//! 1. `lookup` is DETERMINISTIC — same `(mote, snapshot)` → same result.
//! 2. `lookup` is TOTAL — never panics on any input shape.
//! 3. Cache hit on a Committed Mote returns the **right variant** per
//!    the candidate's [`NdClass`] AND carries the snapshot's `result_ref`.
//! 4. Non-Committed Motes ALWAYS miss (Pending, Scheduled, Failed,
//!    Repudiated → `None`).
//! 5. Data-edge Repudiated parents POISON the cache (return `None`) even
//!    when the candidate itself is Committed.
//! 6. Control-edge Repudiated parents do NOT taint (they're sync-only;
//!    Control edges contribute no data semantics).
//! 7. WorldMutating cache hits ALWAYS carry `redispatch_effect: true` —
//!    the constraint is enforced by construction, not by convention.

use std::collections::BTreeMap;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry, RepudiationReason};
use kx_memoizer::{lookup, CacheHit};
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    MoteDefHash, MoteId, NdClass, ParentRef, PromptTemplateHash,
};
use kx_projection::{Projection, Snapshot};
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn make_mote_with_nd(mote_id: MoteId, parents: SmallVec<[ParentRef; 4]>, nd: NdClass) -> Mote {
    Mote {
        id: mote_id,
        def: MoteDef {
            logic_ref: LogicRef([0; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash([0; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: nd,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: kx_mote::InferenceParams::default(),
            schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
        },
        input_data_id: InputDataId([0; 32]),
        graph_position: GraphPosition(vec![0]),
        parents,
    }
}

fn build_committed(mote_id: MoteId, result_ref: ContentRef, nd: NdClass) -> JournalEntry {
    JournalEntry::Committed {
        mote_id,
        idempotency_key: mote_id.0,
        seq: 0,
        nondeterminism: nd,
        result_ref,
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash([0; 32]),
    }
}

fn build_repudiation(target_mote_id: MoteId, target_committed_seq: u64) -> JournalEntry {
    JournalEntry::Repudiated {
        target_mote_id,
        idempotency_key: kx_journal::repudiation_idempotency_key(
            &target_mote_id,
            target_committed_seq,
        ),
        seq: 0,
        target_committed_seq,
        reason_class: RepudiationReason::CriticInvalidated,
        repudiator_id: 0,
    }
}

/// Build a `Snapshot` with `(mote_id, result_ref, nd_class)` triples
/// committed. Optionally repudiate the first one (to test the cascade).
///
/// `InMemoryJournal::append` assigns monotonic `seq` per insertion,
/// overriding whatever the caller put in. To repudiate correctly we must
/// capture the seq the journal assigned to the Committed entry and use
/// it as the Repudiated entry's `target_committed_seq`.
fn fixture_snapshot(commits: &[(MoteId, ContentRef, NdClass)], repudiate_first: bool) -> Snapshot {
    let store = InMemoryContentStore::new();
    let journal = InMemoryJournal::new();
    let mut first_committed_seq: Option<u64> = None;
    let mut first_mid: Option<MoteId> = None;
    for (i, (mid, rr, nd)) in commits.iter().enumerate() {
        let _ = store.put(rr.as_bytes()).ok();
        let returned = journal.append(build_committed(*mid, *rr, *nd)).unwrap();
        if i == 0 {
            first_committed_seq = Some(returned.seq());
            first_mid = Some(*mid);
        }
    }
    if repudiate_first {
        if let (Some(mid), Some(seq)) = (first_mid, first_committed_seq) {
            let _ = journal.append(build_repudiation(mid, seq)).unwrap();
        }
    }
    Projection::from_journal(&journal).unwrap().snapshot()
}

fn arb_seed() -> impl Strategy<Value = u8> {
    1u8..=200
}

fn arb_payload() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=128)
}

fn arb_nd_class() -> impl Strategy<Value = NdClass> {
    // MUST be updated when a NdClass variant is added — this strategy is the
    // test surface's gate against silent variant addition (mirrors the
    // STEP 6.2 canonical-classifier-cannot-drift contract from PR 4.5).
    prop_oneof![
        Just(NdClass::Pure),
        Just(NdClass::ReadOnlyNondet),
        Just(NdClass::WorldMutating),
    ]
}

// ---------------------------------------------------------------------------
// Hand-written unit tests
// ---------------------------------------------------------------------------

#[test]
fn lookup_on_empty_projection_misses() {
    let journal = InMemoryJournal::new();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
    let mote = make_mote_with_nd(MoteId([1; 32]), SmallVec::new(), NdClass::Pure);
    assert_eq!(lookup(&mote, &snapshot), None);
}

#[test]
fn lookup_pure_committed_returns_pure_hit_with_committed_result_ref() {
    let mid = MoteId([5; 32]);
    let rr = ContentRef::of(b"pure-result");
    let snapshot = fixture_snapshot(&[(mid, rr, NdClass::Pure)], false);
    let mote = make_mote_with_nd(mid, SmallVec::new(), NdClass::Pure);
    assert_eq!(
        lookup(&mote, &snapshot),
        Some(CacheHit::Pure { result_ref: rr })
    );
}

#[test]
fn lookup_read_only_nondet_committed_returns_ron_hit() {
    let mid = MoteId([6; 32]);
    let rr = ContentRef::of(b"ron-observation");
    let snapshot = fixture_snapshot(&[(mid, rr, NdClass::ReadOnlyNondet)], false);
    let mote = make_mote_with_nd(mid, SmallVec::new(), NdClass::ReadOnlyNondet);
    assert_eq!(
        lookup(&mote, &snapshot),
        Some(CacheHit::ReadOnlyNondet { result_ref: rr })
    );
}

#[test]
fn lookup_world_mutating_committed_returns_wm_hit_with_redispatch_true() {
    let mid = MoteId([7; 32]);
    let rr = ContentRef::of(b"wm-decision");
    let snapshot = fixture_snapshot(&[(mid, rr, NdClass::WorldMutating)], false);
    let mote = make_mote_with_nd(mid, SmallVec::new(), NdClass::WorldMutating);
    let hit = lookup(&mote, &snapshot).expect("WM committed should be a cache hit");
    match hit {
        CacheHit::WorldMutating {
            result_ref,
            redispatch_effect,
        } => {
            assert_eq!(result_ref, rr);
            assert!(
                redispatch_effect,
                "WM hits MUST have redispatch_effect=true"
            );
        }
        other => panic!("expected WorldMutating, got {other:?}"),
    }
    assert!(hit.requires_redispatch());
}

#[test]
fn lookup_repudiated_mote_misses() {
    let mid = MoteId([8; 32]);
    let rr = ContentRef::of(b"repudiated-payload");
    // Commit then repudiate the same Mote.
    let snapshot = fixture_snapshot(&[(mid, rr, NdClass::Pure)], true);
    let mote = make_mote_with_nd(mid, SmallVec::new(), NdClass::Pure);
    // The Mote itself is Repudiated → cache miss.
    assert_eq!(lookup(&mote, &snapshot), None);
}

#[test]
fn lookup_with_repudiated_data_edge_parent_misses() {
    // Commit a parent + child, then repudiate the parent. Child's cache
    // hit should be poisoned by the cascade.
    let parent_id = MoteId([10; 32]);
    let child_id = MoteId([11; 32]);
    let parent_rr = ContentRef::of(b"parent-result");
    let child_rr = ContentRef::of(b"child-result");

    let store = InMemoryContentStore::new();
    let _ = store.put(parent_rr.as_bytes()).ok();
    let _ = store.put(child_rr.as_bytes()).ok();
    let journal = InMemoryJournal::new();
    let parent_committed = journal
        .append(build_committed(parent_id, parent_rr, NdClass::Pure))
        .unwrap();
    let _ = journal
        .append(build_committed(child_id, child_rr, NdClass::Pure))
        .unwrap();
    let _ = journal
        .append(build_repudiation(parent_id, parent_committed.seq()))
        .unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
        parent_id,
        edge: EdgeMeta::data(),
    }]);
    let child = make_mote_with_nd(child_id, parents, NdClass::Pure);
    assert_eq!(
        lookup(&child, &snapshot),
        None,
        "Repudiated Data-edge parent MUST poison the cache"
    );
}

#[test]
fn lookup_with_repudiated_control_edge_parent_still_hits() {
    // Control edges are sync-only; a Repudiated Control parent does NOT
    // taint the cache (Control carries no data semantics).
    let parent_id = MoteId([12; 32]);
    let child_id = MoteId([13; 32]);
    let parent_rr = ContentRef::of(b"control-parent-result");
    let child_rr = ContentRef::of(b"child-result-ctrl");

    let journal = InMemoryJournal::new();
    let parent_committed = journal
        .append(build_committed(parent_id, parent_rr, NdClass::Pure))
        .unwrap();
    let _ = journal
        .append(build_committed(child_id, child_rr, NdClass::Pure))
        .unwrap();
    let _ = journal
        .append(build_repudiation(parent_id, parent_committed.seq()))
        .unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
        parent_id,
        edge: EdgeMeta::control(),
    }]);
    let child = make_mote_with_nd(child_id, parents, NdClass::Pure);
    assert_eq!(
        lookup(&child, &snapshot),
        Some(CacheHit::Pure {
            result_ref: child_rr
        }),
        "Repudiated Control-edge parent MUST NOT poison the cache (Control = sync only)"
    );
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1: `lookup` is DETERMINISTIC — same inputs → same result.
    #[test]
    fn prop_lookup_is_deterministic(
        seed in arb_seed(),
        payload in arb_payload(),
        nd in arb_nd_class(),
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let snapshot = fixture_snapshot(&[(mid, rr, nd)], false);
        let mote = make_mote_with_nd(mid, SmallVec::new(), nd);
        let a = lookup(&mote, &snapshot);
        let b = lookup(&mote, &snapshot);
        prop_assert_eq!(a, b);
    }

    /// Property 2: `lookup` is TOTAL — never panics regardless of inputs.
    /// Sweeps NdClass × Repudiated × empty/committed snapshot.
    #[test]
    fn prop_lookup_is_total(
        seed in arb_seed(),
        payload in arb_payload(),
        nd in arb_nd_class(),
        is_committed in any::<bool>(),
        is_repudiated in any::<bool>(),
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let commits: Vec<(MoteId, ContentRef, NdClass)> = if is_committed {
            vec![(mid, rr, nd)]
        } else {
            vec![]
        };
        let snapshot = fixture_snapshot(&commits, is_repudiated);
        let mote = make_mote_with_nd(mid, SmallVec::new(), nd);
        // Reaching this assertion proves no panic.
        let _ = lookup(&mote, &snapshot);
    }

    /// Property 3: a Committed Mote (not Repudiated) with no Repudiated
    /// parents ALWAYS yields a CacheHit whose variant mirrors the
    /// candidate's NdClass AND whose result_ref matches the snapshot's.
    #[test]
    fn prop_committed_yields_correct_variant_and_ref(
        seed in arb_seed(),
        payload in arb_payload(),
        nd in arb_nd_class(),
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let snapshot = fixture_snapshot(&[(mid, rr, nd)], false);
        let mote = make_mote_with_nd(mid, SmallVec::new(), nd);
        let hit = lookup(&mote, &snapshot).expect("Committed non-Repudiated MUST hit");
        prop_assert_eq!(*hit.result_ref(), rr);
        match (nd, &hit) {
            (NdClass::Pure, CacheHit::Pure { .. })
            | (NdClass::ReadOnlyNondet, CacheHit::ReadOnlyNondet { .. })
            | (NdClass::WorldMutating, CacheHit::WorldMutating { redispatch_effect: true, .. })
                => {}
            _ => prop_assert!(false,
                "variant mismatch: candidate nd={nd:?}, hit={hit:?}"),
        }
    }

    /// Property 4: a non-Committed Mote ALWAYS misses.
    /// (Pending → not in journal; Repudiated covered separately.)
    #[test]
    fn prop_non_committed_always_misses(
        seed in arb_seed(),
        nd in arb_nd_class(),
    ) {
        // Empty journal → no commit for this mote_id → not Committed.
        let snapshot = fixture_snapshot(&[], false);
        let mote = make_mote_with_nd(MoteId([seed; 32]), SmallVec::new(), nd);
        prop_assert_eq!(lookup(&mote, &snapshot), None);
    }

    /// Property 5: a Repudiated Mote ALWAYS misses regardless of NdClass.
    #[test]
    fn prop_repudiated_self_always_misses(
        seed in arb_seed(),
        payload in arb_payload(),
        nd in arb_nd_class(),
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let snapshot = fixture_snapshot(&[(mid, rr, nd)], true);
        let mote = make_mote_with_nd(mid, SmallVec::new(), nd);
        prop_assert_eq!(lookup(&mote, &snapshot), None);
    }

    /// Property 6: WorldMutating hits ALWAYS have redispatch_effect == true.
    /// Construction-enforced (the variant only has one shape), but pin it
    /// with a property to guard against a future API change relaxing this.
    #[test]
    fn prop_world_mutating_hits_always_require_redispatch(
        seed in arb_seed(),
        payload in arb_payload(),
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let snapshot = fixture_snapshot(&[(mid, rr, NdClass::WorldMutating)], false);
        let mote = make_mote_with_nd(mid, SmallVec::new(), NdClass::WorldMutating);
        let hit = lookup(&mote, &snapshot).expect("WM Committed MUST hit");
        prop_assert!(hit.requires_redispatch());
        if let CacheHit::WorldMutating { redispatch_effect, .. } = hit {
            prop_assert!(redispatch_effect);
        } else {
            prop_assert!(false, "expected WorldMutating, got {:?}", hit);
        }
    }

    /// Property 7: lookup is PURE — read-only over snapshot, no observable
    /// side effects. We approximate "no side effects" by asserting the
    /// snapshot's state is unchanged across N back-to-back lookups.
    /// (Snapshot is immutable by type; this property guards against a
    /// future API change introducing interior mutability.)
    #[test]
    fn prop_lookup_is_pure_over_snapshot(
        seed in arb_seed(),
        payload in arb_payload(),
        nd in arb_nd_class(),
        n in 1usize..=8,
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let snapshot = fixture_snapshot(&[(mid, rr, nd)], false);
        let mote = make_mote_with_nd(mid, SmallVec::new(), nd);
        let baseline = snapshot.committed_count();
        let first = lookup(&mote, &snapshot);
        for _ in 0..n {
            let r = lookup(&mote, &snapshot);
            prop_assert_eq!(&first, &r, "lookup must be idempotent");
        }
        prop_assert_eq!(snapshot.committed_count(), baseline,
            "lookup must NOT mutate the snapshot");
    }

    /// Property 8 (class-covering sweep — mirrors PR 4.5 STEP 6.2): for
    /// every NdClass variant generated by `arb_nd_class`, a Committed
    /// non-Repudiated Mote MUST hit. If a future variant is added but the
    /// strategy isn't updated, this proptest's coverage shrinks but
    /// existing variants still pass — the comment on `arb_nd_class` is
    /// the canonical update site.
    #[test]
    fn prop_every_nd_class_can_hit(
        seed in arb_seed(),
        payload in arb_payload(),
        nd in arb_nd_class(),
    ) {
        let mid = MoteId([seed; 32]);
        let rr = ContentRef::of(&payload);
        let snapshot = fixture_snapshot(&[(mid, rr, nd)], false);
        let mote = make_mote_with_nd(mid, SmallVec::new(), nd);
        prop_assert!(lookup(&mote, &snapshot).is_some(),
            "NdClass {nd:?} MUST be cacheable when Committed and not Repudiated");
    }
}
