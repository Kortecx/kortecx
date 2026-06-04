// SPDX-License-Identifier: Apache-2.0
//! M7.3 action-selection-safety (SN-8 / D70 / D87): fuzzy discovery scores NEVER
//! leak into the committed selection. The catalog analog of
//! `kx-workflow/tests/retrieval_mote.rs::similarity_scores_do_not_leak_into_the_committed_fact`.
//!
//! The boundary is structural: [`SelectionFact`] has no score field and
//! [`commit_selection`] never reads a [`Hit`]'s score, so two index states that
//! return the same neighbours produce a byte-identical selection regardless of the
//! float scores. Similarity is "fuzzy in, exact out" — only the chosen EXACT
//! [`AssetRef`] is ever committed.

#![allow(clippy::unwrap_used)]

use kx_catalog::{commit_selection, AssetPath, AssetRef, ContentRef, Hit, SelectionFact};
use proptest::prelude::*;

fn cref(b: u8) -> ContentRef {
    ContentRef::from_bytes([b; 32])
}

/// The exact asset a hit's content ref (byte `b`) stands for.
fn asset(b: u8) -> AssetRef {
    AssetRef::Path(AssetPath::new("ns", "col", format!("n{b}")).unwrap())
}

/// The caller's content-ref → exact-asset resolver (the fuzzy→exact bridge).
/// Total here by construction; `commit_selection` requires the `Option` signature
/// (a real resolver returns `None` for an unmappable ref — see `unresolvable_hits_are_dropped`).
#[allow(clippy::unnecessary_wraps)]
fn resolver(c: &ContentRef) -> Option<AssetRef> {
    Some(asset(c.as_bytes()[0]))
}

#[test]
fn similarity_scores_do_not_leak_into_the_committed_selection() {
    let high = [
        Hit {
            id: cref(1),
            score: 0.99,
        },
        Hit {
            id: cref(2),
            score: 0.50,
        },
    ];
    let low = [
        Hit {
            id: cref(1),
            score: 0.11,
        },
        Hit {
            id: cref(2),
            score: 0.02,
        },
    ];
    let fa = commit_selection(&high, resolver);
    let fb = commit_selection(&low, resolver);
    assert_eq!(
        fa.encode(),
        fb.encode(),
        "scores must be excluded from the committed selection"
    );
    assert_eq!(fa.selection_ref(), fb.selection_ref());
    assert_eq!(fa, fb);
}

#[test]
fn different_neighbours_yield_different_selection() {
    let s1 = [Hit {
        id: cref(1),
        score: 0.9,
    }];
    let s2 = [Hit {
        id: cref(2),
        score: 0.9,
    }];
    assert_ne!(
        commit_selection(&s1, resolver).selection_ref(),
        commit_selection(&s2, resolver).selection_ref(),
    );
}

#[test]
fn selection_is_canonical_and_order_independent() {
    // Same neighbours, opposite (score-driven) order → identical selection.
    let a = [
        Hit {
            id: cref(1),
            score: 0.9,
        },
        Hit {
            id: cref(2),
            score: 0.1,
        },
    ];
    let b = [
        Hit {
            id: cref(2),
            score: 0.9,
        },
        Hit {
            id: cref(1),
            score: 0.1,
        },
    ];
    assert_eq!(
        commit_selection(&a, resolver),
        commit_selection(&b, resolver)
    );
}

#[test]
fn duplicate_neighbours_dedup() {
    let hits = [
        Hit {
            id: cref(1),
            score: 0.9,
        },
        Hit {
            id: cref(1),
            score: 0.4,
        },
    ];
    assert_eq!(commit_selection(&hits, resolver).chosen(), &[asset(1)]);
}

#[test]
fn unresolvable_hits_are_dropped() {
    let hits = [
        Hit {
            id: cref(1),
            score: 0.9,
        },
        Hit {
            id: cref(2),
            score: 0.8,
        },
    ];
    // Only content-ref byte 1 resolves to an exact asset.
    let only_one = |c: &ContentRef| (c.as_bytes()[0] == 1).then(|| asset(1));
    assert_eq!(commit_selection(&hits, only_one).chosen(), &[asset(1)]);
}

#[test]
fn empty_hits_yield_empty_selection() {
    let fact = commit_selection(&[], resolver);
    assert!(fact.is_empty());
    assert_eq!(fact, SelectionFact::default());
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// For an arbitrary neighbour set, perturbing every score (including NaN /
    /// infinities) leaves the committed selection byte-identical.
    #[test]
    fn selection_ref_is_score_invariant(
        ids in prop::collection::vec(0u8..32, 0..16),
        perturb in prop::collection::vec(any::<f32>(), 16),
    ) {
        let base: Vec<Hit> = ids.iter().map(|&b| Hit { id: cref(b), score: 1.0 }).collect();
        let perturbed: Vec<Hit> = ids
            .iter()
            .enumerate()
            .map(|(k, &b)| Hit { id: cref(b), score: perturb[k % perturb.len()] })
            .collect();
        let fa = commit_selection(&base, resolver);
        let fb = commit_selection(&perturbed, resolver);
        prop_assert_eq!(fa.encode(), fb.encode());
        prop_assert_eq!(fa.selection_ref(), fb.selection_ref());
    }
}
