//! The advisory toolscout seam (the W1.A5 `ListToolManifests` /
//! `ScoreTaskBundle` path).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`String` / `Vec` /
//! `[u8; 32]`) — no `kx-toolscout` / `kx-bundle` type crosses the seam, so
//! gateway-core gains NO toolscout crate dependency and stays off the writer
//! wall. The host (`kx-gateway`) implements [`ToolScoutView`] over its
//! `kx-tool-registry` + a startup-built `ToolManifestIndex`.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8 / advisory-never-authorizes.** Every score and verdict crossing
//!   this seam is DISPLAY-ONLY: never a committed fact, never an identity
//!   input, never an authorization. The host's verdict is a dry-run of the
//!   real fail-closed lowering gate whose output is DISCARDED; the broker
//!   re-gates any future real dispatch.
//! - **Fail-closed request validation.** [`validate_bundle_spec`] caps every
//!   client-supplied dimension BEFORE the seam is called (`invalid_argument`
//!   on violation) — the seam never sees an unbounded input. A `None` seam ⇒
//!   the two RPCs return `unimplemented` (old-gateway forward-compat degrade).
//! - **No floats on the wire.** Scores are integer basis points end to end.

use kx_proto::proto;

/// The fail-closed request caps (validated before the seam call).
const MAX_INTENT_BYTES: usize = 4_096;
const MAX_LANGUAGE_TAGS: usize = 16;
const MAX_LANGUAGE_TAG_BYTES: usize = 35;
const MAX_TOOLS: usize = 32;
const MAX_TOOL_ID_BYTES: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 1_024;
const MAX_KEYWORD_SETS_PER_TOOL: usize = 16;
const MAX_WORDS_PER_SET: usize = 64;
const MAX_WORD_BYTES: usize = 128;
const MAX_THRESHOLD_BP: u32 = 10_000;

/// Normalized keywords under one language tag (wire-vocabulary mirror of the
/// proto `KeywordSet`).
#[derive(Clone, Debug)]
pub struct KeywordSetEntry {
    /// The BCP-47-ish language tag (e.g. `"en"`).
    pub lang: String,
    /// The keywords (the host re-normalizes; clients SHOULD pre-normalize).
    pub words: Vec<String>,
}

/// One registered tool's advisory manifest, as the seam speaks it.
#[derive(Clone, Debug)]
pub struct ToolManifestEntry {
    /// The tool's exact name (the grant-set identity half).
    pub tool_id: String,
    /// The tool's exact version (the other identity half).
    pub tool_version: String,
    /// Free-form human description — NEVER parsed for enforcement.
    pub description: String,
    /// Advisory keywords per language tag.
    pub keywords: Vec<KeywordSetEntry>,
    /// The 32-byte blake3 `ToolFingerprint` content hash (display/join key).
    pub fingerprint_hash: [u8; 32],
    /// The registry kind, as display text (`"Builtin"` / `"Mcp"`).
    pub kind: String,
}

/// One sequenced tool in a validated client bundle spec.
#[derive(Clone, Debug)]
pub struct BundleToolSpecEntry {
    /// The tool's exact name.
    pub tool_id: String,
    /// The tool's exact version.
    pub tool_version: String,
    /// Advisory per-tool description.
    pub description: String,
    /// Advisory per-tool keywords.
    pub keywords: Vec<KeywordSetEntry>,
}

/// A validated, capped client bundle spec — what the seam receives. Produced
/// ONLY by the handler's crate-private fail-closed validation gate, so a host
/// implementation never sees an unbounded or duplicate-bearing input.
#[derive(Clone, Debug)]
pub struct BundleSpecEntry {
    /// The task instruction (≤ 4 KiB, non-empty).
    pub intent: String,
    /// Advisory language tags (≤ 16, each ≤ 35 bytes).
    pub language_tags: Vec<String>,
    /// The ordered tool sequence (1..=32, duplicate names refused).
    pub tool_sequence: Vec<BundleToolSpecEntry>,
    /// The advisory ranking cut in basis points (0..=10 000).
    pub tolerance_threshold_bp: u16,
}

