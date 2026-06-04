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
use std::path::{Path, PathBuf};

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

// ---------------------------------------------------------------------------
// Multi-modal smoke model (PR-2): a small VLM + its vision projector.
//
// Qwen2-VL-2B-Instruct, the canonical ggml-org mtmd test VLM — modern, supported
// at the b9000 pin (models/qwen2vl.cpp), and exercises the M-RoPE image path the
// mtmd helper handles. ~940 MB LLM (Q4_K_M) + ~676 MB projector (Q8_0). Heavy:
// only fetched under the `model-smoke-test-multimodal` feature.
// ---------------------------------------------------------------------------
const VLM_GGUF_URL: &str =
    "https://huggingface.co/ggml-org/Qwen2-VL-2B-Instruct-GGUF/resolve/main/Qwen2-VL-2B-Instruct-Q4_K_M.gguf";
const VLM_GGUF_SHA256: &str = "5745685d2e607a82a0696c1118e56a2a1ae0901da450fd9cd4f161c6b62867d7";
const VLM_GGUF_FILENAME: &str = "Qwen2-VL-2B-Instruct-Q4_K_M.gguf";

const VLM_MMPROJ_URL: &str =
    "https://huggingface.co/ggml-org/Qwen2-VL-2B-Instruct-GGUF/resolve/main/mmproj-Qwen2-VL-2B-Instruct-Q8_0.gguf";
const VLM_MMPROJ_SHA256: &str = "a0ad91f00a7a80dcf84d719a61b00ee2e07b71794f4ee2dfa81a254621a8c418";
const VLM_MMPROJ_FILENAME: &str = "mmproj-Qwen2-VL-2B-Instruct-Q8_0.gguf";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_MODEL_SMOKE_TEST");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_MODEL_SMOKE_TEST_MULTIMODAL");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));

    // Text smoke model (stories260K, ~1.2 MB) — the existing pipeline.
    if env::var("CARGO_FEATURE_MODEL_SMOKE_TEST").is_ok() {
        let model_path = ensure_model(MODEL_URL, MODEL_FILENAME, MODEL_SHA256, 300, &out_dir);
        println!(
            "cargo:rustc-env=KX_LLAMACPP_SMOKE_TEST_MODEL={}",
            model_path.display()
        );
    }

    // Multi-modal smoke model (VLM + projector, ~1.6 GB) — the IMAGE pipeline.
    // Generous timeouts for the large files; cached in OUT_DIR across builds.
    if env::var("CARGO_FEATURE_MODEL_SMOKE_TEST_MULTIMODAL").is_ok() {
        let gguf = ensure_model(
            VLM_GGUF_URL,
            VLM_GGUF_FILENAME,
            VLM_GGUF_SHA256,
            900,
            &out_dir,
        );
        let mmproj = ensure_model(
            VLM_MMPROJ_URL,
            VLM_MMPROJ_FILENAME,
            VLM_MMPROJ_SHA256,
            900,
            &out_dir,
        );
        println!(
            "cargo:rustc-env=KX_LLAMACPP_SMOKE_VLM_GGUF={}",
            gguf.display()
        );
        println!(
            "cargo:rustc-env=KX_LLAMACPP_SMOKE_VLM_MMPROJ={}",
            mmproj.display()
        );
    }
}

/// Ensure `filename` (verified against `expected_sha`) exists in `out_dir`,
/// downloading it from `url` on a miss / SHA mismatch. Returns its path.
fn ensure_model(
    url: &str,
    filename: &str,
    expected_sha: &str,
    timeout_secs: u64,
    out_dir: &Path,
) -> PathBuf {
    let path = out_dir.join(filename);
    if path.exists() {
        // Reuse a cached copy whose SHA matches; otherwise re-download (recovers
        // from a partial write / poisoned cache).
        if verify_sha(&path, expected_sha).is_ok() {
            return path;
        }
        let _ = fs::remove_file(&path);
    }
    download_and_verify(url, &path, expected_sha, timeout_secs);
    path
}

fn download_and_verify(url: &str, dest: &PathBuf, expected_sha: &str, timeout_secs: u64) {
    eprintln!(
        "kx-llamacpp build.rs: downloading {url} → {}",
        dest.display()
    );

    // ureq with default TLS handles the redirects HF serves.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(timeout_secs))
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
