//! `kx-executor-pure-body` — the example Mote body binary exercised by
//! PR 9a-hardening-2+'s integration tests. **NOT a production binary.**
//!
//! Contract:
//! - Reads input bytes from the file path passed in `argv[1]`.
//! - If `argv[2] == "--sleep"`, sleeps for `argv[3]` milliseconds AFTER
//!   reading the input + BEFORE writing the result. The PR 9a-hardening-4
//!   wall-clock integration test uses this to verify the parent-side
//!   `wall_clock_ms` watcher SIGKILLs the child after the budget elapses.
//! - Computes the `result_ref` as `BLAKE3("kx-executor-pure-body-result" || input_bytes)`.
//! - Writes the `result_ref` hex (64 ASCII chars, lowercase, no trailing newline)
//!   to stdout.
//! - Exits 0 on success; non-zero on any error (typically I/O).
//!
//! The body is purposefully tiny — it exists to prove the
//! `fork`+`sandbox_init`+`execvp` path end-to-end on Apple Silicon + the
//! `fork`+`execvp(bwrap)`+body path on Linux. Real PURE Motes wrap a model
//! inference call; the runtime promise is the same.

#![allow(clippy::print_stdout)] // body's output IS its stdout

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

const NIBBLES: &[u8; 16] = b"0123456789abcdef";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: pure_body <input-file>");
        return ExitCode::from(2);
    }

    let input_path = &args[1];
    let input_bytes = match fs::read(input_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pure_body: failed to read input {input_path}: {e}");
            return ExitCode::from(1);
        }
    };

    // Optional --sleep <ms> for wall-clock budget testing.
    if args.len() >= 4 && args[2] == "--sleep" {
        let Ok(sleep_ms) = args[3].parse::<u64>() else {
            eprintln!("pure_body: --sleep value must be a non-negative integer");
            return ExitCode::from(2);
        };
        thread::sleep(Duration::from_millis(sleep_ms));
    }

    // Result derivation — content-addressed: same input → same hex.
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kx-executor-pure-body-result");
    hasher.update(&input_bytes);
    let digest = hasher.finalize();

    // Hex-encode without external crates (kx-executor's example builds
    // with workspace deps + blake3 only).
    let mut hex = [0u8; 64];
    for (i, byte) in digest.as_bytes().iter().enumerate() {
        hex[i * 2] = NIBBLES[(byte >> 4) as usize];
        hex[i * 2 + 1] = NIBBLES[(byte & 0x0F) as usize];
    }

    let mut out = std::io::stdout().lock();
    if out.write_all(&hex).is_err() {
        return ExitCode::from(1);
    }
    if out.flush().is_err() {
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