/// One manifest's advisory rank (integer basis points; never a float).
#[derive(Clone, Debug)]
pub struct ManifestScoreEntry {
    /// The ranked tool's exact name.
    pub tool_id: String,
    /// The ranked tool's exact version.
    pub tool_version: String,
    /// The ladder score in basis points (exact 10 000 > fuzz ≤ 9 000 > cosine ≤ 8 000).
    pub score_bp: u16,
    /// Joins back to [`ToolManifestEntry::fingerprint_hash`].
    pub fingerprint_hash: [u8; 32],
}

/// The lowering dry-run's outcome (display-only — the lowered `WorkflowDef`
/// is discarded host-side; nothing submits, nothing journals).
#[derive(Clone, Debug)]
pub enum LowerVerdictEntry {
    /// No live react runtime on this serve — no server warrant to gate against.
    Unavailable,
    /// The grant gate passed; the bundle lowers to a valid `WorkflowDef`.
    WouldLower,
    /// The gate refused; the detail names the reason (display prose).
    Refused(String),
}

/// The full advisory score view for one bundle spec.
#[derive(Clone, Debug)]
pub struct BundleScoreView {
    /// The 32-byte blake3 `TaskBundle` content fingerprint.
    pub bundle_fingerprint: [u8; 32],
    /// Every registered manifest, best-first (deterministic tiebreak).
    pub ranked: Vec<ManifestScoreEntry>,
    /// What the real lowering gate said (dry-run; output discarded).
    pub verdict: LowerVerdictEntry,
}

/// The advisory toolscout read seam. The host implements it over its tool
/// registry + a startup-built manifest index. A `None` seam on the service ⇒
/// `ListToolManifests` / `ScoreTaskBundle` return `unimplemented`.
pub trait ToolScoutView: Send + Sync {
    /// Every approved tool's manifest, in deterministic
    /// `(tool_id, tool_version)` order.
    fn list_manifests(&self) -> Vec<ToolManifestEntry>;

    /// Rank every manifest against the (validated) bundle spec and dry-run the
    /// real lowering gate. Pure read — no journal write, no digest change.
    fn score_bundle(&self, spec: &BundleSpecEntry) -> BundleScoreView;
}

fn validate_keyword_sets(
    sets: &[proto::KeywordSet],
    context: &str,
) -> Result<Vec<KeywordSetEntry>, String> {
    if sets.len() > MAX_KEYWORD_SETS_PER_TOOL {
        return Err(format!(
            "{context}: at most {MAX_KEYWORD_SETS_PER_TOOL} keyword sets"
        ));
    }
    sets.iter()
        .map(|s| {
            if s.lang.is_empty() || s.lang.len() > MAX_LANGUAGE_TAG_BYTES {
                return Err(format!(
                    "{context}: a keyword-set language tag must be 1..={MAX_LANGUAGE_TAG_BYTES} bytes"
                ));
            }
            if s.words.len() > MAX_WORDS_PER_SET {
                return Err(format!(
                    "{context}: at most {MAX_WORDS_PER_SET} keywords per set"
                ));
            }
            if s.words.iter().any(|w| w.is_empty() || w.len() > MAX_WORD_BYTES) {
                return Err(format!(
                    "{context}: each keyword must be 1..={MAX_WORD_BYTES} bytes"
                ));
            }
            Ok(KeywordSetEntry {
                lang: s.lang.clone(),
                words: s.words.clone(),
            })
        })
        .collect()
}

