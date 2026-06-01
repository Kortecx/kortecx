//! `kx-runtime` — the single-node kortecx runtime binary.
//!
//! ```text
//! kx-runtime run     --journal <path> --content <dir> [--crash-at <pre-commit-stc|post-commit-vtc>] [--checkpoint-every <N>]
//! kx-runtime replay  --journal <path> --content <dir> [--checkpoint-every <N>]
//! kx-runtime digest  --journal <path> --content <dir>
//! ```
//!
//! `run` drives the canonical demo workflow from scratch; `--crash-at` injects
//! a hard `process::abort` at the named window. `replay` recovers from an
//! existing journal and finishes the run. `digest` prints the deterministic
//! projection digest of the on-disk journal (the cross-process comparison
//! surface for the kill-and-replay proof). `--checkpoint-every N` sets the
//! discardable-checkpoint cadence (`0` disables; default 256). All thin: parse →
//! call the library.

use std::process::ExitCode;

use kx_runtime::config::Mode;
use kx_runtime::{engine, RuntimeConfig};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init()
        .ok();

    let config = match RuntimeConfig::from_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kx-runtime: {e}");
            eprintln!(
                "usage: kx-runtime <run|replay|digest> --journal <path> --content <dir> \
                 [--crash-at <pre-commit-stc|post-commit-vtc>] [--checkpoint-every <N>]"
            );
            return ExitCode::from(2);
        }
    };

    match config.mode {
        Mode::Digest => match engine::digest_only(&config) {
            Ok(d) => {
                println!("{}", d.to_hex());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("kx-runtime: {e}");
                ExitCode::FAILURE
            }
        },
        Mode::Run | Mode::Replay => match engine::run(&config) {
            Ok(outcome) => {
                println!(
                    "{} ({}/{} committed)",
                    outcome.digest.to_hex(),
                    outcome.committed,
                    outcome.total
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("kx-runtime: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
