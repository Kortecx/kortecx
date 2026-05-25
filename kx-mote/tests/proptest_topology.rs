//! Property tests for `TopologyDecision` + `ChildDescriptor` (D37 Seam A
//! enforcement primitive). Pinned per topology.md §5 + PR 7.5.
//!
//! Properties:
//!
//! 1. `TopologyDecision::hash()` is **DETERMINISTIC** — two calls on the
//!    same value produce identical 32-byte hashes (the content-address
//!    contract; replay safety depends on this).
//! 2. `TopologyDecision::hash()` is **TOTAL** — never panics on any
//!    input shape (incl. empty children, many children, pathological
//!    Unicode in `RoleId`).
//! 3. `TopologyDecision::hash()` is **SENSITIVE TO ORDER**: reordering
//!    children produces a different hash. Order is identity-bearing
//!    because the child's `graph_position` suffix derives from index.
//! 4. `TopologyDecision::hash()` is **SENSITIVE TO CONTENT**: changing
//!    any field of any child produces a different hash.
//! 5. **D37 LOCK: no `shaper_mote_id` field anywhere on the type**.
//!    Compile-time pinned via the struct layout; if a future PR adds
//!    `shaper_mote_id` this test won't compile (the trait-impl exhaustive
//!    pattern below would fail). Documents the single-source-of-truth
//!    principle as a structural invariant.

use kx_mote::{ChildDescriptor, EffectPattern, LogicRef, NdClass, RoleId, TopologyDecision};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_nd_class() -> impl Strategy<Value = NdClass> {
    // MUST be updated when an NdClass variant is added — canonical-
    // classifier-cannot-drift TEST-level gate (the pattern shipped at
    // PR 6 for RuleSet + PR 5 for NdClass in kx-memoizer).
    prop_oneof![
        Just(NdClass::Pure),
        Just(NdClass::ReadOnlyNondet),
        Just(NdClass::WorldMutating),
    ]
}

fn arb_effect_pattern() -> impl Strategy<Value = EffectPattern> {
    // MUST be updated when an EffectPattern variant is added.
    prop_oneof![
        Just(EffectPattern::IdempotentByConstruction),
        Just(EffectPattern::StageThenCommit),
        Just(EffectPattern::ValidateThenCommit),
    ]
}

fn arb_role_id() -> impl Strategy<Value = RoleId> {
    proptest::collection::vec(
        proptest::sample::select(b"abcdefghijklmnopqrstuvwxyz0123456789-".to_vec()),
        1..=16,
    )
    .prop_map(|v| RoleId(String::from_utf8(v).expect("ascii-only")))
}

fn arb_logic_ref() -> impl Strategy<Value = LogicRef> {
    proptest::array::uniform32(any::<u8>()).prop_map(LogicRef)
}

fn arb_child_descriptor() -> impl Strategy<Value = ChildDescriptor> {
    (
        arb_role_id(),
        arb_logic_ref(),
        arb_nd_class(),
        arb_effect_pattern(),
    )
        .prop_map(
            |(role_id, logic_ref, nd_class, effect_pattern)| ChildDescriptor {
                role_id,
                logic_ref,
                nd_class,
                effect_pattern,
            },
        )
}

fn arb_topology_decision(max_children: usize) -> impl Strategy<Value = TopologyDecision> {
    proptest::collection::vec(arb_child_descriptor(), 0..=max_children)
        .prop_map(|children| TopologyDecision { children })
}

// ---------------------------------------------------------------------------
// Hand-written tests
// ---------------------------------------------------------------------------

#[test]
fn empty_topology_decision_hashes_deterministically() {
    let td = TopologyDecision { children: vec![] };
    assert_eq!(td.hash(), td.hash());
    assert_eq!(td.hash().len(), 32);
}

#[test]
fn empty_vs_singleton_have_distinct_hashes() {
    let empty = TopologyDecision { children: vec![] };
    let singleton = TopologyDecision {
        children: vec![ChildDescriptor {
            role_id: RoleId("critic".into()),
            logic_ref: LogicRef([0u8; 32]),
            nd_class: NdClass::Pure,
            effect_pattern: EffectPattern::IdempotentByConstruction,
        }],
    };
    assert_ne!(empty.hash(), singleton.hash());
}

