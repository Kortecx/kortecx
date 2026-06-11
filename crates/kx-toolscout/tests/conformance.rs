//! The W1.A5 conformance suite — the advisory-never-authorizes contract,
//! pinned as tests:
//!
//! 1. An ungranted tool (unknown name OR right-name-wrong-version) refuses
//!    LOWERING, before any step exists.
//! 2. A fully-granted bundle lowers and passes the FROZEN compile, in
//!    sequence order, each Mote carrying its singleton tool contract.
//! 3. Lowering is deterministic — byte-identical `MoteId`s across runs (plus
//!    a proptest sweep over arbitrary small bundles).
//! 4. Scores STRUCTURALLY cannot reach lowering: two index states that rank
//!    the same tools in OPPOSITE orders lower to byte-identical DAGs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use kx_bundle::{TaskBundle, TASK_BUNDLE_SCHEMA_VERSION};
use kx_dataset::InMemoryRetrievalIndex;
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_toolscout::{
    compile_bundle, lower_to_workflow_def, ToolFingerprint, ToolManifestIndex, ToolScoutError,
    TOOL_FINGERPRINT_SCHEMA_VERSION,
};
use kx_warrant::ToolGrant;
use kx_workflow::{compile, permissive_warrant};
use proptest::prelude::*;

fn model() -> ModelId {
    ModelId("test-model".to_string())
}

fn capability() -> ToolName {
    ToolName("model.generate".to_string())
}

fn tool(name: &str, version: &str) -> (ToolName, ToolVersion) {
    (ToolName(name.to_string()), ToolVersion(version.to_string()))
}

/// A permissive warrant granting exactly `tools`.
fn warrant_granting(tools: &[(ToolName, ToolVersion)]) -> kx_warrant::WarrantSpec {
    let mut w = permissive_warrant(model());
    w.tool_grants = tools
        .iter()
        .map(|(n, v)| ToolGrant {
            tool_id: n.clone(),
            tool_version: v.clone(),
        })
        .collect::<BTreeSet<_>>();
    w
}

fn bundle_of(tools: Vec<(ToolName, ToolVersion)>, intent: &str) -> TaskBundle {
    TaskBundle {
        schema_version: TASK_BUNDLE_SCHEMA_VERSION,
        intent: intent.to_string(),
        language_tags: ["en".to_string()].into_iter().collect(),
        tool_sequence: tools,
        tool_metadata: BTreeMap::new(),
        tolerance_threshold_bp: 5_000,
    }
}

#[test]
fn ungranted_tool_refuses_lowering() {
    let granted = tool("web-search", "2");
    let bundle = bundle_of(vec![granted.clone(), tool("exfiltrate", "1")], "find it");
    let warrant = warrant_granting(&[granted]);

    let err = lower_to_workflow_def(&bundle, &warrant, &model(), &capability()).unwrap_err();
    assert!(
        matches!(err, ToolScoutError::UngrantedTool { ref name, .. } if name.0 == "exfiltrate"),
        "unknown tool must refuse: {err}"
    );
}

#[test]
fn right_name_wrong_version_is_just_as_refused() {
    // SN-8: the grant is the exact (name, version) PAIR — a version drift is
    // an ungranted tool, never a fuzzy match (the kx-toolcall pin, restated).
    let bundle = bundle_of(vec![tool("web-search", "3")], "find it");
    let warrant = warrant_granting(&[tool("web-search", "2")]);

    let err = lower_to_workflow_def(&bundle, &warrant, &model(), &capability()).unwrap_err();
    assert!(matches!(err, ToolScoutError::UngrantedTool { .. }), "{err}");
}

#[test]
fn empty_bundle_refuses() {
    let bundle = bundle_of(vec![], "do nothing");
    let warrant = warrant_granting(&[]);
    let err = lower_to_workflow_def(&bundle, &warrant, &model(), &capability()).unwrap_err();
    assert!(matches!(err, ToolScoutError::EmptyBundle));
}

