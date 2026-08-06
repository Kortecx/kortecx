//! Nothing in this repository tells a reader to run `kx serve --features …`.
//!
//! `--features` is a **cargo** flag. `kx serve` has never accepted it, so every one of
//! these was a command that could only fail for the person who copied it — and they were
//! endemic: docs pages, doc comments, and two runtime error strings a user is shown at the
//! exact moment something is already going wrong.
//!
//! The distinction this checker has to make is that `--features` beside **cargo** is
//! correct and must keep working. So the rule is narrow: the offence is `--features`
//! appearing as an argument to the `kx serve` COMMAND. `cargo run … --features inference`
//! and prose about which features a build needs are both fine, and the positive control
//! below proves the checker still catches the offence while accepting them — a scanner
//! that has never been shown its own subject is not evidence that the subject is absent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

/// `kx serve` followed by anything on the same line up to `--features`. Deliberately not
/// anchored on `kx serve --features` alone: `kx serve --dev-allow-local --features x` is
/// the same broken command with a flag in between.
fn offends(line: &str) -> bool {
    let Some(after) = line.find("kx serve").map(|i| &line[i + "kx serve".len()..]) else {
        return false;
    };
    // Only what is still the SAME command. A command ends at a shell separator, and — the
    // case that actually matters here — an inline code span ends at its closing backtick
    // or tag, after which the words are prose. "Run `kx serve`, or build with
    // `--features inference`" is correct advice and must not be flagged, while
    // `kx serve --dev-allow-local --features hnsw` is the offence with a flag in between.
    let command_tail: &str = after
        .split(['`', ',', ';', '|', '<', '"'])
        .next()
        .unwrap_or(after)
        .split("&&")
        .next()
        .unwrap_or(after);
    command_tail.contains("--features")
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/kx-cli.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn scan(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Vendored, generated and build trees are not ours to police.
            if matches!(
                name.as_ref(),
                "target" | "node_modules" | ".git" | "dist" | "gen" | "v1"
            ) {
                continue;
            }
            scan(&path, out);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "md" | "rs" | "ts" | "tsx" | "py" | "sh" | "toml") {
            continue;
        }
        // This file quotes the offending shape on purpose, to describe and to test it.
        if path.ends_with("serve_commands_are_runnable.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if offends(line) {
                out.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
}

#[test]
fn the_checker_sees_the_offence_and_accepts_cargo() {
    // ⚠ THE POSITIVE CONTROL, first. Every assertion in the sweep below is an absence.
    assert!(
        offends("KX_SERVE_MEMORY=1 kx serve --features inference,hnsw --model gemma"),
        "the checker must catch the plain offence"
    );
    assert!(
        offends("kx serve --dev-allow-local --features inference"),
        "…including with a flag in between"
    );

    // …and the accepting controls, each differing from an offence in exactly one way.
    assert!(
        !offends("cargo run -p kx-cli --features inference,hnsw -- serve"),
        "`--features` belongs to cargo and must keep working"
    );
    assert!(
        !offends("Build with `--features inference`, then run `kx serve`."),
        "prose about which features a build needs is not a command"
    );
    assert!(
        !offends("cargo build --features inference && kx serve --dev-allow-local"),
        "a `kx serve` with no --features of its own is fine, whatever preceded it"
    );
    assert!(
        !offends("Run `kx serve`, or build with `--features inference` first."),
        "once the code span closes the words are prose, not arguments"
    );
    assert!(
        !offends("<code>kx serve</code> needs --features inference at build time"),
        "…and the same for a closing tag"
    );
    assert!(!offends("kx serve --dev-allow-local"), "the real command");
}

#[test]
fn no_source_or_doc_tells_a_reader_to_run_kx_serve_with_features() {
    let mut hits = Vec::new();
    scan(&repo_root(), &mut hits);
    assert!(
        hits.is_empty(),
        "`kx serve` does not accept `--features` (it is a cargo flag), so each of these \
         is a command that cannot run for whoever copies it. Use `kx serve` and state the \
         build requirement in prose, or show the full `cargo run … -- serve` form:\n{}",
        hits.join("\n")
    );
}
