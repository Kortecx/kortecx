//! Multilingual ranking — the ladder orders across languages and the
//! threshold cut + deterministic tiebreak hold.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use kx_bundle::{TaskBundle, ToolMeta, TASK_BUNDLE_SCHEMA_VERSION};
use kx_dataset::InMemoryRetrievalIndex;
use kx_mote::{ToolName, ToolVersion};
use kx_toolscout::{
    Embedder, ToolFingerprint, ToolManifestIndex, SCORE_MAX_BP, TOOL_FINGERPRINT_SCHEMA_VERSION,
};

fn manifest(name: &str, lang_keywords: &[(&str, &[&str])], description: &str) -> ToolFingerprint {
    ToolFingerprint {
        schema_version: TOOL_FINGERPRINT_SCHEMA_VERSION,
        tool_id: ToolName(name.to_string()),
        tool_version: ToolVersion("1".to_string()),
        description: description.to_string(),
        keywords: lang_keywords
            .iter()
            .map(|(lang, words)| {
                (
                    (*lang).to_string(),
                    words
                        .iter()
                        .map(|w| (*w).to_string())
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect(),
    }
}

fn bundle(intent: &str, langs: &[&str], keywords: &[(&str, &[&str])]) -> TaskBundle {
    TaskBundle {
        schema_version: TASK_BUNDLE_SCHEMA_VERSION,
        intent: intent.to_string(),
        language_tags: langs.iter().map(|s| (*s).to_string()).collect(),
        tool_sequence: vec![(ToolName("t".to_string()), ToolVersion("1".to_string()))],
        tool_metadata: BTreeMap::from([(
            ToolName("t".to_string()),
            ToolMeta {
                description: String::new(),
                keywords: keywords
                    .iter()
                    .map(|(lang, words)| {
                        (
                            (*lang).to_string(),
                            words
                                .iter()
                                .map(|w| (*w).to_string())
                                .collect::<BTreeSet<_>>(),
                        )
                    })
                    .collect(),
            },
        )]),
        tolerance_threshold_bp: 6_000,
    }
}

/// A toy deterministic embedder: a 4-dim bag-of-bytes direction. Enough to
/// exercise the rung-3 path end-to-end without any model.
struct ToyEmbedder;
impl Embedder for ToyEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = [0.0f32; 4];
        for (i, b) in text.bytes().enumerate() {
            v[i % 4] += f32::from(b) / 255.0;
        }
        v.to_vec()
    }
}

#[test]
fn an_exact_hindi_hit_outranks_english_fuzz_and_cosine() {
    // The bundle speaks Hindi; one manifest matches खोज exactly, one only
    // fuzzes in English, one matches only by embedding.
    let b = bundle("खोज", &["hi"], &[("hi", &[] as &[&str])]);

    let exact_hi = manifest("exact-hi", &[("hi", &["खोज"])], "");
    let fuzz_en = manifest("fuzz-en", &[("en", &["खोजना"])], "");
    let cosine_only = manifest("cos-only", &[("en", &["zzz"])], "zzz");

    let mut index = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    let embed = ToyEmbedder;
    index.insert(exact_hi.clone(), Some(embed.embed("खोज")));
    index.insert(fuzz_en.clone(), Some(embed.embed("totally different")));
    index.insert(cosine_only.clone(), Some(embed.embed("खोज")));

    let ranked = index.rank(&b, Some(&embed), 3);
    assert_eq!(ranked.len(), 3);
    assert_eq!(ranked[0].0, exact_hi.fingerprint_hash(), "exact wins");
    assert_eq!(ranked[0].1, SCORE_MAX_BP);
    // The खोज/खोजना fuzz beats the no-string cosine-only hit (rung order).
    assert_eq!(ranked[1].0, fuzz_en.fingerprint_hash());
    assert!(ranked[1].1 > ranked[2].1);
}

#[test]
fn the_threshold_cut_is_callers_advisory_filter() {
    let b = bundle("search the web", &["en"], &[]);
    let strong = manifest("strong", &[("en", &["search"])], "");
    let weak = manifest("weak", &[("en", &["zzz"])], "qqq");

    let mut index = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    index.insert(strong, None);
    index.insert(weak, None);

    let ranked = index.rank(&b, None, 10);
    let above: Vec<_> = ranked
        .iter()
        .filter(|(_, score)| *score >= b.tolerance_threshold_bp)
        .collect();
    assert_eq!(above.len(), 1, "only the strong hit passes the 6000bp cut");
}

#[test]
fn equal_scores_tiebreak_by_ascending_content_ref() {
    // Two manifests with the identical keyword set score identically — the
    // order must still be deterministic (ascending ContentRef, the
    // InMemoryRetrievalIndex tiebreak mirrored).
    let b = bundle("echo", &["en"], &[]);
    let one = manifest("one", &[("en", &["echo"])], "");
    let two = manifest("two", &[("en", &["echo"])], "");

    let mut index = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    index.insert(one.clone(), None);
    index.insert(two.clone(), None);

    let ranked = index.rank(&b, None, 2);
    assert_eq!(ranked[0].1, ranked[1].1, "identical scores");
    let mut expected = [one.fingerprint_hash(), two.fingerprint_hash()];
    expected.sort_by(|a, c| a.as_bytes().cmp(c.as_bytes()));
    assert_eq!(ranked[0].0, expected[0]);
    assert_eq!(ranked[1].0, expected[1]);
}

#[test]
fn embedderless_rank_equals_string_rungs_only() {
    let b = bundle("summarize this", &["en"], &[]);
    let m1 = manifest("m1", &[("en", &["summarize"])], "");
    let m2 = manifest("m2", &[("en", &["summary"])], "");

    let mut with_vectors = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    let embed = ToyEmbedder;
    with_vectors.insert(m1.clone(), Some(embed.embed("noise one")));
    with_vectors.insert(m2.clone(), Some(embed.embed("noise two")));

    let mut without_vectors = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    without_vectors.insert(m1, None);
    without_vectors.insert(m2, None);

    // No embedder passed ⇒ vectors are inert; the two indexes agree exactly.
    assert_eq!(
        with_vectors.rank(&b, None, 2),
        without_vectors.rank(&b, None, 2),
        "the neutral fallback ignores stored vectors"
    );
}
