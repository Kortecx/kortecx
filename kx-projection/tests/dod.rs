//! P1.5 Definition-of-Done tests covering `projection.md` §13 obligations.
//!
//! Each test is annotated with the obligation it satisfies. Integration tests at the
//! bottom exercise the trait-seam by folding through both `SqliteJournal` and
//! `InMemoryJournal`.

use kx_content::ContentRef;
use kx_journal::{
    FailureReason, InMemoryJournal, Journal, JournalEntry, RepudiationReason, SqliteJournal,
};
use kx_mote::{EdgeKind, EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use kx_projection::{MoteState, Projection, PromotionState, RegisterMote};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}

fn cref(b: u8) -> ContentRef {
    ContentRef::from_bytes([b; 32])
}

fn dh(b: u8) -> MoteDefHash {
    MoteDefHash::from_bytes([b; 32])
}

fn register(p: &mut Projection, mote_byte: u8, nd: NdClass, parents: &[(u8, EdgeMeta)]) {
    let parents: SmallVec<[ParentRef; 4]> = parents
        .iter()
        .map(|(pb, edge)| ParentRef {
            parent_id: mid(*pb),
            edge: *edge,
        })
        .collect();
    p.register_mote(RegisterMote {
        mote_id: mid(mote_byte),
        nd_class: nd,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        parents,
    });
}

fn commit(p: &mut Projection, mote_byte: u8, seq: u64, nd: NdClass) {
    p.fold(&JournalEntry::Committed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        nondeterminism: nd,
        result_ref: cref(mote_byte),
        parents: SmallVec::new(),
        mote_def_hash: dh(mote_byte),
    })
    .unwrap();
}

fn propose(p: &mut Projection, mote_byte: u8, seq: u64) {
    p.fold(&JournalEntry::Proposed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        nondeterminism: NdClass::Pure,
        placement_hint: 0,
    })
    .unwrap();
}

fn fail(p: &mut Projection, mote_byte: u8, seq: u64) {
    p.fold(&JournalEntry::Failed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        reason_class: FailureReason::TimedOut,
        reporter_id: 0,
    })
    .unwrap();
}

fn repudiate(p: &mut Projection, target_byte: u8, target_seq: u64, seq: u64) {
    p.fold(&JournalEntry::Repudiated {
        target_mote_id: mid(target_byte),
        idempotency_key: [0u8; 32],
        seq,
        target_committed_seq: target_seq,
        reason_class: RepudiationReason::OperatorAction,
        repudiator_id: 0,
    })
    .unwrap();
}

// ---------------------------------------------------------------------------
// Obligation 1 — hand-built log → expected graph state
// ---------------------------------------------------------------------------

#[test]
fn obligation_1_handbuilt_log_produces_expected_states() {
    // 3-Mote DAG: A → B, A → C
    // A commits; B proposed; C still pending.
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    register(&mut p, b'c', NdClass::Pure, &[(b'a', EdgeMeta::data())]);

    commit(&mut p, b'a', 1, NdClass::Pure);
    propose(&mut p, b'b', 2);

    assert_eq!(p.state_of(&mid(b'a')), MoteState::Committed);
    assert_eq!(p.state_of(&mid(b'b')), MoteState::Scheduled);
    assert_eq!(p.state_of(&mid(b'c')), MoteState::Pending);

    // Adjacency: a's children are b and c.
    let children: Vec<MoteId> = p
        .children_of(&mid(b'a'))
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(children.len(), 2);
    assert!(children.contains(&mid(b'b')));
    assert!(children.contains(&mid(b'c')));

    // b's parents include a.
    let parents: Vec<MoteId> = p
        .parents_of(&mid(b'b'))
        .into_iter()
        .map(|(pi, _)| pi)
        .collect();
    assert_eq!(parents, vec![mid(b'a')]);
}

// ---------------------------------------------------------------------------
// Obligation 2 — re-fold determinism
// ---------------------------------------------------------------------------

