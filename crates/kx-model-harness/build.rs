//! `kx-model-harness/build.rs` — optional GGUF download for the real-model tests.
//!
//! When the `with-model` feature is **disabled** (the default), this script does
//! nothing and the crate compiles + tests without network access.
//!
//! When **enabled**, it ensures the pinned Qwen2.5-0.5B-Instruct Q4_K_M GGUF is
//! present (downloading + SHA-256-verifying if missing) and emits
//! `cargo:rustc-env=KX_MODEL_HARNESS_GGUF=<path>`. The default location is
//! `<workspace>/target/models/<file>`; an explicit `KX_MODEL_HARNESS_GGUF` env
//! var overrides it (so a pre-downloaded copy is reused — no re-download).
//!
//! Mirrors `crates/kx-llamacpp/build.rs` (the established GGUF-download pattern).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements,
    clippy::doc_markdown
)]

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

/// Official Qwen GGUF (Q4_K_M). Pinned; if upstream changes the SHA check fails.
const MODEL_URL: &str =
    "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf";

/// SHA-256 of the pinned GGUF (computed locally on download, 2026-05-31).
const MODEL_SHA256: &str = "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db";

/// File name under `<workspace>/target/models/`.
const MODEL_FILENAME: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_WITH_MODEL");
    println!("cargo:rerun-if-env-changed=KX_MODEL_HARNESS_GGUF");

    if env::var("CARGO_FEATURE_WITH_MODEL").is_err() {
        // Feature disabled — nothing to do (crate compiles network-free).
        return;
    }

    let model_path = resolve_path();

    if model_path.exists() {
        if verify_sha(&model_path, MODEL_SHA256).is_ok() {
            println!(
                "cargo:rustc-env=KX_MODEL_HARNESS_GGUF={}",
                model_path.display()
            );
            return;
        }
        let _ = fs::remove_file(&model_path);
    }

    if let Some(parent) = model_path.parent() {
        fs::create_dir_all(parent).expect("create model dir");
    }
    download_and_verify(MODEL_URL, &model_path, MODEL_SHA256);

    println!(
        "cargo:rustc-env=KX_MODEL_HARNESS_GGUF={}",
        model_path.display()
    );
}

/// Resolve the GGUF path: explicit override, else `<workspace>/target/models/`.
fn resolve_path() -> PathBuf {
    if let Ok(p) = env::var("KX_MODEL_HARNESS_GGUF") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    manifest
        .join("../../target/models")
        .join(MODEL_FILENAME)
}

fn download_and_verify(url: &str, dest: &PathBuf, expected_sha: &str) {
    eprintln!("kx-model-harness build.rs: downloading {url} → {}", dest.display());
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(600))
        .build();
    let resp = agent
        .get(url)
        .call()
        .unwrap_or_else(|e| panic!("HTTP GET failed for {url}: {e}"));
    let mut reader = resp.into_reader();
    let mut bytes = Vec::with_capacity(512 * 1024 * 1024);
    reader
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("failed to read GGUF body from {url}: {e}"));

    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let actual = hex_encode(&hasher.finalize());
    assert_eq!(
        actual, expected_sha,
        "SHA-256 mismatch for {url}: expected {expected_sha}, got {actual}"
    );

    let tmp = dest.with_extension("partial");
    {
        let mut f = fs::File::create(&tmp)
            .unwrap_or_else(|e| panic!("failed to create {}: {e}", tmp.display()));
        f.write_all(&bytes)
            .unwrap_or_else(|e| panic!("failed to write {}: {e}", tmp.display()));
        f.sync_all().ok();
    }
    fs::rename(&tmp, dest)
        .unwrap_or_else(|e| panic!("failed to rename {} → {}: {e}", tmp.display(), dest.display()));
}

fn verify_sha(path: &PathBuf, expected: &str) -> Result<(), String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got = hex_encode(&hasher.finalize());
    if got == expected {
        Ok(())
    } else {
        Err(format!("expected {expected}, got {got}"))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}
