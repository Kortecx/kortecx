// SPDX-License-Identifier: Apache-2.0
//! `just test-skill [pack-dir…]` — run the RC-SW1 DECLARATIVE-family gate over
//! `kortecx.skill/v1` packs, printing a per-check report (and the
//! machine-readable JSON) and exiting non-zero on any failure.
//!
//! Usage:
//!   cargo run -p kx-extension-sdk --example skill_conformance             # the in-tree reference packs
//!   cargo run -p kx-extension-sdk --example skill_conformance -- skills/email-triage
//!   cargo run -p kx-extension-sdk --example skill_conformance -- path/to/my-skill
//!
//! External skill authors run this against their own pack before submission —
//! the same report CI gates the in-tree `skills/**` packs with.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::doc_markdown
)]

use std::path::PathBuf;

use kx_extension_sdk::skill_conformance::run_skill_conformance;

/// The in-tree reference packs (the no-args default). Resolved relative to the
/// workspace root when run via `cargo run` (CARGO_MANIFEST_DIR/../..).
fn reference_packs() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("skills");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut packs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    packs.sort();
    packs
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let packs: Vec<PathBuf> = if args.is_empty() {
        let found = reference_packs();
        if found.is_empty() {
            eprintln!(
                "no pack dirs given and no in-tree skills/ directory found.\n\
                 usage: cargo run -p kx-extension-sdk --example skill_conformance -- <pack-dir>…"
            );
            std::process::exit(2);
        }
        found
    } else {
        args.into_iter().map(PathBuf::from).collect()
    };

    let mut all_passed = true;
    for pack in &packs {
        let report = run_skill_conformance(pack);
        println!(
            "skill {:?} — {} wished tool(s) — {}",
            report.connector,
            report.discovered,
            if report.passed() { "PASS" } else { "FAIL" }
        );
        for c in &report.checks {
            println!(
                "  [{}] {:<26} (gate {}) {}",
                if c.passed { "ok" } else { "XX" },
                c.name,
                c.gate_item,
                c.detail
            );
        }
        println!("{}", serde_json::to_string(&report).unwrap());
        all_passed &= report.passed();
    }
    if !all_passed {
        std::process::exit(1);
    }
}