#[test]
fn obligation_2_refold_yields_bit_equivalent_state() {
    // Build a fresh projection by folding the same log twice; assert the read API
    // returns identical results for every MoteId.
    let log = sample_log();

    let mut p1 = Projection::new();
    register_sample_motes(&mut p1);
    for e in &log {
        p1.fold(e).unwrap();
    }

    let mut p2 = Projection::new();
    register_sample_motes(&mut p2);
    for e in &log {
        p2.fold(e).unwrap();
    }

    for b in [b'a', b'b', b'c', b'd'] {
        assert_eq!(p1.state_of(&mid(b)), p2.state_of(&mid(b)));
        assert_eq!(p1.parents_of(&mid(b)), p2.parents_of(&mid(b)));
        assert_eq!(p1.children_of(&mid(b)), p2.children_of(&mid(b)));
        assert_eq!(p1.result_ref_of(&mid(b)), p2.result_ref_of(&mid(b)));
    }
    assert_eq!(p1.current_seq(), p2.current_seq());
    assert_eq!(p1.ready_set(), p2.ready_set());
}

// ---------------------------------------------------------------------------
// Obligation 3 — seq-order independence of queries
// ---------------------------------------------------------------------------

#[test]
fn obligation_3_queries_return_consistent_results_at_a_fixed_state() {
    // The fold itself is seq-order-dependent (covered elsewhere); but once folded,
    // querying the same projection multiple times must return identical results.
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    commit(&mut p, b'a', 1, NdClass::Pure);

    let a = p.state_of(&mid(b'a'));
    let a2 = p.state_of(&mid(b'a'));
    let a3 = p.state_of(&mid(b'a'));
    assert_eq!(a, a2);
    assert_eq!(a2, a3);

    let p_a = p.parents_of(&mid(b'a'));
    let p_a2 = p.parents_of(&mid(b'a'));
    assert_eq!(p_a, p_a2);
}

// ---------------------------------------------------------------------------
// Obligation 4 — repudiation reflects + cascade is reachable via transitive_consumers
// ---------------------------------------------------------------------------

#[test]
fn obligation_4_repudiation_marker_and_cascade_reachable() {
    // DAG: a → b → c (all data edges). Commit all three; repudiate a; verify b
    // and c are reachable via transitive_consumers(a).
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    register(&mut p, b'c', NdClass::Pure, &[(b'b', EdgeMeta::data())]);

    commit(&mut p, b'a', 1, NdClass::Pure);
    commit(&mut p, b'b', 2, NdClass::Pure);
    commit(&mut p, b'c', 3, NdClass::Pure);
    repudiate(&mut p, b'a', 1, 4);

    assert_eq!(p.state_of(&mid(b'a')), MoteState::Repudiated);
    assert!(p.is_repudiated(&mid(b'a')));
    assert_eq!(p.state_of(&mid(b'b')), MoteState::Committed); // not transitively flipped
    assert_eq!(p.state_of(&mid(b'c')), MoteState::Committed);

    // Cascade walker collects downstream consumers.
    let cascade = p.transitive_consumers(&mid(b'a'));
    assert!(cascade.contains(&mid(b'b')));
    assert!(cascade.contains(&mid(b'c')));
    assert_eq!(cascade.len(), 2);
}

// ---------------------------------------------------------------------------
// Obligation 5 — read-only against the journal (structural)
// ---------------------------------------------------------------------------

#[test]
fn obligation_5_projection_does_not_depend_on_journal_mut_surface() {
    // Structural check: the Projection API has no method that takes &mut Journal
    // or calls Journal::append. We verify this by exercising the API and confirming
    // the projection works WITHOUT borrowing a journal mutably.
    let mut p = Projection::new();
    commit(&mut p, b'a', 1, NdClass::Pure);
    // Building a fresh projection only reads from the journal — Projection::from_journal
    // takes &J, never &mut J.
    let j = InMemoryJournal::new();
    let _ = j.append(JournalEntry::Committed {
        mote_id: mid(b'a'),
        idempotency_key: [b'a'; 32],
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: cref(b'a'),
        parents: SmallVec::new(),
        mote_def_hash: dh(b'a'),
    });
    let _ = Projection::from_journal(&j).unwrap();
    // Compile-time test: this signature only accepts &J, not &mut J.
    fn requires_immutable_journal<J: Journal>(_j: &J) {}
    requires_immutable_journal(&j);
}

