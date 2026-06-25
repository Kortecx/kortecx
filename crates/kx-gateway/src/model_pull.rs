//! Model Control v2 — the host model-acquisition orchestrator ([`ModelPuller`]).
//!
//! HOST INFRASTRUCTURE, not a client Mote (SN-8): a pull mutates operator/host state
//! (the filesystem, the served set, network egress). DENY-BY-DEFAULT: every pull is
//! refused unless the operator sets `KX_SERVE_ALLOW_MODEL_PULL`, and a direct-URL pull
//! must target an allowlisted host (HuggingFace by default) over `https` and verify a
//! mandatory SHA-256 before the bytes are registered. The blocking download runs on
//! `spawn_blocking`, OFF the durable-spine tokio workers. On success the model is
//! registered into the SAME catalog + lifecycle set + routing the running serve uses,
//! plus a `kx/recipes/m-<id>` chat recipe — so it is immediately switchable + chattable
//! WITHOUT a restart. Off-journal / off-digest (the catalog + this progress are RAM).

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use kx_gateway_core::{
    ModelPuller, ModelSummaryEntry, PullAdmission, PullPhase, PullProgress, PullSource,
};
use kx_mote::ModelId;

use crate::provision::DemoLibrary;

/// The default model-download host allowlist (HuggingFace + its known LFS/Xet CDNs an
/// HF `/resolve/` link 302-redirects to). Operator-extensible via
/// `KX_SERVE_MODEL_PULL_HOSTS`. Deny-by-default: a redirect to any other host is refused.
const DEFAULT_PULL_HOSTS: &[&str] = &[
    "huggingface.co",
    "hf.co",
    "cdn-lfs.huggingface.co",
    "cdn-lfs-us-1.huggingface.co",
    "cdn-lfs-eu-1.huggingface.co",
    "cas-bridge.xethub.hf.co",
];

