//! [`fingerprint_tolerance_score`] — the advisory ranking ladder.
//!
//! Three rungs, strictly ordered so a stronger signal always outranks a
//! weaker one: exact normalized-keyword equality (10 000 bp, short-circuit) >
//! Jaro-Winkler fuzz (capped 9 000 bp) > embedding cosine (capped 8 000 bp,
//! only when the caller supplies an [`Embedder`](crate::Embedder) AND the
//! manifest was indexed with a vector — the NEUTRAL fallback: no embedder, no
//! rung, the string rungs alone decide). The result is `max(rungs)` in bp —
//! floats exist only transiently inside this function (`floor` projection),
//! so the value is deterministic across runs and SAFE to persist if a caller
//! ever needs to (integer-scaled, the `kx-catalog` advisory-metadata
//! discipline). SN-8: this number ORDERS a picker; it never authorizes.

use kx_bundle::TaskBundle;

use crate::fingerprint::{normalize_keyword, ToolFingerprint};
use crate::jw::jaro_winkler;

/// The ladder's ceiling: an exact keyword hit, in basis points.
pub const SCORE_MAX_BP: u16 = 10_000;

/// Rung 2's ceiling — strictly below rung 1 so fuzz never ties exactness.
const JW_CAP_BP: f64 = 9_000.0;

/// Rung 3's ceiling — strictly below rung 2 so opaque-vector similarity never
/// outranks a string match the user can see and audit.
const COSINE_CAP_BP: f64 = 8_000.0;

