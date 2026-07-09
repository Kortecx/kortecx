//! Cross-surface golden corpus for `kortecx.appbundle/v1`.
//!
//! `tests/golden/apps/bundle_corpus.json` pins the exact canonical bundle bytes so
//! the Rust / Python / TypeScript codecs stay byte-for-byte identical. The contract
//! (mirrors the envelope corpus): for every committed `bundle` string `s`,
//! `from_json(s)` → `to_json()` MUST equal `s`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeMap;

use kx_appbundle::AppBundle;

fn r(seed: u8) -> String {
    format!("{seed:02x}").repeat(32)
}

/// The canonical cases, constructed from typed inputs. `to_json()` of each must
/// equal the committed corpus entry of the same name.
fn cases() -> Vec<(&'static str, AppBundle)> {
    // empty-closure: a minimal envelope, no blobs, no lineage.
    let empty = AppBundle {
        app_digest: r(0x11),
        source_digest: None,
        envelope: br#"{"blueprint":{"steps":[]},"name":"hello-app","schema":"kortecx.app/v1","version":"1"}"#.to_vec(),
        blobs: BTreeMap::new(),
    };

    // single-blob: one prompt body travels.
    let mut single_blobs = BTreeMap::new();
    single_blobs.insert(r(0xaa), b"You are a helpful assistant.".to_vec());
    let single = AppBundle {
        app_digest: r(0x22),
        source_digest: None,
        envelope: br#"{"blueprint":{"steps":[{"kind":"model","prompt":"Help."}]},"name":"helper","references":{"prompts":[{"content_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"sys"}]},"schema":"kortecx.app/v1","version":"1"}"#.to_vec(),
        blobs: single_blobs,
    };

    // multi-blob: binary bytes + a text rule; inserted out of order to test sorting.
    let mut multi_blobs = BTreeMap::new();
    multi_blobs.insert(r(0xbb), b"Always cite your sources.".to_vec());
    multi_blobs.insert(r(0xaa), vec![0u8, 1, 2, 253, 254, 255]);
    let multi = AppBundle {
        app_digest: r(0x33),
        source_digest: None,
        envelope: br#"{"blueprint":{"steps":[{"kind":"model","prompt":"Answer."}]},"name":"grounded","references":{"context":[{"content_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","media_type":"application/octet-stream","name":"blob"}],"rules":[{"content_ref":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","name":"cite"}]},"schema":"kortecx.app/v1","version":"1"}"#.to_vec(),
        blobs: multi_blobs,
    };

    // clone-lineage: single-blob plus a source_digest (a clone/import records it).
    let mut clone_blobs = BTreeMap::new();
    clone_blobs.insert(r(0xaa), b"You are a helpful assistant.".to_vec());
    let clone = AppBundle {
        app_digest: r(0x44),
        source_digest: Some(r(0x22)),
        envelope: br#"{"blueprint":{"steps":[{"kind":"model","prompt":"Help."}]},"name":"helper-copy","references":{"prompts":[{"content_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"sys"}]},"schema":"kortecx.app/v1","version":"1"}"#.to_vec(),
        blobs: clone_blobs,
    };

    vec![
        ("empty-closure", empty),
        ("single-blob", single),
        ("multi-blob", multi),
        ("clone-lineage", clone),
    ]
}

#[derive(serde::Deserialize)]
struct Entry {
    name: String,
    bundle: String,
}

/// Print the corpus when `KX_REGEN_GOLDEN=1`; otherwise a no-op (so `just ci`
/// never regenerates). Run with `--nocapture` to capture the committed strings.
#[test]
fn regen_prints_the_corpus() {
    if std::env::var("KX_REGEN_GOLDEN").as_deref() != Ok("1") {
        return;
    }
    let mut out = String::from("[\n");
    let cs = cases();
    for (i, (name, bundle)) in cs.iter().enumerate() {
        let json = bundle.to_json().unwrap();
        let entry = serde_json::json!({ "name": name, "bundle": json });
        out.push_str("  ");
        out.push_str(&serde_json::to_string(&entry).unwrap());
        if i + 1 < cs.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("]\n");
    println!("{out}");
}

/// The cross-language contract: every committed bundle string round-trips
/// byte-for-byte through `from_json` → `to_json`.
#[test]
fn corpus_round_trips_byte_identically() {
    let corpus: Vec<Entry> = serde_json::from_str(include_str!(
        "../../../tests/golden/apps/bundle_corpus.json"
    ))
    .unwrap();
    assert!(!corpus.is_empty(), "bundle corpus must not be empty");
    for e in &corpus {
        let parsed = AppBundle::from_json(&e.bundle)
            .unwrap_or_else(|err| panic!("case {}: parse failed: {err}", e.name));
        assert_eq!(
            parsed.to_json().unwrap(),
            e.bundle,
            "case {}: canonicalization is not idempotent",
            e.name
        );
    }
}

/// The corpus is locked to the code: each constructed case's canonical bytes equal
/// the committed string (regenerate via `KX_REGEN_GOLDEN=1` if this drifts).
#[test]
fn corpus_matches_constructed_cases() {
    let corpus: Vec<Entry> = serde_json::from_str(include_str!(
        "../../../tests/golden/apps/bundle_corpus.json"
    ))
    .unwrap();
    for (name, bundle) in cases() {
        let entry = corpus
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("corpus missing case {name}"));
        assert_eq!(
            bundle.to_json().unwrap(),
            entry.bundle,
            "case {name}: constructed bytes drifted from the corpus"
        );
    }
}