/// Whether operator-enabled model downloads are ON (`KX_SERVE_ALLOW_MODEL_PULL`). OFF
/// by default (deny-by-default; the operator's explicit opt-in authorizes egress, SN-8).
pub(crate) fn pull_enabled() -> bool {
    matches!(
        std::env::var("KX_SERVE_ALLOW_MODEL_PULL").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

/// The resolved model-download policy (operator config — never client-controlled).
pub(crate) struct PullPolicy {
    /// The master opt-in (`KX_SERVE_ALLOW_MODEL_PULL`). `false` ⇒ every pull is refused.
    enabled: bool,
    /// The host allowlist a direct-URL pull (and every redirect hop) must match. Read
    /// only by the `inference`-gated direct-URL path (constructed always via env).
    #[cfg_attr(not(feature = "inference"), allow(dead_code))]
    allowlist: Vec<String>,
    /// Where a direct-URL GGUF lands (`KX_SERVE_MODELS_DIR`, else a serve-relative dir).
    /// Read only by the `inference`-gated direct-URL path.
    #[cfg_attr(not(feature = "inference"), allow(dead_code))]
    models_dir: PathBuf,
}

impl PullPolicy {
    /// Resolve the policy from the environment (operator config).
    pub(crate) fn from_env(default_models_dir: PathBuf) -> Self {
        let mut allowlist: Vec<String> = DEFAULT_PULL_HOSTS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        if let Ok(extra) = std::env::var("KX_SERVE_MODEL_PULL_HOSTS") {
            for h in extra
                .split([',', ';', '\n', '\r'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                allowlist.push(h.to_ascii_lowercase());
            }
        }
        let models_dir =
            std::env::var_os("KX_SERVE_MODELS_DIR").map_or(default_models_dir, PathBuf::from);
        Self {
            enabled: pull_enabled(),
            allowlist,
            models_dir,
        }
    }
}

/// The host model-pull orchestrator. `Clone` (all-`Arc`) so `start` hands a clone to
/// the background download task; the shared tracker keeps live phase/byte progress.
#[derive(Clone)]
pub(crate) struct HostModelPuller {
    policy: Arc<PullPolicy>,
    tracker: Arc<Mutex<HashMap<String, PullProgress>>>,
    /// The shared served-catalog entries (a pulled model is appended here).
    catalog: Arc<RwLock<Vec<ModelSummaryEntry>>>,
    /// The shared lifecycle registered set (a pulled model becomes load/offload-able).
    registered: Arc<RwLock<BTreeSet<String>>>,
    /// The recipe library (a pulled model gets its `kx/recipes/m-<id>` chat recipe).
    library: Arc<DemoLibrary>,
    /// The Ollama engine (the `pull <tag>` path register_tag's onto it).
    ollama: Option<Arc<kx_ollama::OllamaBackend>>,
    /// The runtime-mutable llama resolver (the `--url` path registers a GGUF into it).
    #[cfg(feature = "inference")]
    llama_registry: Option<Arc<kx_model_store::MutableRegistry>>,
}

impl HostModelPuller {
    #[cfg(feature = "serve-engine")]
    pub(crate) fn new(
        policy: Arc<PullPolicy>,
        catalog: Arc<RwLock<Vec<ModelSummaryEntry>>>,
        registered: Arc<RwLock<BTreeSet<String>>>,
        library: Arc<DemoLibrary>,
        ollama: Option<Arc<kx_ollama::OllamaBackend>>,
        #[cfg(feature = "inference")] llama_registry: Option<Arc<kx_model_store::MutableRegistry>>,
    ) -> Self {
        Self {
            policy,
            tracker: Arc::new(Mutex::new(HashMap::new())),
            catalog,
            registered,
            library,
            ollama,
            #[cfg(feature = "inference")]
            llama_registry,
        }
    }

    fn set(&self, model_id: &str, phase: PullPhase, downloaded: u64, total: u64, detail: &str) {
        if let Ok(mut t) = self.tracker.lock() {
            t.insert(
                model_id.to_string(),
                PullProgress {
                    phase,
                    bytes_downloaded: downloaded,
                    bytes_total: total,
                    detail: detail.to_string(),
                },
            );
        }
    }

    fn fail(&self, model_id: &str, detail: &str) {
        tracing::warn!(model_id, detail, "model pull failed");
        self.set(model_id, PullPhase::Failed, 0, 0, detail);
    }

    /// Register a runtime-acquired model into the live catalog + lifecycle set + its
    /// `kx/recipes/m-<id>` chat recipe — the "immediately usable, no restart" step.
    fn register_runtime_model(
        &self,
        model_id: &ModelId,
        entry: ModelSummaryEntry,
    ) -> Result<(), String> {
        crate::models::HostModelCatalog::register_entry(&self.catalog, entry);
        crate::model_lifecycle::HostModelLifecycle::register_model_id(
            &self.registered,
            &model_id.0,
        );
        self.library
            .seed_model_recipe(model_id)
            .map_err(|e| format!("recipe seeding failed: {e}"))?;
        Ok(())
    }

    /// The Ollama `pull <tag>` background body (runs on `spawn_blocking`).
    fn run_ollama(&self, tag: &str, model_id: &str) {
        let Some(ollama) = self.ollama.clone() else {
            self.fail(model_id, "the Ollama engine is no longer running");
            return;
        };
        self.set(
            model_id,
            PullPhase::Downloading,
            0,
            0,
            "pulling from the Ollama registry",
        );
        let driver = self.clone();
        let mid = model_id.to_string();
        if let Err(e) = ollama.pull(tag, &mut |status, completed, total| {
            driver.set(&mid, PullPhase::Downloading, completed, total, status);
        }) {
            self.fail(model_id, &format!("pull failed: {e}"));
            return;
        }
        self.set(model_id, PullPhase::Registering, 0, 0, "registering");
        let mref = ModelId(model_id.to_string());
        if let Err(e) = ollama.register_tag(tag) {
            self.fail(model_id, &format!("register failed: {e}"));
            return;
        }
        let entry = crate::model_exec::pulled_ollama_entry(&ollama, &mref);
        if let Err(e) = self.register_runtime_model(&mref, entry) {
            self.fail(model_id, &e);
            return;
        }
        self.set(model_id, PullPhase::Done, 0, 0, "registered");
    }

    /// The direct-URL `pull --url` background body (runs on `spawn_blocking`).
    #[cfg(feature = "inference")]
    fn run_url(&self, url: &str, sha256_hex: &str, filename: &str, model_id: &str) {
        let Some(registry) = self.llama_registry.clone() else {
            self.fail(
                model_id,
                "the in-process inference engine is no longer running",
            );
            return;
        };
        let dest = self.policy.models_dir.join(filename);
        let partial = self.policy.models_dir.join(format!(".{filename}.partial"));
        self.set(model_id, PullPhase::Downloading, 0, 0, "downloading");
        if let Err(e) = self.download_and_verify(url, sha256_hex, &partial, &dest, model_id) {
            let _ = std::fs::remove_file(&partial);
            self.fail(model_id, &e);
            return;
        }
        self.set(model_id, PullPhase::Registering, 0, 0, "registering");
        match crate::model_exec::register_pulled_gguf(&registry, &dest) {
            Ok((mid, entry)) => {
                if let Err(e) = self.register_runtime_model(&mid, entry) {
                    self.fail(model_id, &e);
                    return;
                }
                self.set(model_id, PullPhase::Done, 0, 0, "registered");
            }
            Err(e) => {
                // An unfit GGUF never registers — delete it (no half-registered model).
                let _ = std::fs::remove_file(&dest);
                self.fail(model_id, &e);
            }
        }
    }

    /// Stream the URL to a `.partial` file, hashing as we go; verify the SHA-256
    /// BEFORE the atomic rename + registration. Re-validates the FINAL host after any
    /// redirect (`ureq` follows them) — the DNS-rebind / CDN SSRF guard.
    #[cfg(feature = "inference")]
    fn download_and_verify(
        &self,
        url: &str,
        sha_hex: &str,
        partial: &std::path::Path,
        dest: &std::path::Path,
        model_id: &str,
    ) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        use std::io::{Read, Write};

        std::fs::create_dir_all(&self.policy.models_dir).map_err(|e| e.to_string())?;
        let resp = ureq::get(url)
            .call()
            .map_err(|e| format!("download failed: {e}"))?;
        // SSRF: an HF `/resolve/` link 302s to a CDN — the FINAL host must also be
        // allowlisted (a redirect can never escape the allowlist).
        let final_url = resp.get_url().to_string();
        validate_url(&final_url, &self.policy.allowlist)
            .map_err(|e| format!("redirected to a disallowed host: {e}"))?;
        let total: u64 = resp
            .header("Content-Length")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(partial).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut downloaded: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            hasher.update(&buf[..n]);
            downloaded += n as u64;
            self.set(
                model_id,
                PullPhase::Downloading,
                downloaded,
                total,
                "downloading",
            );
        }
        file.flush().map_err(|e| e.to_string())?;
        drop(file);
        self.set(
            model_id,
            PullPhase::Verifying,
            downloaded,
            total,
            "verifying sha256",
        );
        let got_hex = hex_lower(hasher.finalize().as_slice());
        if !got_hex.eq_ignore_ascii_case(sha_hex.trim()) {
            return Err(format!(
                "sha256 mismatch: expected {}, got {got_hex} (the file was discarded)",
                sha_hex.trim()
            ));
        }
        std::fs::rename(partial, dest).map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "inference")]
    fn start_url(&self, url: String, sha256: String) -> PullAdmission {
        let filename = match validate_url(&url, &self.policy.allowlist) {
            Ok(f) => f,
            Err(detail) => return PullAdmission::Refused { detail },
        };
        if self.llama_registry.is_none() {
            return PullAdmission::Refused {
                detail: "the in-process inference engine is not running; cannot register a \
                         downloaded GGUF"
                    .to_string(),
            };
        }
        let model_id = crate::model_exec::serve_model_id(std::path::Path::new(&filename)).0;
        self.set(&model_id, PullPhase::Resolving, 0, 0, "validating");
        let driver = self.clone();
        let (mid, u, s, f) = (model_id.clone(), url, sha256, filename);
        tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || driver.run_url(&u, &s, &f, &mid)).await;
        });
        PullAdmission::Accepted { model_id }
    }

    #[cfg(not(feature = "inference"))]
    #[allow(clippy::unused_self)]
    fn start_url(&self, _url: String, _sha256: String) -> PullAdmission {
        PullAdmission::Refused {
            detail: "direct-URL model download requires the inference build".to_string(),
        }
    }
}

