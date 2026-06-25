//! Model Control v2 — the model-acquisition seam behind `PullModel` /
//! `GetPullStatus`.
//!
//! A pull is HOST INFRASTRUCTURE, not a client Mote (SN-8): it mutates operator/host
//! state (the filesystem, the served set, network egress) — axes a client warrant
//! never asserts. The host impl owns the runtime (a background download task + an
//! advisory in-memory tracker) + the deny-by-default opt-in/allowlist/SHA gate; this
//! seam is the FFI-free vocabulary the gateway service speaks. `None` ⇒ `PullModel` /
//! `GetPullStatus` return `unimplemented`. Off-journal, off-digest — the catalog,
//! the residency, and this progress are all pure RAM display state.

/// The pull lifecycle phase (mirrors `proto::get_pull_status_response::Phase`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PullPhase {
    /// Validating the request (operator opt-in, host allowlist, sha presence).
    Resolving,
    /// Bytes in flight (the daemon `/api/pull` stream or the direct GGUF download).
    Downloading,
    /// SHA-256 verify of the downloaded bytes (the direct-URL path).
    Verifying,
    /// Registering the model into the catalog + routing + chat recipe.
    Registering,
    /// Registered + immediately switchable (no restart).
    Done,
    /// The pull failed; the `detail` carries the advisory reason.
    Failed,
}

/// The resolved pull progress (the host's advisory tracker — never authority).
#[derive(Clone, Debug)]
pub struct PullProgress {
    /// The current phase.
    pub phase: PullPhase,
    /// Bytes downloaded so far (`0` until the stream reports a layer).
    pub bytes_downloaded: u64,
    /// Total bytes (`0` when unknown — Ollama may omit it; HF `Content-Length`).
    pub bytes_total: u64,
    /// Advisory progress / failure text (never authority).
    pub detail: String,
}

/// The download source the host puller acts on (already discriminated from the proto
/// `oneof`). The SHA on the URL variant is REQUIRED (validated at the RPC boundary).
pub enum PullSource {
    /// Pull from the Ollama registry via `/api/pull` (the quick/easy GR24 path).
    OllamaTag(String),
    /// Download a GGUF from a `huggingface.co` `/resolve/` URL; `sha256` is the
    /// REQUIRED hex digest the download is verified against before registration.
    Url {
        /// The `https://huggingface.co/<repo>/resolve/<rev>/<file>.gguf` URL.
        url: String,
        /// The expected SHA-256 of the file (hex).
        sha256: String,
    },
}

/// The admission decision for a `PullModel` request — the deny-by-default negative
/// path is a `Refused` (no egress, no file, no journal fact).
pub enum PullAdmission {
    /// Accepted: the background pull started (poll `GetPullStatus` with `model_id`).
    Accepted {
        /// The catalog id the pull registers under (progress is keyed by it).
        model_id: String,
    },
    /// Refused BEFORE any egress (downloads disabled / host not allowlisted / no
    /// sha256 / malformed source). The `detail` is the honest reason.
    Refused {
        /// The honest refusal reason (surfaced as `PullModelResponse.detail`).
        detail: String,
    },
}

/// The host-side model-pull orchestrator seam. The host impl owns the runtime (the
/// background download task), the advisory tracker, the registration handles (the
/// catalog, lifecycle, and per-engine register), and the operator opt-in plus the
/// host-allowlist and SHA-256 gate. An unwired seam makes `PullModel` and
/// `GetPullStatus` return `unimplemented`.
pub trait ModelPuller: Send + Sync {
    /// Admit + start (or resume) a background pull. The opt-in + allowlist gate is
    /// enforced HERE, BEFORE any egress: a refused pull returns `Refused` and never
    /// touches the network or the filesystem. An accepted pull spawns a background
    /// task and returns immediately (poll `status`).
    fn start(&self, source: PullSource, desired_id: &str) -> PullAdmission;

    /// The current pull progress for `model_id` (advisory). `None` ⇒ an unknown id
    /// (never started, or evicted after completion).
    fn status(&self, model_id: &str) -> Option<PullProgress>;
}