/// The query keywords: the bundle's intent tokens plus every keyword AND
/// per-tool description word from its own tool metadata, normalized,
/// restricted to languages in `bundle.language_tags` (every language when the
/// set is empty — the neutral multilingual fallback). Single tokens feed the
/// pairwise rungs; MULTI-WORD manifest keywords are handled by the
/// phrase-containment check in [`fingerprint_tolerance_score`] (a token list
/// alone could never exact-match `"web search"`).
fn query_keywords(bundle: &TaskBundle) -> Vec<String> {
    let mut out: Vec<String> = bundle
        .intent
        .split_whitespace()
        .map(normalize_keyword)
        .collect();
    for meta in bundle.tool_metadata.values() {
        out.extend(meta.description.split_whitespace().map(normalize_keyword));
        for (lang, words) in &meta.keywords {
            if bundle.language_tags.is_empty() || bundle.language_tags.contains(lang) {
                out.extend(words.iter().map(|w| normalize_keyword(w)));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Word-boundary phrase containment: `true` iff the normalized `keyword`
/// appears in `haystack_padded` (a normalized text pre-padded with one space
/// on each side) as a whole word/phrase — `"web search"` hits inside
/// `"run a web search now"`, while `"search"` can NOT hit inside
/// `"researching"`.
fn phrase_hit(haystack_padded: &str, keyword: &str) -> bool {
    !keyword.is_empty() && haystack_padded.contains(&format!(" {keyword} "))
}

/// The fingerprint's candidate keywords, language-filtered the same way.
fn manifest_keywords<'a>(bundle: &TaskBundle, fp: &'a ToolFingerprint) -> Vec<&'a str> {
    let langs_intersect = fp
        .keywords
        .keys()
        .any(|lang| bundle.language_tags.contains(lang));
    fp.keywords
        .iter()
        .filter(|(lang, _)| !langs_intersect || bundle.language_tags.contains(*lang))
        .flat_map(|(_, words)| words.iter().map(String::as_str))
        .collect()
}

/// Score `fp` against the bundle's intent. See the module note for the ladder
/// and the SN-8 contract. `embedding_cosine` is the rung-3 input the caller
/// resolves — typically [`ToolManifestIndex::rank`](crate::ToolManifestIndex)
/// querying the retrieval index; `None` is the neutral fallback.
///
/// **ADVISORY ONLY (SN-8).** This number orders a discovery/picker surface and
/// nothing else — it must NEVER gate what runs or what is granted. The sole
/// authorization gate is [`lower_to_workflow_def`](crate::lower_to_workflow_def)'s
/// exact `(name, version)` `tool_grants` check (re-enforced by the broker at
/// dispatch); that path takes no score input by construction.
#[must_use]
pub fn fingerprint_tolerance_score(
    bundle: &TaskBundle,
    fp: &ToolFingerprint,
    embedding_cosine: Option<f32>,
) -> u16 {
    let query = query_keywords(bundle);
    let candidates = manifest_keywords(bundle, fp);
    // Word-boundary-padded normalized intent, so MULTI-WORD manifest keywords
    // ("web search") can exact-hit rung 1 — token-vs-token alone never could.
    let intent_padded = format!(" {} ", normalize_keyword(&bundle.intent));

    // Rung 1 — exact normalized equality (token-vs-keyword OR the keyword as
    // a whole phrase inside the intent): the ceiling, short-circuit.
    for c in &candidates {
        if phrase_hit(&intent_padded, c) || query.iter().any(|q| q == c) {
            return SCORE_MAX_BP;
        }
    }

    // Rung 2 — best pairwise Jaro-Winkler over keywords + the description.
    let mut best_jw = 0.0f64;
    for q in &query {
        for c in &candidates {
            let s = jaro_winkler(q, c);
            if s > best_jw {
                best_jw = s;
            }
        }
        let s = jaro_winkler(q, &normalize_keyword(&fp.description));
        if s > best_jw {
            best_jw = s;
        }
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // SAFETY: best_jw ∈ [0,1] ⇒ the product ∈ [0, 9000] — in u16 range.
    let jw_bp = (best_jw * JW_CAP_BP).floor() as u16;

    // Rung 3 — embedding cosine, only when a vector similarity was resolved.
    let cos_bp = match embedding_cosine {
        Some(cos) => {
            let unit = f64::midpoint(f64::from(cos.clamp(-1.0, 1.0)), 1.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // SAFETY: unit ∈ [0,1] ⇒ the product ∈ [0, 8000] — in u16 range.
            let bp = (unit * COSINE_CAP_BP).floor() as u16;
            bp
        }
        None => 0,
    };

    jw_bp.max(cos_bp)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kx_bundle::{TaskBundle, ToolMeta, TASK_BUNDLE_SCHEMA_VERSION};
    use kx_mote::{ToolName, ToolVersion};

    use super::*;
    use crate::fingerprint::TOOL_FINGERPRINT_SCHEMA_VERSION;

    fn fp(keywords: &[(&str, &[&str])], description: &str) -> ToolFingerprint {
        ToolFingerprint {
            schema_version: TOOL_FINGERPRINT_SCHEMA_VERSION,
            tool_id: ToolName("t".to_string()),
            tool_version: ToolVersion("1".to_string()),
            description: description.to_string(),
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
        }
    }

    fn bundle(intent: &str, langs: &[&str]) -> TaskBundle {
        TaskBundle {
            schema_version: TASK_BUNDLE_SCHEMA_VERSION,
            intent: intent.to_string(),
            language_tags: langs.iter().map(|s| (*s).to_string()).collect(),
            tool_sequence: vec![(ToolName("t".to_string()), ToolVersion("1".to_string()))],
            tool_metadata: BTreeMap::from([(ToolName("t".to_string()), ToolMeta::default())]),
            tolerance_threshold_bp: 5_000,
        }
    }

    #[test]
    fn a_multi_word_manifest_keyword_exact_hits_inside_the_intent() {
        // The review-confirmed gap: token-vs-token alone could never match
        // "web search" — the phrase-containment rung must.
        let b = bundle("run a web search now", &["en"]);
        let phrase = fp(&[("en", &["web search"])], "");
        assert_eq!(fingerprint_tolerance_score(&b, &phrase, None), SCORE_MAX_BP);

        // Word boundaries hold: "search" must NOT exact-hit inside
        // "researching" (it may still fuzz on rung 2, strictly below).
        let b2 = bundle("researching the topic", &["en"]);
        let single = fp(&[("en", &["search"])], "");
        assert!(fingerprint_tolerance_score(&b2, &single, None) < SCORE_MAX_BP);
    }

    #[test]
    fn bundle_tool_meta_descriptions_feed_the_query() {
        // The bundle's own per-tool description words are rung material too.
        let mut b = bundle("zzz", &["en"]);
        b.tool_metadata.insert(
            ToolName("t".to_string()),
            ToolMeta {
                description: "deterministic echo helper".to_string(),
                keywords: BTreeMap::new(),
            },
        );
        let manifest = fp(&[("en", &["echo"])], "");
        assert_eq!(
            fingerprint_tolerance_score(&b, &manifest, None),
            SCORE_MAX_BP
        );
    }

    #[test]
    fn the_rungs_are_strictly_ordered() {
        let b = bundle("search the web", &["en"]);
        let exact = fingerprint_tolerance_score(&b, &fp(&[("en", &["search"])], ""), None);
        let fuzzy = fingerprint_tolerance_score(&b, &fp(&[("en", &["searches"])], ""), None);
        let cosine_only =
            fingerprint_tolerance_score(&b, &fp(&[("en", &["zzz"])], "zzz"), Some(0.95));
        assert_eq!(exact, SCORE_MAX_BP);
        assert!(fuzzy < exact, "jw ({fuzzy}) must stay below exact");
        assert!(
            cosine_only < fuzzy,
            "cosine ({cosine_only}) must stay below a strong jw"
        );
        assert!(cosine_only > 0);
    }

    #[test]
    fn no_embedder_is_a_neutral_fallback() {
        let b = bundle("translate the document", &["en"]);
        let manifest = fp(&[("en", &["zzz"])], "qqq");
        let without = fingerprint_tolerance_score(&b, &manifest, None);
        let with = fingerprint_tolerance_score(&b, &manifest, Some(0.9));
        assert!(without < with, "the cosine rung only adds when resolved");
    }

    #[test]
    fn language_intersection_filters_and_falls_back() {
        // The bundle speaks hi; the manifest has en+hi → only hi keywords match.
        let b = bundle("खोज", &["hi"]);
        let manifest = fp(&[("en", &["search"]), ("hi", &["खोज"])], "");
        assert_eq!(
            fingerprint_tolerance_score(&b, &manifest, None),
            SCORE_MAX_BP
        );

        // No language overlap at all → every manifest language is considered.
        let b2 = bundle("search", &["fr"]);
        let manifest2 = fp(&[("en", &["search"])], "");
        assert_eq!(
            fingerprint_tolerance_score(&b2, &manifest2, None),
            SCORE_MAX_BP
        );
    }
}