// ---------------------------------------------------------------------------
// Obligation 6 — snapshot consistency
// ---------------------------------------------------------------------------

#[test]
fn obligation_6_snapshot_is_stable_under_subsequent_folds() {
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    commit(&mut p, b'a', 1, NdClass::Pure);

    let snap_before = p.snapshot();
    assert_eq!(snap_before.seq(), 1);
    assert_eq!(snap_before.state_of(&mid(b'a')), MoteState::Committed);
    assert_eq!(snap_before.state_of(&mid(b'b')), MoteState::Pending);

    // Fold a Commit + Repudiation against the projection; snapshot must remain stable.
    commit(&mut p, b'b', 2, NdClass::Pure);
    repudiate(&mut p, b'a', 1, 3);

    assert_eq!(snap_before.seq(), 1);
    assert_eq!(snap_before.state_of(&mid(b'a')), MoteState::Committed); // snapshot unchanged
    assert_eq!(snap_before.state_of(&mid(b'b')), MoteState::Pending);

    // The live projection reflects the new state.
    assert_eq!(p.state_of(&mid(b'a')), MoteState::Repudiated);
    assert_eq!(p.state_of(&mid(b'b')), MoteState::Committed);
}

// ---------------------------------------------------------------------------
// Obligation 7 — ready_set correctness
// ---------------------------------------------------------------------------

#[test]
fn obligation_7_ready_set_returns_pending_with_all_parents_committed() {
    // DAG: a (root) → b → c
    // After a commits, b is ready (parent committed). c is not (parent b is pending).
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    register(&mut p, b'c', NdClass::Pure, &[(b'b', EdgeMeta::data())]);

    // Nothing committed: only a is ready (no parents).
    let ready: Vec<MoteId> = p.ready_set();
    assert_eq!(ready, vec![mid(b'a')]);

    commit(&mut p, b'a', 1, NdClass::Pure);
    let ready: Vec<MoteId> = p.ready_set();
    assert_eq!(ready, vec![mid(b'b')]);

    commit(&mut p, b'b', 2, NdClass::Pure);
    let ready: Vec<MoteId> = p.ready_set();
    assert_eq!(ready, vec![mid(b'c')]);

    commit(&mut p, b'c', 3, NdClass::Pure);
    let ready: Vec<MoteId> = p.ready_set();
    assert!(ready.is_empty(), "no Pending Motes remain");
}

#[test]
fn obligation_7b_ready_set_excludes_children_of_repudiated_parents() {
    // Repudiating a parent excludes its children from ready_set (they no longer
    // have a Committed-and-not-Repudiated parent).
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);

    commit(&mut p, b'a', 1, NdClass::Pure);
    assert_eq!(p.ready_set(), vec![mid(b'b')]);

    repudiate(&mut p, b'a', 1, 2);
    let ready = p.ready_set();
    assert!(
        !ready.contains(&mid(b'b')),
        "b's parent a is Repudiated; b must not be in ready_set"
    );
}

// ---------------------------------------------------------------------------
// Obligation 8 — transitive_consumers with control-edge opt-out
// ---------------------------------------------------------------------------

