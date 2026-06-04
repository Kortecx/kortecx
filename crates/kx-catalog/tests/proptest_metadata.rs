// SPDX-License-Identifier: Apache-2.0
//! Property tests for the M7.3 advisory metadata sidecar (D84): set/get/remove,
//! deterministic [`AssetRef`]-ordered iteration, the inverted tag index matches a
//! brute-force scan, the per-asset tag bound is fail-closed, and `Tag` validation.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use kx_catalog::{
    AdvisoryMetadata, AdvisoryMetadataStore, AssetPath, AssetRef, Tag, TagError, MAX_TAGS_PER_ASSET,
};
use proptest::prelude::*;

/// A distinct path-addressed asset for index `i`.
fn asset(i: usize) -> AssetRef {
    AssetRef::Path(AssetPath::new("ns", "col", format!("n{i}")).unwrap())
}

/// Advisory metadata carrying a tag set.
fn meta_with(tags: BTreeSet<Tag>) -> AdvisoryMetadata {
    AdvisoryMetadata {
        confidence_scaled: 0,
        tags,
        curated_by: "curator".into(),
        notes: String::new(),
    }
}

#[test]
fn tag_new_rejects_empty() {
    assert!(matches!(Tag::new(""), Err(TagError::Empty)));
}

#[test]
fn tag_new_rejects_too_long() {
    let s = "a".repeat(kx_catalog::MAX_TAG_LEN + 1);
    assert!(matches!(Tag::new(s), Err(TagError::TooLong(_))));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Canonical-class strings (lowercase `[a-z0-9._-]`, bounded) always construct.
    #[test]
    fn tag_new_accepts_canonical(s in "[a-z0-9._-]{1,64}") {
        prop_assert!(Tag::new(s).is_ok());
    }

    /// Uppercase characters are outside the canonical class — fail-closed.
    #[test]
    fn tag_new_rejects_illegal_char(s in "[A-Z]{1,8}") {
        prop_assert!(matches!(Tag::new(s), Err(TagError::IllegalChar(_))));
    }

    #[test]
    fn set_get_roundtrip(i in 0usize..200, conf in any::<i64>(), tagn in 0usize..5) {
        let mut store = AdvisoryMetadataStore::new();
        let a = asset(i);
        let tags: BTreeSet<Tag> = (0..tagn).map(|t| Tag::new(format!("t{t}")).unwrap()).collect();
        let mut meta = meta_with(tags);
        meta.confidence_scaled = conf;
        store.set(a.clone(), meta.clone()).unwrap();
        prop_assert_eq!(store.get(&a), Some(&meta));
        prop_assert_eq!(store.len(), 1);
        prop_assert!(store.get(&asset(i + 1)).is_none());
    }

    /// Overwriting an asset's record re-indexes its tags: the prior tag no longer
    /// points at the asset, the new one does.
    #[test]
    fn set_overwrites_and_reindexes(i in 0usize..50) {
        let mut store = AdvisoryMetadataStore::new();
        let a = asset(i);
        let t_old = Tag::new("old").unwrap();
        let t_new = Tag::new("new").unwrap();
        store.set(a.clone(), meta_with(BTreeSet::from([t_old.clone()]))).unwrap();
        store.set(a.clone(), meta_with(BTreeSet::from([t_new.clone()]))).unwrap();
        prop_assert!(store.assets_with_tag(&t_old).next().is_none());
        let via_new: Vec<&AssetRef> = store.assets_with_tag(&t_new).collect();
        prop_assert_eq!(via_new, vec![&a]);
        prop_assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_returns_and_clears_index(i in 0usize..50) {
        let mut store = AdvisoryMetadataStore::new();
        let a = asset(i);
        let t = Tag::new("x").unwrap();
        store.set(a.clone(), meta_with(BTreeSet::from([t.clone()]))).unwrap();
        prop_assert!(store.remove(&a).is_some());
        prop_assert!(store.remove(&a).is_none());
        prop_assert!(store.assets_with_tag(&t).next().is_none());
        prop_assert!(store.is_empty());
    }

    /// The load-bearing property: the inverted tag index equals a brute-force scan
    /// over the forward map, for every tag.
    #[test]
    fn tag_index_matches_linear_scan(
        entries in prop::collection::vec((0usize..40, 0usize..6), 0..30)
    ) {
        let mut store = AdvisoryMetadataStore::new();
        let all_tags: Vec<Tag> = (0..6).map(|t| Tag::new(format!("t{t}")).unwrap()).collect();
        for (i, tagn) in entries {
            let tags: BTreeSet<Tag> = all_tags.iter().take(tagn).cloned().collect();
            store.set(asset(i), meta_with(tags)).unwrap();
        }
        for tag in &all_tags {
            let via_index: Vec<&AssetRef> = store.assets_with_tag(tag).collect();
            let via_scan: Vec<&AssetRef> = store
                .iter()
                .filter(|(_, m)| m.tags.contains(tag))
                .map(|(a, _)| a)
                .collect();
            // Both are AssetRef-ordered (BTreeSet bucket / BTreeMap iteration).
            prop_assert_eq!(via_index, via_scan);
        }
    }

    #[test]
    fn iter_is_assetref_ordered(ids in prop::collection::vec(0usize..100, 0..30)) {
        let mut store = AdvisoryMetadataStore::new();
        for i in &ids {
            store.set(asset(*i), AdvisoryMetadata::default()).unwrap();
        }
        let keys: Vec<AssetRef> = store.iter().map(|(a, _)| a.clone()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        prop_assert_eq!(keys, sorted);
    }

    /// Exceeding the per-asset tag bound is a loud, fail-closed refusal — no
    /// partial write.
    #[test]
    fn over_tag_limit_is_refused(extra in 1usize..8) {
        let mut store = AdvisoryMetadataStore::new();
        let n = MAX_TAGS_PER_ASSET + extra;
        let tags: BTreeSet<Tag> = (0..n).map(|t| Tag::new(format!("t{t}")).unwrap()).collect();
        prop_assert_eq!(tags.len(), n);
        let err = store.set(asset(1), meta_with(tags)).unwrap_err();
        prop_assert!(matches!(err, TagError::TooManyTags(_)));
        prop_assert!(store.is_empty(), "fail-closed: no partial write on refusal");
    }
}
