//! `kx-llamacpp/build.rs` — optional GGUF download for the model smoke test.
//!
//! When the `model-smoke-test` feature is **disabled** (the default), this
//! script does nothing. The crate compiles without network access.
//!
//! When the feature is **enabled**, this script downloads a tiny GGUF
//! (stories260K, ~1.2 MB) into `OUT_DIR`, verifies its SHA-256, and emits
//! `cargo:rustc-env=KX_LLAMACPP_SMOKE_TEST_MODEL=<path>` so the integration
//! test in `tests/smoke.rs` can `include_str!` / `env!` the path.
//!
//! The file is cached in `OUT_DIR` across builds; subsequent invocations
//! reuse the on-disk copy as long as the SHA matches.

// TODO(workspace.lints cleanup): see same-named TODO in kx-llamacpp-sys/build.rs.
// Build-time env-var unwraps are the appropriate failure mode for malformed
// cargo invocations; the workspace policy migrates this to `deny` with proper
// `expect(...)` messages in a follow-up.
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

/// The model URL — a 1.18 MB tinystories model in GGUF v3 format.
///
/// Hosted by ggml-org/models. This is the canonical tiny-GGUF location and
/// has been stable since the GGUF v3 transition.
const MODEL_URL: &str =
    "https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories260K.gguf";

/// SHA-256 of the model file. Computed locally + committed.
/// If the upstream file changes, the download will fail loudly.
const MODEL_SHA256: &str = "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d";

/// The output filename inside `OUT_DIR`.
const MODEL_FILENAME: &str = "stories260K.gguf";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_MODEL_SMOKE_TEST");

    if env::var("CARGO_FEATURE_MODEL_SMOKE_TEST").is_err() {
        // Feature disabled — nothing to do.
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let model_path = out_dir.join(MODEL_FILENAME);

    if model_path.exists() {
        // Verify the cached copy matches the expected SHA. If it does, reuse.
        // If it doesn't, re-download (cache poisoning / partial-write recovery).
        if verify_sha(&model_path, MODEL_SHA256).is_ok() {
            println!(
                "cargo:rustc-env=KX_LLAMACPP_SMOKE_TEST_MODEL={}",
                model_path.display()
            );
            return;
        }
        let _ = fs::remove_file(&model_path);
    }

    download_and_verify(MODEL_URL, &model_path, MODEL_SHA256);

    println!(
        "cargo:rustc-env=KX_LLAMACPP_SMOKE_TEST_MODEL={}",
        model_path.display()
    );
}

fn download_and_verify(url: &str, dest: &PathBuf, expected_sha: &str) {
    eprintln!(
        "kx-llamacpp build.rs: downloading {url} → {}",
        dest.display()
    );

    // ureq with default TLS handles the redirects HF serves.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(300))
        .build();

    let resp = agent
        .get(url)
        .call()
        .unwrap_or_else(|e| panic!("HTTP GET failed for {url}: {e}"));

    let mut reader = resp.into_reader();
    let mut bytes = Vec::with_capacity(2 * 1024 * 1024);
    reader
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("failed to read GGUF body from {url}: {e}"));

    eprintln!(
        "kx-llamacpp build.rs: downloaded {} bytes, verifying SHA-256",
        bytes.len()
    );

    // Verify SHA-256 BEFORE writing — never trust a download.
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let actual = hex_encode(&hasher.finalize());
    assert_eq!(
        actual, expected_sha,
        "SHA-256 mismatch for {url}: expected {expected_sha}, got {actual}. \
         If the upstream file was updated, recompute the SHA and update build.rs."
    );

    // Write to a temporary path then rename — POSIX atomic-on-completion.
    let tmp = dest.with_extension("partial");
    {
        let mut f = fs::File::create(&tmp)
            .unwrap_or_else(|e| panic!("failed to create {}: {e}", tmp.display()));
        f.write_all(&bytes)
            .unwrap_or_else(|e| panic!("failed to write {}: {e}", tmp.display()));
        f.sync_all().ok();
    }
    fs::rename(&tmp, dest).unwrap_or_else(|e| {
        panic!(
            "failed to rename {} → {}: {e}",
            tmp.display(),
            dest.display()
        )
    });
}

fn verify_sha(path: &PathBuf, expected: &str) -> Result<(), String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&buf);
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
