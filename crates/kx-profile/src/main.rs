//! `kx-profile` — capture an environment-labelled runtime profile (Golden Rule
//! 10).
//!
//! Hosts a fresh in-process gateway per iteration, measures warm-up
//! (`start` → health `SERVING`) and submit→Committed latency over the FFI-free
//! echo demo, and writes a schema-1 JSON [`Report`] to `target/profile/`
//! (gitignored). `just profile` runs this + re-runs the existing scale/ceiling
//! spikes; the captured JSON is then copied into the PRIVATE
//! `docs/benchmarks/` trend record (never committed to OSS — SN-2).
//!
//! Usage: `kx-profile [--iterations N] [--out PATH]` (default `N = 8`).

use std::path::PathBuf;

use kx_profile::{
    capture_git_sha, percentile, react_spikes, spikes, Environment, Metric, ProfileError, Report,
};

#[tokio::main]
async fn main() {
    // Surface the gateway-under-measurement's tracing (best-effort; ignore an
    // already-installed global subscriber).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    if let Err(e) = run().await {
        eprintln!("kx-profile: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ProfileError> {
    let args = Args::parse(std::env::args().skip(1));

    // Capture the load-bearing labels FIRST — abort before any work if the
    // environment can't be fully described (Golden Rule 10).
    let git_sha = capture_git_sha()?;
    let env = Environment::capture()?;
    eprintln!(
        "kx-profile: profiling {sha} on {host} ({cores} cores, {os}/{arch}) — {n} iteration(s)",
        sha = short_sha(&git_sha),
        host = env.host,
        cores = env.cores,
        os = env.os,
        arch = env.arch,
        n = args.iterations,
    );

    let samples = spikes::measure(args.iterations).await?;
    let mut metrics = vec![
        Metric::spike(
            "warmup_to_serving_p50",
            percentile(&samples.warmup_ms, 50),
            "ms",
        ),
        Metric::spike(
            "submit_to_committed_p50",
            percentile(&samples.submit_ms, 50),
            "ms",
        ),
        Metric::spike(
            "submit_to_committed_p99",
            percentile(&samples.submit_ms, 99),
            "ms",
        ),
    ];

    // PR-2d-2 — M7a/M7b: the live react chain's settle machinery, model-free at
    // the coordinator layer (M7b fires the REAL bundled stdio tool; skipped —
    // empty samples — when the bin is absent).
    let react = react_spikes::measure(args.iterations).await?;
    metrics.push(Metric::spike(
        "react_answer_settle_p50",
        percentile(&react.answer_ms, 50),
        "ms",
    ));
    metrics.push(Metric::spike(
        "react_answer_settle_p99",
        percentile(&react.answer_ms, 99),
        "ms",
    ));
    if !react.tool_round_ms.is_empty() {
        metrics.push(Metric::spike(
            "react_tool_round_p50",
            percentile(&react.tool_round_ms, 50),
            "ms",
        ));
        metrics.push(Metric::spike(
            "react_tool_round_p99",
            percentile(&react.tool_round_ms, 99),
            "ms",
        ));
    }

    let report = Report::new(git_sha.clone(), env, metrics);
    let json = report
        .to_json()
        .map_err(|e| ProfileError::Report(e.to_string()))?;

    let out = args.out.unwrap_or_else(|| default_out(&git_sha));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ProfileError::Report(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&out, format!("{json}\n"))
        .map_err(|e| ProfileError::Report(format!("write {}: {e}", out.display())))?;
    eprintln!("kx-profile: wrote {}", out.display());

    // The JSON also goes to stdout so `just profile` can tee it into the
    // private trend record.
    println!("{json}");
    Ok(())
}

/// Parsed CLI arguments (hand-rolled, no clap — the FFI-free `kx` convention).
struct Args {
    iterations: usize,
    out: Option<PathBuf>,
}

impl Args {
    fn parse(mut argv: impl Iterator<Item = String>) -> Self {
        let mut iterations = 8usize;
        let mut out = None;
        while let Some(flag) = argv.next() {
            match flag.as_str() {
                "--iterations" | "-n" => {
                    if let Some(v) = argv.next() {
                        if let Ok(n) = v.parse::<usize>() {
                            iterations = n.max(1);
                        } else {
                            eprintln!("kx-profile: invalid --iterations {v:?}; using {iterations}");
                        }
                    }
                }
                "--out" | "-o" => {
                    out = argv.next().map(PathBuf::from);
                }
                other => {
                    eprintln!("kx-profile: ignoring unrecognized argument {other:?}");
                }
            }
        }
        Self { iterations, out }
    }
}

/// The first 12 hex of a commit sha (for human log lines / file names).
fn short_sha(sha: &str) -> &str {
    let end = sha.len().min(12);
    &sha[..end]
}

/// Default output path: `target/profile/profile-<sha12>.json` (`target/` is
/// already gitignored).
fn default_out(git_sha: &str) -> PathBuf {
    PathBuf::from("target")
        .join("profile")
        .join(format!("profile-{}.json", short_sha(git_sha)))
}