#[test]
fn reordering_children_changes_the_hash() {
    // **D37 order is identity-bearing**: the child's graph_position
    // suffix derives from its index in the children vector.
    let child_a = ChildDescriptor {
        role_id: RoleId("a".into()),
        logic_ref: LogicRef([0u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let child_b = ChildDescriptor {
        role_id: RoleId("b".into()),
        logic_ref: LogicRef([1u8; 32]),
        nd_class: NdClass::ReadOnlyNondet,
        effect_pattern: EffectPattern::StageThenCommit,
    };
    let td_ab = TopologyDecision {
        children: vec![child_a.clone(), child_b.clone()],
    };
    let td_ba = TopologyDecision {
        children: vec![child_b, child_a],
    };
    assert_ne!(td_ab.hash(), td_ba.hash());
}

#[test]
fn changing_role_id_changes_the_hash() {
    let base = ChildDescriptor {
        role_id: RoleId("critic".into()),
        logic_ref: LogicRef([0u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let mut modified = base.clone();
    modified.role_id = RoleId("worker".into());
    let td_base = TopologyDecision {
        children: vec![base],
    };
    let td_mod = TopologyDecision {
        children: vec![modified],
    };
    assert_ne!(td_base.hash(), td_mod.hash());
}

#[test]
fn changing_logic_ref_changes_the_hash() {
    let base = ChildDescriptor {
        role_id: RoleId("critic".into()),
        logic_ref: LogicRef([0u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let mut modified = base.clone();
    modified.logic_ref = LogicRef([1u8; 32]);
    let td_base = TopologyDecision {
        children: vec![base],
    };
    let td_mod = TopologyDecision {
        children: vec![modified],
    };
    assert_ne!(td_base.hash(), td_mod.hash());
}

#[test]
fn changing_nd_class_changes_the_hash() {
    let base = ChildDescriptor {
        role_id: RoleId("critic".into()),
        logic_ref: LogicRef([0u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let mut modified = base.clone();
    modified.nd_class = NdClass::WorldMutating;
    let td_base = TopologyDecision {
        children: vec![base],
    };
    let td_mod = TopologyDecision {
        children: vec![modified],
    };
    assert_ne!(td_base.hash(), td_mod.hash());
}

#[test]
fn changing_effect_pattern_changes_the_hash() {
    let base = ChildDescriptor {
        role_id: RoleId("critic".into()),
        logic_ref: LogicRef([0u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    let mut modified = base.clone();
    modified.effect_pattern = EffectPattern::StageThenCommit;
    let td_base = TopologyDecision {
        children: vec![base],
    };
    let td_mod = TopologyDecision {
        children: vec![modified],
    };
    assert_ne!(td_base.hash(), td_mod.hash());
}

/// **D37 LOCK: no shaper_mote_id field** — the closed payload
/// contract. This test instantiates a `ChildDescriptor` with EXACTLY
/// 4 fields (role_id, logic_ref, nd_class, effect_pattern); if a
/// future PR adds a 5th field, the field-init shorthand below will
/// fail to compile (E0063 — missing field). This is a structural pin
/// of D37's single-source-of-truth principle.
#[test]
fn child_descriptor_has_exactly_four_fields_d37_lock() {
    // Field-list exhaustiveness — if a new field is added without
    // updating this test, compilation fails.
    #[allow(clippy::no_effect_underscore_binding)]
    let _ = ChildDescriptor {
        role_id: RoleId("x".into()),
        logic_ref: LogicRef([0u8; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
    };
    // Same for TopologyDecision — exactly one field (children).
    let _ = TopologyDecision { children: vec![] };
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1 (DETERMINISTIC): two calls on the same value produce
    /// identical 32-byte hashes.
    #[test]
    fn prop_topology_decision_hash_is_deterministic(td in arb_topology_decision(8)) {
        let h1 = td.hash();
        let h2 = td.hash();
        prop_assert_eq!(h1, h2);
        prop_assert_eq!(h1.len(), 32);
    }

    /// Property 2 (TOTAL): never panics on arbitrary input shapes.
    /// Sweep deeply-populated TopologyDecisions to exercise the
    /// canonical_bincode path under load.
    #[test]
    fn prop_topology_decision_hash_is_total(td in arb_topology_decision(32)) {
        // Reaching this assertion proves no panic.
        let _ = td.hash();
    }

    /// Property 3 (ORDER-SENSITIVE): swapping any two distinct
    /// children produces a different hash. Order is identity-bearing.
    /// **Note**: when the two children happen to be byte-identical
    /// (e.g., proptest generates identical descriptors), the swap is
    /// a no-op and hashes are equal — we filter that case out.
    #[test]
    fn prop_swapping_distinct_children_changes_the_hash(
        td in arb_topology_decision(8),
        i in 0usize..8,
        j in 0usize..8,
    ) {
        prop_assume!(td.children.len() >= 2);
        let i = i % td.children.len();
        let j = j % td.children.len();
        prop_assume!(i != j);
        prop_assume!(td.children[i] != td.children[j]);

        let h_orig = td.hash();
        let mut swapped = td.clone();
        swapped.children.swap(i, j);
        let h_swapped = swapped.hash();
        prop_assert_ne!(h_orig, h_swapped);
    }

    /// Property 4 (CONTENT-SENSITIVE): changing the role_id of any
    /// child produces a different hash.
    #[test]
    fn prop_changing_role_id_changes_the_hash(
        td in arb_topology_decision(8),
        idx in 0usize..8,
        new_role in arb_role_id(),
    ) {
        prop_assume!(!td.children.is_empty());
        let idx = idx % td.children.len();
        prop_assume!(td.children[idx].role_id != new_role);

        let h_orig = td.hash();
        let mut modified = td.clone();
        modified.children[idx].role_id = new_role;
        let h_mod = modified.hash();
        prop_assert_ne!(h_orig, h_mod);
    }

    /// Property 5 (cross-classifier sweep): every NdClass + EffectPattern
    /// combination is representable in a ChildDescriptor and produces a
    /// stable hash. Mirrors STEP 6.2 from PR 4.5 (canonical-classifier-
    /// cannot-drift) — if a future variant of NdClass or EffectPattern
    /// is added and the proptest strategies aren't updated, coverage
    /// shrinks but the property still passes for known variants.
    #[test]
    fn prop_every_nd_class_and_effect_pattern_combo_hashes(
        role_id in arb_role_id(),
        logic_ref in arb_logic_ref(),
        nd_class in arb_nd_class(),
        effect_pattern in arb_effect_pattern(),
    ) {
        let td = TopologyDecision {
            children: vec![ChildDescriptor {
                role_id,
                logic_ref,
                nd_class,
                effect_pattern,
            }],
        };
        let h = td.hash();
        prop_assert_eq!(h.len(), 32);
        prop_assert_eq!(td.hash(), h, "hash must be stable across calls");
    }
}