impl ModelPuller for HostModelPuller {
    fn start(&self, source: PullSource, _desired_id: &str) -> PullAdmission {
        // Deny-by-default: the operator opt-in is the FIRST gate, before any egress.
        if !self.policy.enabled {
            return PullAdmission::Refused {
                detail: "model downloads are disabled; set KX_SERVE_ALLOW_MODEL_PULL=1 \
                         (operator opt-in) to enable them"
                    .to_string(),
            };
        }
        match source {
            PullSource::OllamaTag(tag) => {
                let tag = tag.trim().to_string();
                if self.ollama.is_none() {
                    return PullAdmission::Refused {
                        detail: "no Ollama daemon is serving; cannot pull an Ollama tag"
                            .to_string(),
                    };
                }
                let model_id = tag.clone();
                self.set(&model_id, PullPhase::Resolving, 0, 0, "starting");
                let driver = self.clone();
                let mid = model_id.clone();
                tokio::spawn(async move {
                    let _ =
                        tokio::task::spawn_blocking(move || driver.run_ollama(&tag, &mid)).await;
                });
                PullAdmission::Accepted { model_id }
            }
            PullSource::Url { url, sha256 } => self.start_url(url, sha256),
        }
    }

    fn status(&self, model_id: &str) -> Option<PullProgress> {
        self.tracker.lock().ok()?.get(model_id).cloned()
    }
}

/// Lowercase hex of a byte slice (the SHA-256 comparison form).
#[cfg_attr(not(feature = "inference"), allow(dead_code))] // only the inference URL path hashes
fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        s.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap_or('0'));
    }
    s
}

