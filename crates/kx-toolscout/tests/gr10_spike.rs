//! GR10 — the measure-and-persist spike (run on demand, numbers → the PRIVATE
//! `docs/benchmarks/` corpus):
//!
//! ```sh
//! cargo test -p kx-toolscout --release --test gr10_spike -- --ignored --nocapture
//! ```

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use kx_bundle::{TaskBundle, TASK_BUNDLE_SCHEMA_VERSION};
use kx_dataset::InMemoryRetrievalIndex;
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_toolscout::{
    compile_bundle, Embedder, ToolFingerprint, ToolManifestIndex, TOOL_FINGERPRINT_SCHEMA_VERSION,
};
use kx_warrant::ToolGrant;
use kx_workflow::permissive_warrant;

struct HashEmbedder;
impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        // 32-dim deterministic direction from the blake3 bytes.
        blake3::hash(text.as_bytes())
            .as_bytes()
            .iter()
            .map(|b| f32::from(*b) / 255.0)
            .collect()
    }
}

#[test]
#[ignore = "GR10 spike — run with --ignored --nocapture and persist the numbers"]
fn rank_1k_fingerprints_and_lower_a_10_step_bundle() {
    let embed = HashEmbedder;

    // --- corpus: 1,000 manifests, 3 languages × 8 keywords each ---
    let mut index = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    for i in 0..1_000u32 {
        let mut keywords = BTreeMap::new();
        for lang in ["en", "hi", "ja"] {
            keywords.insert(
                lang.to_string(),
                (0..8)
                    .map(|k| format!("{lang}-kw-{i}-{k}"))
                    .collect::<BTreeSet<_>>(),
            );
        }
        let fp = ToolFingerprint {
            schema_version: TOOL_FINGERPRINT_SCHEMA_VERSION,
            tool_id: ToolName(format!("tool-{i}")),
            tool_version: ToolVersion("1".to_string()),
            description: format!("synthetic tool number {i}"),
            keywords,
        };
        let vector = embed.embed(&fp.description);
        index.insert(fp, Some(vector));
    }

    let bundle = TaskBundle {
        schema_version: TASK_BUNDLE_SCHEMA_VERSION,
        intent: "find the synthetic tool that searches the web".to_string(),
        language_tags: ["en".to_string()].into_iter().collect(),
        tool_sequence: (0..10)
            .map(|i| (ToolName(format!("tool-{i}")), ToolVersion("1".to_string())))
            .collect(),
        tool_metadata: BTreeMap::new(),
        tolerance_threshold_bp: 5_000,
    };

    let t = Instant::now();
    let without = index.rank(&bundle, None, 10);
    let rank_string_only = t.elapsed();

    let t = Instant::now();
    let with = index.rank(&bundle, Some(&embed), 10);
    let rank_with_embedding = t.elapsed();

    // --- lower + compile a 10-step bundle ---
    let mut warrant = permissive_warrant(ModelId("m".to_string()));
    warrant.tool_grants = bundle
        .tool_sequence
        .iter()
        .map(|(n, v)| ToolGrant {
            tool_id: n.clone(),
            tool_version: v.clone(),
        })
        .collect();

    let t = Instant::now();
    let one = compile_bundle(
        &bundle,
        &warrant,
        &ModelId("m".to_string()),
        &ToolName("model.generate".to_string()),
    )
    .unwrap();
    let lower_compile = t.elapsed();
    let two = compile_bundle(
        &bundle,
        &warrant,
        &ModelId("m".to_string()),
        &ToolName("model.generate".to_string()),
    )
    .unwrap();
    let a: Vec<_> = one.motes.iter().map(|m| m.mote.id).collect();
    let b: Vec<_> = two.motes.iter().map(|m| m.mote.id).collect();
    assert_eq!(a, b, "spike doubles as a determinism oracle");

    println!("GR10 kx-toolscout spike (1k manifests, 3 langs × 8 kw):");
    println!(
        "  rank k=10, string rungs only : {rank_string_only:?} (top {})",
        without.len()
    );
    println!(
        "  rank k=10, with 32-dim embed : {rank_with_embedding:?} (top {})",
        with.len()
    );
    println!(
        "  lower+compile 10-step bundle : {lower_compile:?} ({} motes)",
        one.motes.len()
    );
}
