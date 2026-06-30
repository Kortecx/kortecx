//! End-to-end witnesses for the W1.A5 advisory toolscout RPCs over a REAL
//! bound port:
//!
//! - `ListToolManifests` enumerates the OSS builtin tools (the default build's
//!   registry surface) with curated, normalized keywords;
//! - `ScoreTaskBundle` ranks every manifest deterministically (an exact intent
//!   keyword scores the matching tool at the 10 000 bp ceiling), returns the
//!   bundle's content fingerprint, and — on a build with no react runtime —
//!   degrades the lowering verdict to `UNAVAILABLE`;
//! - the request caps refuse oversized/empty/duplicate specs fail-closed
//!   (`invalid_argument`) BEFORE any seam work;
//! - both RPCs are gated by the auth interceptor (deny-all refuses them);
//! - scoring is ADVISORY end to end: it registers no run (the view holds no
//!   submitter by construction — `ListRuns` stays empty after a score).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use kx_gateway::start;
use kx_proto::proto;
use tonic::Code;

fn read_file_request() -> proto::ScoreTaskBundleRequest {
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

#[tokio::test]
async fn list_tool_manifests_enumerates_the_builtins() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    let manifests = c
        .list_tool_manifests(proto::ListToolManifestsRequest {})
        .await
        .unwrap()
        .into_inner()
        .manifests;

    // `mcp-echo/echo` registers only when the bundled bin is present (env-dependent — in
    // CI's inference-checks it is absent), so filter it out and assert the DETERMINISTIC
    // registry builtins. RC4b: the read-only `retrieve@1` tool is seeded whenever datasets
    // are available (a `serve-engine` + `hnsw` build), so it joins the OSS builtins in the
    // advisory discovery surface — sorted between `fs-write` and `text-summarize`.
    let ids: Vec<(&str, &str)> = manifests
        .iter()
        .map(|m| (m.tool_id.as_str(), m.tool_version.as_str()))
        .filter(|(id, _)| !id.starts_with("mcp-echo"))
        .collect();
    #[cfg(all(feature = "serve-engine", feature = "hnsw"))]
    let expected = vec![
        ("fs-read", "1"),
        ("fs-write", "1"),
        ("retrieve", "1"),
        ("text-summarize", "1"),
    ];
    #[cfg(not(all(feature = "serve-engine", feature = "hnsw")))]
    let expected = vec![("fs-read", "1"), ("fs-write", "1"), ("text-summarize", "1")];
    assert_eq!(
        ids, expected,
        "the OSS builtins, in deterministic (tool_id, tool_version) order"
    );
    // (skip the env-dependent `mcp-echo/echo` — it is kind `Mcp`, not `Builtin`).
    for m in manifests
        .iter()
        .filter(|m| !m.tool_id.starts_with("mcp-echo"))
    {
        assert_eq!(m.kind, "Builtin");
        assert!(
            !m.description.is_empty(),
            "{}: description present",
            m.tool_id
        );
        assert_eq!(
            m.fingerprint_hash.len(),
            32,
            "{}: 32B fingerprint",
            m.tool_id
        );
        assert_eq!(m.keywords.len(), 1, "{}: one 'en' keyword set", m.tool_id);
        assert_eq!(m.keywords[0].lang, "en");
        assert!(!m.keywords[0].words.is_empty());
    }
}

