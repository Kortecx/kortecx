//! `kx-model-harness` binary — drives a selected workflow row through the real
//! `kx_runtime::run_with_seams` orchestrator with a real GGUF model behind the
//! executor + broker seams.
//!
//! ```text
//! kx-model-harness <run|replay|digest> --journal <path> --content <dir> \
//!     --gguf <path> --row <serve|greedy|sampled|tool> [--crash-at <point>] [--seed <n>]
//! ```
//!
//! Used by the Test C subprocess kill-and-replay: `run --crash-at post-commit-vtc`
//! aborts after the model Mote commits; a fresh `replay` recovers and must serve
//! the committed result (never re-sample). `digest` prints the journal digest.

use std::path::PathBuf;
use std::process::ExitCode;

use kx_model_harness::{harness_warrant, model_id_for, workflow_for_row, Harness};
use kx_runtime::config::Mode;
use kx_runtime::{digest_only, CrashPoint, RuntimeConfig};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mode = match args.next().as_deref() {
        Some("run") => Mode::Run,
        Some("replay") => Mode::Replay,
        Some("digest") => Mode::Digest,
        other => {
            eprintln!("kx-model-harness: bad mode {other:?} (run|replay|digest)");
            return ExitCode::from(2);
        }
    };

    let mut journal: Option<PathBuf> = None;
    let mut content: Option<PathBuf> = None;
    let mut gguf: Option<PathBuf> = None;
    let mut row = String::from("serve");
    let mut crash_at: Option<CrashPoint> = None;
    let mut seed: u32 = 7;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--journal" => journal = args.next().map(PathBuf::from),
            "--content" => content = args.next().map(PathBuf::from),
            "--gguf" => gguf = args.next().map(PathBuf::from),
            "--row" => row = args.next().unwrap_or(row),
            "--crash-at" => {
                if let Some(Ok(c)) = args.next().as_deref().map(str::parse::<CrashPoint>) {
                    crash_at = Some(c);
                } else {
                    eprintln!("kx-model-harness: bad --crash-at");
                    return ExitCode::from(2);
                }
            }
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(seed),
            other => {
                eprintln!("kx-model-harness: unknown flag {other:?}");
                return ExitCode::from(2);
            }
        }
    }

    let (Some(journal), Some(content)) = (journal, content) else {
        eprintln!("kx-model-harness: --journal and --content are required");
        return ExitCode::from(2);
    };

    let config = RuntimeConfig {
        journal_path: journal,
        content_root: content,
        mode,
        crash_at,
    };

    if mode == Mode::Digest {
        return match digest_only(&config) {
            Ok(d) => {
                println!("DIGEST={}", d.to_hex());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("kx-model-harness: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let gguf = gguf.unwrap_or_else(kx_model_harness::default_gguf_path);
    let model_id = match model_id_for(&gguf) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("kx-model-harness: cannot hash GGUF {}: {e}", gguf.display());
            return ExitCode::FAILURE;
        }
    };
    let warrant = harness_warrant(&model_id, 64, 60_000);
    let Some(workflow) = workflow_for_row(&row, &model_id, &warrant, seed) else {
        eprintln!("kx-model-harness: unknown --row {row:?}");
        return ExitCode::from(2);
    };

    let harness = match Harness::open(&config, &gguf, model_id) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("kx-model-harness: open failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    match harness.drive(&config, &workflow) {
        Ok(outcome) => {
            println!("DIGEST={}", outcome.digest.to_hex());
            println!("COMMITTED={}/{}", outcome.committed, outcome.total);
            println!("CALLS={}", harness.backend.calls());
            println!("DISPATCHES={}", harness.observer.dispatches());
            ExitCode::SUCCESS
        }
        Err(e) => {
            // Fail-closed stalls (e.g. a withheld consumer) are surfaced, not hidden.
            eprintln!("kx-model-harness: drive ended: {e}");
            println!("CALLS={}", harness.backend.calls());
            ExitCode::FAILURE
        }
    }
}
