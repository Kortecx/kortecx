//! The `--json` contract, asserted verb by verb.
//!
//! `docs/site/docs/scripts.md` tells the reader that every read-only client verb
//! answers `--json` with machine-readable output, so the runtime scripts into a
//! pipeline. Nothing proved it: the flag is parsed per verb (the CLI's argument
//! handling is hand-rolled), so a verb could accept `--json` and print a human
//! table, or drop the flag entirely, and every other gate would stay green.
//!
//! Three shapes are asserted here, because the runtime honestly has three:
//!
//! | shape | verbs | contract |
//! |---|---|---|
//! | document | the read verbs below | exit 0 and stdout is ONE JSON value |
//! | newline-delimited | `events` | every non-blank stdout line is a JSON value |
//! | capability-gated | `datasets` | refuses naming `hnsw` without it, answers JSON with it |
//!
//! The third row is the load-bearing one. A gateway built without `hnsw` has no
//! dataset plane, and the honest answer is a refusal that says which build flag
//! is missing — NOT an empty `{"datasets":[]}`, which would read as "you have no
//! datasets" when the truth is "this binary cannot have any". Both arms are
//! asserted, so neither feature set can drift into the other's behaviour.
//!
//! `kx memory` is deliberately NOT here. Its availability is not a feature
//! predicate — it needs `inference,hnsw` AND `KX_SERVE_MEMORY=1` — so a cfg-keyed
//! assertion would be wrong in one of the four combinations. It belongs in a test
//! that owns the env axis too.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, run_kx, start_gateway, stderr, stdout};
use tempfile::TempDir;

/// Read-only client verbs that answer `--json` with a single JSON document on a
/// DEFAULT-feature gateway. Adding a read verb to the CLI means adding a row.
const JSON_DOCUMENT_VERBS: &[&[&str]] = &[
    &["info"],
    &["health"],
    &["runs", "list"],
    &["recipe", "list"],
    &["tools", "list"],
    &["connections", "list"],
    &["skills", "list"],
    &["secrets", "list"],
    &["triggers", "list"],
    &["context", "list"],
    &["app", "list"],
    &["branch", "list"],
    &["models", "list"],
    &["feedback", "list"],
    &["replan", "list"],
    &["react", "list"],
    &["rerank", "list"],
    &["capture", "list"],
    &["approvals", "list"],
    &["signatures", "list"],
];

/// The observability read verbs succeed only when the gateway build carries the
/// `observability` feature (W6.1); without it the server answers the seam's
/// designed `unimplemented`, which the CLI surfaces as a non-zero exit.
#[cfg(feature = "observability")]
const OBSERVABILITY_JSON_VERBS: &[&[&str]] = &[
    &["telemetry", "list"],
    &["telemetry", "summary"],
    &["alerts", "list"],
];

/// The dataset plane is a build feature; `hnsw` is the flag its refusal must name.
const DATASETS_LIST: &[&str] = &["datasets", "list"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_read_verb_answers_json_with_one_document() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    #[cfg(feature = "observability")]
    let verbs: Vec<&[&str]> = [JSON_DOCUMENT_VERBS, OBSERVABILITY_JSON_VERBS].concat();
    #[cfg(not(feature = "observability"))]
    let verbs: Vec<&[&str]> = JSON_DOCUMENT_VERBS.to_vec();
    for verb in verbs {
        let mut args: Vec<&str> = verb.to_vec();
        args.extend_from_slice(&["--endpoint", &ep, "--json"]);
        let out = run_kx(argv(&args)).await;
        let label = verb.join(" ");

        assert!(
            out.status.success(),
            "`kx {label} --json` failed: {}",
            stderr(&out)
        );
        let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&out.stdout);
        assert!(
            parsed.is_ok(),
            "`kx {label} --json` did not print one JSON value: {}",
            stdout(&out)
        );
    }
}

/// `events` is a TAIL, so its `--json` is newline-delimited: one JSON value per
/// line, emitted as facts land. A run is driven first, because zero events would
/// make an empty stdout pass whatever the code did.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_event_tail_answers_json_line_by_line() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let inv = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"json-contract"}"#,
        "--endpoint",
        &ep,
        "--json",
        "--wait",
    ]))
    .await;
    assert!(inv.status.success(), "invoke failed: {}", stderr(&inv));

    let out = run_kx(argv(&[
        "events",
        "--all",
        "--since",
        "0",
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    assert!(
        out.status.success(),
        "`kx events --all --json` failed: {}",
        stderr(&out)
    );

    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "the tail emitted nothing after a committed run — an empty stream would \
         make this assertion vacuous"
    );
    for line in lines {
        assert!(
            serde_json::from_str::<serde_json::Value>(line).is_ok(),
            "a tail line was not JSON: {line}"
        );
    }
}

/// Without `hnsw` there is no dataset plane, and the refusal must name the flag.
/// An empty success would claim "you have no datasets" when the truth is "this
/// binary cannot have any".
#[cfg(not(feature = "hnsw"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn datasets_refuses_and_names_hnsw_when_the_build_lacks_it() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let mut args: Vec<&str> = DATASETS_LIST.to_vec();
    args.extend_from_slice(&["--endpoint", &ep, "--json"]);
    let out = run_kx(argv(&args)).await;

    assert!(
        !out.status.success(),
        "`kx datasets list --json` SUCCEEDED on a build without `hnsw` — an \
         unavailable capability must refuse, not answer emptily: {}",
        stdout(&out)
    );
    let msg = stderr(&out);
    assert!(
        msg.contains("hnsw"),
        "the refusal did not name the `hnsw` build flag: {msg}"
    );
}

/// With `hnsw` the same verb is an ordinary JSON read — the other half of the
/// contract, so the refusal above cannot quietly become permanent.
#[cfg(feature = "hnsw")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn datasets_answers_json_when_the_build_carries_hnsw() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let mut args: Vec<&str> = DATASETS_LIST.to_vec();
    args.extend_from_slice(&["--endpoint", &ep, "--json"]);
    let out = run_kx(argv(&args)).await;

    assert!(
        out.status.success(),
        "`kx datasets list --json` failed on an `hnsw` build: {}",
        stderr(&out)
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).is_ok(),
        "`kx datasets list --json` did not print one JSON value: {}",
        stdout(&out)
    );
}
