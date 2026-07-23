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
    for want in [
        "minimal", "agentic", "full", "grounded", "reach", "codified",
    ] {
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
    // …and the skills rail: a SkillRef with instructions_ref + a tool wish.
    assert!(full.canonical.contains("\"instructions_ref\""));
    assert!(full.canonical.contains("\"skills\""));
    // the grounded case (T-RUNAPP-CONTEXT-RAIL) carries the datasets rail (dataset_ref +
    // cas_refs) + steering_config.tools.requested_grants + steering_config.context.dataset_refs.
    let grounded = cases.iter().find(|c| c.name == "grounded").unwrap();
    assert!(grounded.canonical.contains("\"datasets\""));
    assert!(grounded.canonical.contains("\"dataset_ref\":\"research\""));
    assert!(grounded
        .canonical
        .contains("\"requested_grants\":{\"retrieve\":\"1\"}"));
    assert!(grounded
        .canonical
        .contains("\"dataset_refs\":[\"research\"]"));
    // the reach case proves the additive `reach` selector rides steering_config.tools
    // (sorted before requested_grants) and serializes snake_case.
    let reach = cases.iter().find(|c| c.name == "reach").unwrap();
    assert!(reach
        .canonical
        .contains("\"tools\":{\"reach\":\"inherit_principal\",\"requested_grants\":"));
    // the codified case pins the additive `mode` field's canonical PLACEMENT — sorted
    // between `description` and `name`. That placement is the whole risk of an additive
    // field: a surface that appended it instead of sorting it would still parse every case
    // here and still emit "valid" JSON, while producing different bytes and therefore a
    // different app_ref for the same App.
    let codified = cases.iter().find(|c| c.name == "codified").unwrap();
    assert!(codified
        .canonical
        .contains("\"mode\":\"codified\",\"name\":"));
    assert!(codified.canonical.contains("\"description\":\"Turns"));
    // …and that no OTHER case emits the key at all, which is what makes the field free:
    // an App that never set a mode serializes exactly as it did before the field existed.
    for c in cases.iter().filter(|c| c.name != "codified") {
        assert!(
            !c.canonical.contains("\"mode\":"),
            "case {:?} must not carry a mode key",
            c.name
        );
    }
}
