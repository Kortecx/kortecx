//! The pinned-hex fingerprint golden — the cross-machine byte-determinism
//! oracle for [`TaskBundle`]'s canonical encoding (the I1.c discipline applied
//! to the new type). If this hex moves, the encoding bytes moved: that is a
//! `TASK_BUNDLE_SCHEMA_VERSION` bump + a deliberate, documented re-derive,
//! never an accident.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use kx_bundle::{TaskBundle, ToolMeta, TASK_BUNDLE_SCHEMA_VERSION};
use kx_mote::{ToolName, ToolVersion};

fn golden_bundle() -> TaskBundle {
    let mut keywords = BTreeMap::new();
    keywords.insert(
        "en".to_string(),
        ["search", "web"].iter().map(|s| (*s).to_string()).collect(),
    );
    keywords.insert(
        "hi".to_string(),
        ["khoj"].iter().map(|s| (*s).to_string()).collect(),
    );
    TaskBundle {
        schema_version: TASK_BUNDLE_SCHEMA_VERSION,
        intent: "find and summarize the topic".to_string(),
        language_tags: ["en".to_string(), "hi".to_string()].into_iter().collect(),
        tool_sequence: vec![
            (
                ToolName("web-search".to_string()),
                ToolVersion("2".to_string()),
            ),
            (
                ToolName("summarize".to_string()),
                ToolVersion("1".to_string()),
            ),
        ],
        tool_metadata: BTreeMap::from([(
            ToolName("web-search".to_string()),
            ToolMeta {
                description: "search the public web".to_string(),
                keywords,
            },
        )]),
        tolerance_threshold_bp: 7_500,
    }
}

#[test]
fn the_v1_fingerprint_is_pinned() {
    let hex = golden_bundle().fingerprint().to_hex();
    // Pinned at first derivation (schema v1). A change here is a deliberate
    // schema bump, never drift.
    assert_eq!(
        hex, "5843eb09131d67033e705d59bd0bd8398ee29ffea69fd68580ce39ff1c73878c",
        "TaskBundle v1 canonical encoding moved"
    );
}
