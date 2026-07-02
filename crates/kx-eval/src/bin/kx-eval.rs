//! `kx-eval` — the deterministic eval gate (`just eval`).
//!
//! Scores the embedded `golden-v1` corpus (Tier A — scripted transcripts, no model) and
//! compares the aggregate Gate metrics to the committed baseline
//! (`corpus/golden-v1/baseline.json`). Exits non-zero on any regression or corpus drift.
//! `--update-baseline` re-captures the committed baseline from the current scorers (the
//! "before" snapshot RC1 commits; later RCs raise it in-PR).

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kx_eval::{compare_to_baseline, embedded_baseline, score_golden_v1, Baseline, EvalReport};

/// The in-source baseline path (`--update-baseline` writes here; the gate otherwise uses
/// the embedded copy so it runs from an installed binary too).
fn source_baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/golden-v1/baseline.json")
}

struct Args {
    baseline: Option<PathBuf>, // None ⇒ the embedded committed baseline.
    update: bool,
    json: bool,
    tolerance: u32,
    // RC-SW1: score ONE capability family (report-only iteration; the committed
    // baseline stays the aggregate gate — --suite + --update-baseline is refused).
    suite: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        baseline: None,
        update: false,
        json: false,
        tolerance: 0,
        suite: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "run" => {}
            "--update-baseline" => a.update = true,
            "--json" => a.json = true,
            "--baseline" => {
                a.baseline = Some(
                    it.next()
                        .map(PathBuf::from)
                        .ok_or("--baseline needs a path")?,
                );
            }
            "--suite" => {
                a.suite = Some(it.next().ok_or("--suite needs a family name")?);
            }
            "--tolerance" => {
                let v = it.next().ok_or("--tolerance needs a number")?;
                a.tolerance = v.parse().map_err(|_| format!("invalid tolerance: {v}"))?;
            }
            "-h" | "--help" => return Err(
                "usage: kx-eval run [--suite <family>] [--baseline <path>] [--update-baseline] \
                     [--json] [--tolerance <per_mille>]"
                    .to_string(),
            ),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(a)
}

fn build_report() -> Result<EvalReport, String> {
    score_golden_v1(env_label(), git_sha()).map_err(|e| format!("scoring failed: {e}"))
}

/// `git rev-parse HEAD`, or `"unknown"` (the eval gate does not require a repo).
fn git_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// A lightweight environment label (GR10: every recorded number carries a label).
fn env_label() -> String {
    let cores = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    format!(
        "{}/{} ({cores} cores)",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn load_baseline(path: &Path) -> Result<Baseline, String> {
    let s = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "read baseline {} (run `kx-eval run --update-baseline` first?): {e}",
            path.display()
        )
    })?;
    serde_json::from_str(&s).map_err(|e| format!("parse baseline {}: {e}", path.display()))
}

fn print_gates(report: &EvalReport) {
    let short = report
        .suite_digest
        .get(..16)
        .unwrap_or(report.suite_digest.as_str());
    println!("eval suite '{}' (digest {short}…)", report.suite_id);
    for g in &report.gates {
        println!("  {:<18} {:>4} / 1000", g.id, g.per_mille);
    }
}

fn run() -> Result<bool, String> {
    let args = parse_args()?;
    if args.suite.is_some() && args.update {
        return Err(
            "--suite is report-only; the committed baseline is aggregate-only \
                    (drop --update-baseline)"
                .to_string(),
        );
    }
    let report = match &args.suite {
        Some(family) => kx_eval::score_golden_v1_family(family, env_label(), git_sha())
            .map_err(|e| format!("scoring failed: {e}"))?,
        None => build_report()?,
    };

    if args.json {
        let json = report
            .to_json()
            .map_err(|e| format!("serialize report: {e}"))?;
        println!("{json}");
    }

    if args.update {
        let path = args.baseline.clone().unwrap_or_else(source_baseline_path);
        let baseline = report.to_baseline();
        let json = serde_json::to_string_pretty(&baseline)
            .map_err(|e| format!("serialize baseline: {e}"))?;
        std::fs::write(&path, format!("{json}\n"))
            .map_err(|e| format!("write baseline {}: {e}", path.display()))?;
        print_gates(&report);
        println!("\nbaseline updated: {}", path.display());
        return Ok(true);
    }

    if args.suite.is_some() && args.baseline.is_none() {
        // Family iteration: print the gates, no baseline compare (the aggregate
        // baseline covers a different task set — comparing would be dishonest).
        print_gates(&report);
        println!("\neval: report-only (--suite; no baseline compare)");
        return Ok(true);
    }
    let baseline = match &args.baseline {
        Some(path) => load_baseline(path)?,
        None => embedded_baseline().map_err(|e| e.to_string())?,
    };
    let cmp = compare_to_baseline(&report, &baseline, args.tolerance).map_err(|e| e.to_string())?;
    print_gates(&report);
    if cmp.ok {
        println!(
            "\neval: PASS — all {} gate(s) >= baseline (tolerance {} per-mille)",
            baseline.gates.len(),
            args.tolerance
        );
    } else {
        println!("\neval: FAIL — {} regression(s):", cmp.regressions.len());
        for r in &cmp.regressions {
            println!(
                "  - {}: {} < baseline {}",
                r.metric_id, r.current_per_mille, r.baseline_per_mille
            );
        }
    }
    Ok(cmp.ok)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(msg) => {
            eprintln!("kx-eval: {msg}");
            ExitCode::from(2)
        }
    }
}