/// Validate + cap a client `ScoreTaskBundleRequest` fail-closed. Every
/// violation is a detail string the handler maps to `invalid_argument` BEFORE
/// the seam runs — the host never sees an oversized, empty, or
/// duplicate-bearing spec. (The small `String` error keeps the validators off
/// clippy's `result_large_err`; `tonic::Status` exists only at the handler.)
pub(crate) fn validate_bundle_spec(
    req: &proto::ScoreTaskBundleRequest,
) -> Result<BundleSpecEntry, String> {
    if req.intent.is_empty() || req.intent.len() > MAX_INTENT_BYTES {
        return Err(format!("intent must be 1..={MAX_INTENT_BYTES} bytes"));
    }
    if req.language_tags.len() > MAX_LANGUAGE_TAGS {
        return Err(format!("at most {MAX_LANGUAGE_TAGS} language tags"));
    }
    if req
        .language_tags
        .iter()
        .any(|t| t.is_empty() || t.len() > MAX_LANGUAGE_TAG_BYTES)
    {
        return Err(format!(
            "each language tag must be 1..={MAX_LANGUAGE_TAG_BYTES} bytes"
        ));
    }
    if req.tool_sequence.is_empty() || req.tool_sequence.len() > MAX_TOOLS {
        return Err(format!("tool_sequence must name 1..={MAX_TOOLS} tools"));
    }
    if req.tolerance_threshold_bp > MAX_THRESHOLD_BP {
        return Err(format!(
            "tolerance_threshold_bp must be 0..={MAX_THRESHOLD_BP}"
        ));
    }
    let mut seen_names = std::collections::BTreeSet::new();
    let tool_sequence = req
        .tool_sequence
        .iter()
        .map(|t| {
            if t.tool_id.is_empty() || t.tool_id.len() > MAX_TOOL_ID_BYTES {
                return Err(format!(
                    "each tool_id must be 1..={MAX_TOOL_ID_BYTES} bytes"
                ));
            }
            if t.tool_version.is_empty() || t.tool_version.len() > MAX_TOOL_ID_BYTES {
                return Err(format!(
                    "each tool_version must be 1..={MAX_TOOL_ID_BYTES} bytes"
                ));
            }
            if t.description.len() > MAX_DESCRIPTION_BYTES {
                return Err(format!(
                    "each tool description must be at most {MAX_DESCRIPTION_BYTES} bytes"
                ));
            }
            // The TaskBundle's tool_metadata is keyed by name — one entry per
            // name — so a duplicate name would silently collapse. Refuse it.
            if !seen_names.insert(t.tool_id.clone()) {
                return Err(format!(
                    "duplicate tool name in tool_sequence: {}",
                    t.tool_id
                ));
            }
            Ok(BundleToolSpecEntry {
                tool_id: t.tool_id.clone(),
                tool_version: t.tool_version.clone(),
                description: t.description.clone(),
                keywords: validate_keyword_sets(&t.keywords, &t.tool_id)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: bounded to MAX_THRESHOLD_BP (10_000) above.
    Ok(BundleSpecEntry {
        intent: req.intent.clone(),
        language_tags: req.language_tags.clone(),
        tool_sequence,
        tolerance_threshold_bp: req.tolerance_threshold_bp as u16,
    })
}

fn keyword_set_to_proto(k: KeywordSetEntry) -> proto::KeywordSet {
    proto::KeywordSet {
        lang: k.lang,
        words: k.words,
    }
}

/// Map a seam manifest into the wire type.
pub(crate) fn tool_manifest_to_proto(m: ToolManifestEntry) -> proto::ToolManifest {
    proto::ToolManifest {
        tool_id: m.tool_id,
        tool_version: m.tool_version,
        description: m.description,
        keywords: m.keywords.into_iter().map(keyword_set_to_proto).collect(),
        fingerprint_hash: m.fingerprint_hash.to_vec(),
        kind: m.kind,
    }
}

/// Map a seam score view into the wire response.
pub(crate) fn bundle_score_to_proto(v: BundleScoreView) -> proto::ScoreTaskBundleResponse {
    let (verdict, verdict_detail) = match v.verdict {
        LowerVerdictEntry::Unavailable => (
            proto::LowerVerdict::Unavailable,
            "no live react runtime on this serve (run with --features inference and a model)"
                .to_string(),
        ),
        LowerVerdictEntry::WouldLower => (proto::LowerVerdict::WouldLower, String::new()),
        LowerVerdictEntry::Refused(detail) => (proto::LowerVerdict::Refused, detail),
    };
    proto::ScoreTaskBundleResponse {
        bundle_fingerprint: v.bundle_fingerprint.to_vec(),
        ranked: v
            .ranked
            .into_iter()
            .map(|r| proto::ManifestScore {
                tool_id: r.tool_id,
                tool_version: r.tool_version,
                score_bp: u32::from(r.score_bp),
                fingerprint_hash: r.fingerprint_hash.to_vec(),
            })
            .collect(),
        verdict: verdict.into(),
        verdict_detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> proto::ScoreTaskBundleRequest {
        proto::ScoreTaskBundleRequest {
            intent: "read a file from disk".to_string(),
            language_tags: vec!["en".to_string()],
            tool_sequence: vec![proto::BundleToolSpec {
                tool_id: "fs-read".to_string(),
                tool_version: "1".to_string(),
                description: "read the input".to_string(),
                keywords: vec![proto::KeywordSet {
                    lang: "en".to_string(),
                    words: vec!["read".to_string(), "file".to_string()],
                }],
            }],
            tolerance_threshold_bp: 6_000,
        }
    }

    #[test]
    fn a_valid_request_passes_and_maps_field_for_field() {
        let spec = validate_bundle_spec(&valid_request()).expect("valid");
        assert_eq!(spec.intent, "read a file from disk");
        assert_eq!(spec.language_tags, vec!["en"]);
        assert_eq!(spec.tool_sequence.len(), 1);
        assert_eq!(spec.tool_sequence[0].tool_id, "fs-read");
        assert_eq!(spec.tolerance_threshold_bp, 6_000);
    }

    #[test]
    fn every_cap_violation_is_refused_with_a_detail() {
        let mut empty_intent = valid_request();
        empty_intent.intent = String::new();
        let mut huge_intent = valid_request();
        huge_intent.intent = "x".repeat(MAX_INTENT_BYTES + 1);
        let mut many_tags = valid_request();
        many_tags.language_tags = (0..=MAX_LANGUAGE_TAGS).map(|i| format!("l{i}")).collect();
        let mut no_tools = valid_request();
        no_tools.tool_sequence.clear();
        let mut many_tools = valid_request();
        many_tools.tool_sequence = (0..=MAX_TOOLS)
            .map(|i| proto::BundleToolSpec {
                tool_id: format!("t{i}"),
                tool_version: "1".to_string(),
                description: String::new(),
                keywords: vec![],
            })
            .collect();
        let mut dup_names = valid_request();
        dup_names.tool_sequence.push(proto::BundleToolSpec {
            tool_id: "fs-read".to_string(),
            tool_version: "2".to_string(),
            description: String::new(),
            keywords: vec![],
        });
        let mut bad_threshold = valid_request();
        bad_threshold.tolerance_threshold_bp = MAX_THRESHOLD_BP + 1;

        for (name, req) in [
            ("empty intent", empty_intent),
            ("oversized intent", huge_intent),
            ("too many language tags", many_tags),
            ("empty tool_sequence", no_tools),
            ("too many tools", many_tools),
            ("duplicate tool names", dup_names),
            ("threshold above 10000", bad_threshold),
        ] {
            let detail = validate_bundle_spec(&req).expect_err(name);
            assert!(!detail.is_empty(), "{name}: the refusal carries a detail");
        }
    }

    #[test]
    fn verdicts_map_to_their_wire_values() {
        let view = |verdict| BundleScoreView {
            bundle_fingerprint: [7; 32],
            ranked: vec![],
            verdict,
        };
        let unavailable = bundle_score_to_proto(view(LowerVerdictEntry::Unavailable));
        assert_eq!(
            unavailable.verdict,
            i32::from(proto::LowerVerdict::Unavailable)
        );
        assert!(!unavailable.verdict_detail.is_empty());

        let would = bundle_score_to_proto(view(LowerVerdictEntry::WouldLower));
        assert_eq!(would.verdict, i32::from(proto::LowerVerdict::WouldLower));

        let refused =
            bundle_score_to_proto(view(LowerVerdictEntry::Refused("ungranted tool".into())));
        assert_eq!(refused.verdict, i32::from(proto::LowerVerdict::Refused));
        assert_eq!(refused.verdict_detail, "ungranted tool");
    }
}