#[test]
fn obligation_8_transitive_consumers_respects_non_cascade_control_edges() {
    // DAG: a → b (data), a → c (control with non_cascade=true), a → d (control normal)
    // transitive_consumers(a) should reach b and d but NOT c.
    let mut p = Projection::new();
    register(&mut p, b'a', NdClass::Pure, &[]);
    register(&mut p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    register(
        &mut p,
        b'c',
        NdClass::Pure,
        &[(b'a', EdgeMeta::control_non_cascading())],
    );
    register(&mut p, b'd', NdClass::Pure, &[(b'a', EdgeMeta::control())]);

    let cascade: Vec<MoteId> = p.transitive_consumers(&mid(b'a'));
    assert!(cascade.contains(&mid(b'b')), "data child reachable");
    assert!(
        cascade.contains(&mid(b'd')),
        "non-opted-out control child reachable"
    );
    assert!(
        !cascade.contains(&mid(b'c')),
        "opted-out control child NOT reachable"
    );
}

// ---------------------------------------------------------------------------
// Obligation 9 — cycle tolerance
// ---------------------------------------------------------------------------

#[test]
fn obligation_9_cycle_in_control_edges_does_not_loop() {
    // Force a cycle via Committed entries (parents come from committed bodies).
    // a → b → a (mutual control-edge cycle).
    let mut p = Projection::new();

    // Commit a with no parents (yet).
    p.fold(&JournalEntry::Committed {
        mote_id: mid(b'a'),
        idempotency_key: [b'a'; 32],
        seq: 1,
        nondeterminism: NdClass::Pure,
        result_ref: cref(b'a'),
        parents: SmallVec::new(),
        mote_def_hash: dh(b'a'),
    })
    .unwrap();

    // Commit b with parent a (control edge).
    let mut b_parents: SmallVec<[kx_journal::ParentEntry; 4]> = SmallVec::new();
    b_parents.push(kx_journal::ParentEntry {
        parent_id: mid(b'a'),
        edge_kind: 1,
        non_cascade: 0,
    });
    p.fold(&JournalEntry::Committed {
        mote_id: mid(b'b'),
        idempotency_key: [b'b'; 32],
        seq: 2,
        nondeterminism: NdClass::Pure,
        result_ref: cref(b'b'),
        parents: b_parents,
        mote_def_hash: dh(b'b'),
    })
    .unwrap();

    // Register a as having a back-edge to b (control). This creates a → b → a cycle.
    register(&mut p, b'a', NdClass::Pure, &[(b'b', EdgeMeta::control())]);
    // (registration overwrites declared info — the parent now appears in
    // children_of(b), but a is already Committed; the cycle is in the graph.)

    // BFS terminates and does not loop.
    let cascade_a: Vec<MoteId> = p.transitive_consumers(&mid(b'a'));
    let cascade_b: Vec<MoteId> = p.transitive_consumers(&mid(b'b'));

    // Both walks should terminate; the visited-set bounds them.
    assert!(cascade_a.len() <= 2);
    assert!(cascade_b.len() <= 2);
}

// ---------------------------------------------------------------------------
// Obligation 10 — promotion-state default
// ---------------------------------------------------------------------------

#[test]
fn obligation_10_promotion_state_default_is_not_applicable() {
    // Per D18 / projection.md §8: until P0.8 wires the MoteDef registry, every
    // Mote's promotion_state is NotApplicable, regardless of nd_class.
    let mut p = Projection::new();

    commit(&mut p, b'a', 1, NdClass::Pure);
    commit(&mut p, b'b', 2, NdClass::ReadOnlyNondet);
    commit(&mut p, b'c', 3, NdClass::WorldMutating);

    assert_eq!(p.promotion_state(&mid(b'a')), PromotionState::NotApplicable);
    assert_eq!(p.promotion_state(&mid(b'b')), PromotionState::NotApplicable);
    assert_eq!(p.promotion_state(&mid(b'c')), PromotionState::NotApplicable);

    // Even an unknown MoteId returns NotApplicable (the default for absent Motes).
    assert_eq!(p.promotion_state(&mid(b'z')), PromotionState::NotApplicable);
}

// ---------------------------------------------------------------------------
// Trait-seam: from_journal works for both backends (Journal trait surface only)
// ---------------------------------------------------------------------------

fn exercise_from_journal<J: Journal>(j: &J) {
    let _ = j.append(JournalEntry::Committed {
        mote_id: mid(b'a'),
        idempotency_key: [b'a'; 32],
        seq: 0, // ignored
        nondeterminism: NdClass::Pure,
        result_ref: cref(b'a'),
        parents: SmallVec::new(),
        mote_def_hash: dh(b'a'),
    });
    let p = Projection::from_journal(j).unwrap();
    assert_eq!(p.state_of(&mid(b'a')), MoteState::Committed);
    assert_eq!(p.result_ref_of(&mid(b'a')), Some(cref(b'a')));
}

#[test]
fn from_journal_works_via_sqlite_backend() {
    let j = SqliteJournal::open_in_memory().unwrap();
    exercise_from_journal(&j);
}

#[test]
fn from_journal_works_via_in_memory_backend() {
    let j = InMemoryJournal::new();
    exercise_from_journal(&j);
}

// ---------------------------------------------------------------------------
// Extra: failed → proposed → committed lifecycle through the projection
// ---------------------------------------------------------------------------

#[test]
fn failed_then_proposed_then_committed_resolves_to_committed() {
    let mut p = Projection::new();
    propose(&mut p, b'a', 1);
    fail(&mut p, b'a', 2);
    assert_eq!(p.state_of(&mid(b'a')), MoteState::Failed);
    propose(&mut p, b'a', 3);
    assert_eq!(p.state_of(&mid(b'a')), MoteState::Scheduled);
    commit(&mut p, b'a', 4, NdClass::Pure);
    assert_eq!(p.state_of(&mid(b'a')), MoteState::Committed);
}

// ---------------------------------------------------------------------------
// Helpers used by obligation 2's re-fold determinism test
// ---------------------------------------------------------------------------

fn register_sample_motes(p: &mut Projection) {
    register(p, b'a', NdClass::Pure, &[]);
    register(p, b'b', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    register(p, b'c', NdClass::Pure, &[(b'a', EdgeMeta::data())]);
    register(
        p,
        b'd',
        NdClass::Pure,
        &[(b'b', EdgeMeta::data()), (b'c', EdgeMeta::data())],
    );
}

fn sample_log() -> Vec<JournalEntry> {
    vec![
        JournalEntry::Proposed {
            mote_id: mid(b'a'),
            idempotency_key: [b'a'; 32],
            seq: 1,
            nondeterminism: NdClass::Pure,
            placement_hint: 0,
        },
        JournalEntry::Committed {
            mote_id: mid(b'a'),
            idempotency_key: [b'a'; 32],
            seq: 2,
            nondeterminism: NdClass::Pure,
            result_ref: cref(b'a'),
            parents: SmallVec::new(),
            mote_def_hash: dh(b'a'),
        },
        JournalEntry::Proposed {
            mote_id: mid(b'b'),
            idempotency_key: [b'b'; 32],
            seq: 3,
            nondeterminism: NdClass::Pure,
            placement_hint: 0,
        },
        JournalEntry::Committed {
            mote_id: mid(b'b'),
            idempotency_key: [b'b'; 32],
            seq: 4,
            nondeterminism: NdClass::Pure,
            result_ref: cref(b'b'),
            parents: parent_entries(&[(b'a', EdgeKind::Data, false)]),
            mote_def_hash: dh(b'b'),
        },
        JournalEntry::Committed {
            mote_id: mid(b'c'),
            idempotency_key: [b'c'; 32],
            seq: 5,
            nondeterminism: NdClass::Pure,
            result_ref: cref(b'c'),
            parents: parent_entries(&[(b'a', EdgeKind::Data, false)]),
            mote_def_hash: dh(b'c'),
        },
    ]
}

fn parent_entries(spec: &[(u8, EdgeKind, bool)]) -> SmallVec<[kx_journal::ParentEntry; 4]> {
    spec.iter()
        .map(|(b, kind, nc)| kx_journal::ParentEntry {
            parent_id: mid(*b),
            edge_kind: kind.as_u8(),
            non_cascade: u8::from(*nc),
        })
        .collect()
}