#[tokio::test]
async fn score_task_bundle_ranks_deterministically_and_stays_advisory() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    let first = c
        .score_task_bundle(read_file_request())
        .await
        .unwrap()
        .into_inner();

    // Every manifest ranked, best first; the exact intent keywords ("read",
    // "file", "disk") hit fs-read's curated keyword set at the 10 000 ceiling.
    // Filter the env-dependent `mcp-echo/echo`; RC4b: +1 (retrieve@1) on a serve-engine +
    // hnsw build (see the manifests test).
    let ranked_builtins = first
        .ranked
        .iter()
        .filter(|r| !r.tool_id.starts_with("mcp-echo"))
        .count();
    #[cfg(all(feature = "serve-engine", feature = "hnsw"))]
    let expected_count = 4;
    #[cfg(not(all(feature = "serve-engine", feature = "hnsw")))]
    let expected_count = 3;
    assert_eq!(
        ranked_builtins, expected_count,
        "every registered manifest is ranked"
    );
    assert_eq!(first.ranked[0].tool_id, "fs-read");
    assert_eq!(first.ranked[0].score_bp, 10_000);
    assert!(
        first
            .ranked
            .windows(2)
            .all(|w| w[0].score_bp >= w[1].score_bp),
        "best-first ordering"
    );
    assert_eq!(first.bundle_fingerprint.len(), 32);

    // The default (FFI-free) build has no react runtime → the lowering verdict
    // degrades honestly to UNAVAILABLE, and the detail says why.
    assert_eq!(first.verdict, i32::from(proto::LowerVerdict::Unavailable));
    assert!(!first.verdict_detail.is_empty());

    // Deterministic: the same spec scores byte-identically.
    let second = c
        .score_task_bundle(read_file_request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.bundle_fingerprint, second.bundle_fingerprint);
    let pairs = |r: &[proto::ManifestScore]| -> Vec<(String, u32)> {
        r.iter().map(|s| (s.tool_id.clone(), s.score_bp)).collect()
    };
    assert_eq!(pairs(&first.ranked), pairs(&second.ranked));

    // GR10 spike (never a gate): the per-call RPC latency over a warm channel —
    // run with `--nocapture` to read it; persisted to the private benchmarks
    // baseline per the profiling rule.
    let mut samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let t0 = std::time::Instant::now();
        let _ = c.score_task_bundle(read_file_request()).await.unwrap();
        samples.push(t0.elapsed());
    }
    samples.sort();
    eprintln!(
        "GR10 ScoreTaskBundle (3 manifests, 1-tool spec, debug build): p50 {:?} · p99 {:?}",
        samples[24], samples[49]
    );

    // ADVISORY end to end: no run was registered by scoring (the view holds no
    // submitter by construction; this pins it observably).
    let runs = c
        .list_runs(proto::ListRunsRequest {
            limit: None,
            before_seq: None,
        })
        .await
        .unwrap()
        .into_inner()
        .runs;
    assert!(
        runs.is_empty(),
        "ScoreTaskBundle leaves no run/journal trace"
    );
}

#[tokio::test]
async fn the_request_caps_refuse_bad_specs_fail_closed() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    let mut empty_intent = read_file_request();
    empty_intent.intent = String::new();
    let mut oversized_intent = read_file_request();
    oversized_intent.intent = "x".repeat(5_000);
    let mut no_tools = read_file_request();
    no_tools.tool_sequence.clear();
    let mut dup_names = read_file_request();
    dup_names.tool_sequence.push(proto::BundleToolSpec {
        tool_id: "fs-read".to_string(),
        tool_version: "2".to_string(),
        description: String::new(),
        keywords: vec![],
    });
    let mut bad_threshold = read_file_request();
    bad_threshold.tolerance_threshold_bp = 10_001;

    for (name, req) in [
        ("empty intent", empty_intent),
        ("oversized intent", oversized_intent),
        ("empty tool_sequence", no_tools),
        ("duplicate tool names", dup_names),
        ("threshold above 10000", bad_threshold),
    ] {
        let err = c.score_task_bundle(req).await.expect_err(name);
        assert_eq!(err.code(), Code::InvalidArgument, "{name}");
    }
}

#[tokio::test]
async fn toolscout_rpcs_are_gated_by_auth_under_deny_all() {
    let dir = tempfile::TempDir::new().unwrap();
    // Neither --dev-allow-local nor tokens: the deny-all resolver gates everything.
    let running = start(common::gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    let list = c
        .list_tool_manifests(proto::ListToolManifestsRequest {})
        .await;
    assert_eq!(list.expect_err("denied").code(), Code::Unauthenticated);

    let score = c.score_task_bundle(read_file_request()).await;
    assert_eq!(score.expect_err("denied").code(), Code::Unauthenticated);
}