#[test]
fn granted_bundle_lowers_and_compiles_in_sequence_order() {
    let seq = vec![tool("web-search", "2"), tool("summarize", "1")];
    let bundle = bundle_of(seq.clone(), "find and summarize");
    let warrant = warrant_granting(&seq);

    let compiled = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
    assert_eq!(compiled.motes.len(), 2);
    // Topological (submission) order is the sequence order — and each Mote
    // carries exactly its own singleton tool contract.
    for (mote, (name, version)) in compiled.motes.iter().zip(seq.iter()) {
        assert_eq!(
            mote.mote.def.tool_contract,
            BTreeMap::from([(name.clone(), version.clone())]),
            "singleton contract per step"
        );
    }
}

#[test]
fn lowering_is_deterministic() {
    let seq = vec![tool("a", "1"), tool("b", "1"), tool("c", "2")];
    let bundle = bundle_of(seq.clone(), "chain them");
    let warrant = warrant_granting(&seq);

    let one = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
    let two = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
    let ids =
        |c: &kx_workflow::CompiledWorkflow| c.motes.iter().map(|m| m.mote.id).collect::<Vec<_>>();
    assert_eq!(ids(&one), ids(&two), "byte-identical MoteIds across runs");
}

#[test]
fn scores_never_reach_lowering() {
    // Two manifest indexes ranking the SAME tools in OPPOSITE orders (one has
    // an exact keyword hit, the other only fuzz) — the lowered DAG bytes must
    // be IDENTICAL, because lowering's signature admits no score at all.
    let seq = vec![tool("web-search", "2"), tool("summarize", "1")];
    let bundle = bundle_of(seq.clone(), "search the web");
    let warrant = warrant_granting(&seq);

    let manifest = |name: &str, kw: &str| ToolFingerprint {
        schema_version: TOOL_FINGERPRINT_SCHEMA_VERSION,
        tool_id: ToolName(name.to_string()),
        tool_version: ToolVersion("2".to_string()),
        description: String::new(),
        keywords: BTreeMap::from([(
            "en".to_string(),
            [kw.to_string()].into_iter().collect::<BTreeSet<_>>(),
        )]),
    };

    let mut index_a = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    index_a.insert(manifest("web-search", "search"), None); // exact hit
    index_a.insert(manifest("summarize", "zzz"), None);
    let mut index_b = ToolManifestIndex::new(InMemoryRetrievalIndex::new());
    index_b.insert(manifest("web-search", "zzz"), None);
    index_b.insert(manifest("summarize", "search"), None); // opposite winner

    let rank_a = index_a.rank(&bundle, None, 2);
    let rank_b = index_b.rank(&bundle, None, 2);
    assert_ne!(
        rank_a.first().map(|(_, s)| *s),
        rank_b.iter().map(|(_, s)| *s).min(),
        "the two indexes really do rank differently"
    );

    let one = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
    let two = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
    let ids =
        |c: &kx_workflow::CompiledWorkflow| c.motes.iter().map(|m| m.mote.id).collect::<Vec<_>>();
    assert_eq!(ids(&one), ids(&two), "ranking state cannot move identity");
}

proptest! {
    /// Determinism sweep: ANY granted bundle (1..=6 distinct tools, arbitrary
    /// short intents) lowers + compiles to the same MoteIds twice.
    #[test]
    fn any_granted_bundle_is_deterministic(
        n in 1usize..=6,
        intent in "[a-z ]{1,32}",
    ) {
        let seq: Vec<_> = (0..n).map(|i| tool(&format!("tool-{i}"), "1")).collect();
        let bundle = bundle_of(seq.clone(), &intent);
        let warrant = warrant_granting(&seq);

        let one = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
        let two = compile_bundle(&bundle, &warrant, &model(), &capability()).unwrap();
        let a: Vec<_> = one.motes.iter().map(|m| m.mote.id).collect();
        let b: Vec<_> = two.motes.iter().map(|m| m.mote.id).collect();
        prop_assert_eq!(a, b);
    }
}

/// The frozen compile stays the structural gate: the lowered def passes the
/// SAME `compile` everything else uses (no parallel compile path).
#[test]
fn the_lowered_def_passes_the_frozen_compile_directly() {
    let seq = vec![tool("a", "1")];
    let bundle = bundle_of(seq.clone(), "one step");
    let warrant = warrant_granting(&seq);
    let wf = lower_to_workflow_def(&bundle, &warrant, &model(), &capability()).unwrap();
    compile(&wf).expect("the frozen structural gate admits the lowered def");
}
