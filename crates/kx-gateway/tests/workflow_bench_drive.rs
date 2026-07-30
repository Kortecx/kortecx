//! The `workflow` bench family's MODEL-FREE drive: every task driven down the real
//! harness path (`drive_for` → `RunWorkflow` → `fold_workflow_transcript` →
//! `score_transcript`) against an embedded-worker serve and the hermetic routed
//! fixture — no model anywhere.
//!
//! This is the family's standing regression gate AND the dev drive that settles the
//! corpus's `ideal_turns` before capture: `turns_used` is asserted EQUAL to each
//! task's `ideal_turns` (the ancestor-closure size plus the terminal — skip commits
//! included), because `loop_efficiency` caps at 1000 and would silently forgive an
//! overprediction. A capture must never be the first time these numbers are checked.
//!
//! `serve-engine`-gated (the `eval_bench` module's own gate) but MODEL-FREE and
//! FFI-free — `just ci`'s default-feature pass misses it, so the `eval-bench` recipe
//! runs it as a preflight and the S-gate runs it explicitly.

#![cfg(feature = "serve-engine")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use kx_gateway::eval_bench::score_live_suite;
use kx_gateway::start;

const WORKFLOW_TASK_IDS: [&str; 7] = [
    "workflow-sequential-carry",
    "workflow-parallel-join",
    "workflow-conditional-high-reading",
    "workflow-conditional-low-reading",
    "workflow-wait-then-carry",
    "workflow-retry-recovers",
    "workflow-continue-placeholder",
];

#[tokio::test(flavor = "multi_thread")]
async fn the_workflow_family_scores_clean_and_its_ideal_turns_are_exact() {
    // The credential the sequential task's http step resolves BY NAME at dispatch
    // (the env value is the full header value; the step declares an empty scheme).
    std::env::set_var(
        common::bench_http::BENCH_HTTP_CRED_ENV,
        common::bench_http::BENCH_HTTP_BEARER,
    );
    // Hold the suite to the workflow family: this drive exists to attribute exactly
    // one family, and the other families need recipes/models this serve cannot run.
    std::env::set_var("KX_BENCH_ONLY", WORKFLOW_TASK_IDS.join(","));

    let routed = common::bench_routes::BenchRoutedServer::start();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = common::connect_client(running.local_addr()).await;

    assert!(
        common::bench_routes::provision_workflow_fixtures(&mut c, &routed.base_url()).await,
        "the seven workflow fixtures provision on a model-free serve"
    );

    let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
    let outcome = score_live_suite(
        &mut c,
        &corpus,
        "model-free-drive".into(),
        "dev".into(),
        Duration::from_secs(60),
    )
    .await
    .expect("the workflow family drives to completion");
    std::env::remove_var("KX_BENCH_ONLY");
    std::env::remove_var(common::bench_http::BENCH_HTTP_CRED_ENV);

    // Every workflow task ran (nothing skipped but the filtered-out remainder).
    let driven: Vec<&str> = outcome
        .transcripts
        .iter()
        .map(|t| t.task_id.as_str())
        .collect();
    for id in WORKFLOW_TASK_IDS {
        assert!(driven.contains(&id), "{id} was driven");
    }

    // The family gates: presence oracles all pass, the absence half of the
    // conditional pair holds, and the runtime spent exactly the ideal step count.
    let gate = |id: &str| {
        outcome
            .report
            .gates
            .iter()
            .find(|g| g.id == id)
            .unwrap_or_else(|| panic!("gate {id} is emitted"))
            .per_mille
    };
    assert_eq!(
        gate("task_success@workflow"),
        1000,
        "every oracle satisfied"
    );
    assert_eq!(
        gate("injection_resistance@workflow"),
        1000,
        "no untaken arm and no failure placeholder leaked into a terminal"
    );
    assert_eq!(gate("loop_efficiency@workflow"), 1000, "no wasted steps");

    // The exact-turns pin (see the module doc): committed ancestors + the terminal.
    for task in corpus.suite.tasks.iter().filter(|t| t.family == "workflow") {
        let transcript = outcome
            .transcripts
            .iter()
            .find(|t| t.task_id == task.id)
            .unwrap_or_else(|| panic!("{} has a transcript", task.id));
        assert_eq!(
            transcript.turns_used(),
            task.expect.ideal_turns,
            "{}: the corpus ideal_turns states the exact ancestor closure",
            task.id
        );
    }

    // The two sentinels' raw evidence, read the same way the harness reads it.
    let wait_ms = outcome
        .drive_wall_ms
        .get("workflow-wait-then-carry")
        .copied()
        .unwrap_or(0);
    assert!(
        wait_ms >= 3000,
        "the wait task's drive wall clock ({wait_ms}ms) shows the 3s hold"
    );
    let depot: Vec<_> = routed
        .captured()
        .into_iter()
        .filter(|call| call.path == "/depot")
        .collect();
    let distinct: std::collections::BTreeSet<String> = depot
        .iter()
        .filter_map(|call| call.idempotency_key.clone())
        .collect();
    assert!(
        distinct.len() >= 2 && depot.iter().any(|call| !call.refused),
        "the depot saw a FRESH attempt identity recover ({} dials, {} keys)",
        depot.len(),
        distinct.len()
    );

    running.shutdown().await.unwrap();
}
