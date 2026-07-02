//! The cross-surface byte-shape gate (GR12). `tests/golden/apps/corpus.json` pins
//! the canonical serialization of representative envelopes; Rust, Python, and TS
//! all assert idempotent canonicalization against these SAME committed strings, so
//! any divergence in key order / separators / number format / escaping is caught.
//! See `tests/golden/apps/SPEC.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_app::AppEnvelope;
use serde::Deserialize;

const CORPUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/golden/apps/corpus.json"
));

#[derive(Deserialize)]
struct Case {
    name: String,
    canonical: String,
}

#[test]
fn corpus_round_trips_byte_identically() {
    let cases: Vec<Case> = serde_json::from_str(CORPUS).expect("the golden apps corpus parses");
    assert!(cases.len() >= 3, "corpus is populated");
    for c in &cases {
        let env = AppEnvelope::from_json_slice(c.canonical.as_bytes())
            .unwrap_or_else(|e| panic!("case {:?} parses + validates: {e}", c.name));
        let re = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert_eq!(
            re, c.canonical,
            "case {:?} must canonicalize byte-identically",
            c.name
        );
    }
}

#[test]
fn corpus_covers_the_required_shapes() {
    let cases: Vec<Case> = serde_json::from_str(CORPUS).unwrap();
    let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    for want in ["minimal", "agentic", "full"] {
        assert!(
            names.contains(&want),
            "corpus must cover the {want:?} shape"
        );
    }
    // the agentic case proves an authored @-step round-trips inside the wrapper.
    let agentic = cases.iter().find(|c| c.name == "agentic").unwrap();
    assert!(agentic.canonical.contains("tool_contract"));
    assert!(agentic.canonical.contains("mcp-echo/echo"));
    // the full case proves a multi-modal media_type ref is carried at the envelope layer.
    let full = cases.iter().find(|c| c.name == "full").unwrap();
    assert!(full.canonical.contains("\"media_type\":\"image/png\""));
    // …and the skills rail (RC-SW1): a SkillRef with instructions_ref + a tool wish.
    assert!(full.canonical.contains("\"instructions_ref\""));
    assert!(full.canonical.contains("\"skills\""));
}