/// Validate a direct-GGUF download URL, deny-by-default (the admission half of the
/// two-gate egress split; the resolved-IP DNS-rebind re-check at connect time is a
/// follow-up). Requires `https`, a host on `allowlist` (or a public IP literal), no
/// userinfo, a `/resolve/` path, and a `.gguf` filename. Returns the filename.
#[cfg_attr(not(feature = "inference"), allow(dead_code))] // the direct-URL path is inference-gated
fn validate_url(url: &str, allowlist: &[String]) -> Result<String, String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "model download URL must be https".to_string())?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if authority.contains('@') {
        return Err("userinfo (credentials) in a download URL is refused".to_string());
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
    // An IP literal must be a public address (no SSRF to loopback/metadata/internal).
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if !is_public_ip(&ip) {
            return Err(format!(
                "host {host} is a non-public address (internal/loopback/metadata refused)"
            ));
        }
    }
    let host_lc = host.trim_end_matches('.').to_ascii_lowercase();
    if !allowlist
        .iter()
        .any(|h| h.trim_end_matches('.').to_ascii_lowercase() == host_lc)
    {
        return Err(format!(
            "host {host} is not in the model-download allowlist (set KX_SERVE_MODEL_PULL_HOSTS)"
        ));
    }
    let path = &rest[authority.len()..];
    if !path.contains("/resolve/") {
        return Err("a direct download URL must be a HuggingFace /resolve/ link".to_string());
    }
    let filename = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    if !filename.to_ascii_lowercase().ends_with(".gguf") {
        return Err("a direct download URL must point at a .gguf file".to_string());
    }
    Ok(filename)
}

/// Whether `ip` is a public address (the std-only SSRF classifier — internal,
/// loopback, link-local, metadata, CGNAT, and ULA are NOT public).
#[cfg_attr(not(feature = "inference"), allow(dead_code))] // only the direct-URL SSRF guard uses it
fn is_public_ip(ip: &std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            let cgnat = o[0] == 100 && (o[1] & 0xc0) == 64;
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || cgnat)
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            let link_local = (seg[0] & 0xffc0) == 0xfe80;
            let unique_local = (seg[0] & 0xfe00) == 0xfc00;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || link_local
                || unique_local
                || v6.to_ipv4_mapped().is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow() -> Vec<String> {
        vec![
            "huggingface.co".to_string(),
            "cdn-lfs.huggingface.co".to_string(),
        ]
    }

    #[test]
    fn validate_url_accepts_an_hf_resolve_gguf() {
        let f = validate_url(
            "https://huggingface.co/org/repo/resolve/main/model-q4_k_m.gguf",
            &allow(),
        )
        .unwrap();
        assert_eq!(f, "model-q4_k_m.gguf");
    }

    #[test]
    fn validate_url_rejects_non_https() {
        assert!(validate_url("http://huggingface.co/o/r/resolve/main/m.gguf", &allow()).is_err());
    }

    #[test]
    fn validate_url_rejects_non_allowlisted_host() {
        // C2: a host outside the allowlist is refused before any egress.
        let err =
            validate_url("https://evil.example.com/o/r/resolve/main/m.gguf", &allow()).unwrap_err();
        assert!(err.contains("not in the model-download allowlist"));
    }

    #[test]
    fn validate_url_rejects_private_and_metadata_ips() {
        // C3: SSRF — internal / metadata literals are refused.
        for u in [
            "https://169.254.169.254/o/r/resolve/main/m.gguf",
            "https://127.0.0.1/o/r/resolve/main/m.gguf",
            "https://10.0.0.5/o/r/resolve/main/m.gguf",
            "https://[::1]/o/r/resolve/main/m.gguf",
        ] {
            assert!(
                validate_url(u, &allow()).is_err(),
                "expected refusal for {u}"
            );
        }
    }

    #[test]
    fn validate_url_rejects_userinfo_and_non_gguf_and_non_resolve() {
        assert!(validate_url(
            "https://user:tok@huggingface.co/o/r/resolve/main/m.gguf",
            &allow()
        )
        .is_err());
        assert!(validate_url(
            "https://huggingface.co/o/r/resolve/main/notes.txt",
            &allow()
        )
        .is_err());
        assert!(validate_url("https://huggingface.co/o/r/blob/main/m.gguf", &allow()).is_err());
    }

    #[test]
    fn public_ip_classifier_matches_the_egress_policy() {
        use std::net::IpAddr;
        assert!(is_public_ip(&"1.1.1.1".parse::<IpAddr>().unwrap()));
        assert!(!is_public_ip(&"127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(!is_public_ip(&"169.254.169.254".parse::<IpAddr>().unwrap()));
        assert!(!is_public_ip(&"100.64.0.1".parse::<IpAddr>().unwrap())); // CGNAT
    }
}
