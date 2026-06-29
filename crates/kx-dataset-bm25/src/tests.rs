// SPDX-License-Identifier: Apache-2.0
//! Unit tests: BM25 ranking correctness, idempotency, determinism, stopwords,
//! the keyword-match property, and the cache codec round-trip.

use kx_content::ContentRef;
use kx_dataset::LexicalIndex;

use crate::index::{Bm25Index, Bm25Params};
use crate::persist::{decode_records, encode_records};

fn cref(tag: u8) -> ContentRef {
    ContentRef::from_bytes([tag; 32])
}

#[test]
fn keyword_match_ranks_first() {
    let mut idx = Bm25Index::new();
    idx.insert(cref(0), "the cat sat on the mat");
    idx.insert(cref(1), "a dog ran in the park");
    idx.insert(cref(2), "quantum chromodynamics and gluon fields");
    let hits = idx.query("gluon", 3);
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0].id,
        cref(2),
        "the doc containing 'gluon' must rank first"
    );
}

#[test]
fn rarer_term_outweighs_common_term() {
    let mut idx = Bm25Index::new();
    // "the" appears everywhere (low IDF); "platypus" is rare (high IDF).
    idx.insert(cref(0), "the the the the the platypus");
    idx.insert(cref(1), "the the the the the the the");
    idx.insert(cref(2), "the the the the the the the");
    let hits = idx.query("the platypus", 3);
    assert_eq!(hits[0].id, cref(0), "the rare-term doc dominates");
}

#[test]
fn shorter_doc_with_same_term_scores_higher() {
    let mut idx = Bm25Index::new();
    idx.insert(cref(0), "needle");
    idx.insert(
        cref(1),
        "needle wrapped in a great deal of unrelated padding text that dilutes it heavily",
    );
    let hits = idx.query("needle", 2);
    // BM25 length-normalization: the concise doc ranks above the padded one.
    assert_eq!(hits[0].id, cref(0));
}

#[test]
fn idempotent_duplicate_insert() {
    let mut idx = Bm25Index::new();
    idx.insert(cref(1), "alpha beta");
    idx.insert(cref(1), "alpha beta");
    assert_eq!(idx.len(), 1);
}

#[test]
fn deterministic_repeat_query() {
    let mut idx = Bm25Index::new();
    for (i, t) in ["red green blue", "green blue", "blue alone", "red red red"]
        .iter()
        .enumerate()
    {
        idx.insert(cref(i as u8), t);
    }
    assert_eq!(idx.query("green blue", 4), idx.query("green blue", 4));
}

#[test]
fn empty_index_and_k_zero_and_empty_query_are_empty() {
    let empty = Bm25Index::new();
    assert!(empty.is_empty());
    assert!(empty.query("anything", 3).is_empty());

    let mut one = Bm25Index::new();
    one.insert(cref(1), "alpha");
    assert!(one.query("alpha", 0).is_empty());
    assert!(one.query("", 3).is_empty());
    assert!(one.query("!!!", 3).is_empty()); // tokenizes to nothing
}

#[test]
fn no_match_returns_empty() {
    let mut idx = Bm25Index::new();
    idx.insert(cref(0), "alpha beta gamma");
    assert!(idx.query("zzzz", 3).is_empty());
}

#[test]
fn stopwords_filter_when_enabled() {
    let mut on = Bm25Index::with_params(Bm25Params {
        stopwords: true,
        ..Bm25Params::default()
    });
    on.insert(cref(0), "the quick brown fox");
    // "the" is a stopword → not indexed → no hit when searched alone.
    assert!(on.query("the", 3).is_empty());
    // a content word still matches.
    assert_eq!(on.query("fox", 3)[0].id, cref(0));

    let mut off = Bm25Index::new();
    off.insert(cref(0), "the quick brown fox");
    assert!(!off.query("the", 3).is_empty(), "default keeps stopwords");
}

#[test]
fn tokenizer_is_case_insensitive_and_unicode() {
    let mut idx = Bm25Index::new();
    idx.insert(cref(0), "CamelCase MIXED café 日本語");
    assert_eq!(idx.query("camelcase", 1)[0].id, cref(0));
    assert_eq!(idx.query("CAFÉ", 1)[0].id, cref(0));
    assert_eq!(idx.query("日本語", 1)[0].id, cref(0));
}

#[test]
fn record_codec_roundtrip() {
    let ids = vec![cref(1), cref(2)];
    let texts = vec!["first document".to_string(), "second café 日本".to_string()];
    let bytes = encode_records(&ids, &texts);
    let recs = decode_records(&bytes).unwrap();
    assert_eq!(
        recs,
        vec![
            (cref(1), "first document".to_string()),
            (cref(2), "second café 日本".to_string()),
        ]
    );
}

#[test]
fn codec_rejects_garbage_and_truncation_and_trailing() {
    assert!(decode_records(b"nope").is_err());
    let bytes = encode_records(&[cref(1)], &["hello".to_string()]);
    let mut truncated = bytes.clone();
    truncated.truncate(bytes.len() - 1);
    assert!(decode_records(&truncated).is_err());
    let mut trailing = bytes;
    trailing.push(0xFF);
    assert!(decode_records(&trailing).is_err());
}

#[test]
fn rebuild_from_records_preserves_ranking() {
    let mut idx = Bm25Index::new();
    idx.insert(cref(0), "the cat sat on the mat");
    idx.insert(cref(1), "quantum gluon fields");
    let before = idx.query("gluon cat", 2);

    // Round-trip through the cache codec (the rebuild-on-open path).
    let (ids, texts) = idx.snapshot();
    let bytes = encode_records(ids, texts);
    let recs = decode_records(&bytes).unwrap();
    let mut rebuilt = Bm25Index::new();
    for (id, text) in recs {
        rebuilt.insert(id, &text);
    }
    assert_eq!(before, rebuilt.query("gluon cat", 2));
}
