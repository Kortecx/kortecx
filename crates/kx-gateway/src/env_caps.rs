//! POC-5a (CAS env-knobs / F4) — operator overrides for the byte/token caps that
//! bound the serve context window, agentic-edit decode budget, and chat-RAG fan-in.
//!
//! Each knob is **additive and default-preserving**: unset (or unparseable, or out
//! of a sane range) ⇒ the original hard-coded default, so a serve with no env set is
//! byte- and behaviour-identical to before (the canonical digest is unaffected). A
//! set knob is a deliberate operator choice (e.g. scaffolding larger agentic-app
//! trees needs a bigger per-file decode budget). Values are read at point-of-use; the
//! process environment is constant for a serve's lifetime, so a cap that must stay
//! deterministic within a run (the F-7 assemble window) reads the SAME value every
//! call — determinism (R49) holds, while staying trivially testable (no global cache).
//!
//! SN-8 / D35: a token knob NEVER widens a warrant beyond what the model executor
//! accepts — these size the SEED-time recipe warrant (the ceiling), which the
//! executor's `inference_params_from_mote` still clamps; the env only moves the
//! seeded ceiling, it cannot bypass downstream enforcement.

/// Default F-7 serve-context window. Used only on the inference serve path
/// (`window_bytes` + the `assemble_serve` overflow test), so it is gated to match.
#[cfg(feature = "serve-engine")]
pub(crate) const DEFAULT_WINDOW_BYTES: usize = 32 * 1024;
/// Default agentic-edit / scaffold-write input-token budget (the react-edit ceiling).
pub(crate) const DEFAULT_EDIT_MAX_INPUT_TOKENS: u32 = 8_192;
/// Default agentic-edit / scaffold-write output-token budget (a full file rewrite).
pub(crate) const DEFAULT_EDIT_MAX_OUTPUT_TOKENS: u32 = 3_072;
/// Default chat-RAG top-k ceiling (the untrusted `k` is clamped to this).
pub(crate) const DEFAULT_CHAT_RAG_MAX_K: usize = 16;

// Defensive upper bounds — a garbage-large env value clamps here rather than blowing
// the model window / decode loop. Generous (these are operator opt-ins), never silent
// (out-of-range falls back to the default, see `parse_cap`).
#[cfg(feature = "serve-engine")]
const MAX_WINDOW_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
const MAX_EDIT_TOKENS: u32 = 131_072; // 128k
const MAX_CHAT_RAG_K: usize = 256;

/// Resolve a `usize` cap from a raw env string: parse, accept only `min..=max`,
/// else fall back to `default`. Pure + total (the unit-tested core).
fn parse_cap(raw: Option<&str>, default: usize, min: usize, max: usize) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&v| v >= min && v <= max)
        .unwrap_or(default)
}

/// Resolve a `u32` token cap (`min` is 1 — a zero budget would never decode).
fn parse_cap_u32(raw: Option<&str>, default: u32, max: u32) -> u32 {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&v| v >= 1 && v <= max)
        .unwrap_or(default)
}

/// The F-7 serve-context window cap (`KX_SERVE_WINDOW_BYTES`).
#[cfg(feature = "serve-engine")]
pub(crate) fn window_bytes() -> usize {
    parse_cap(
        std::env::var("KX_SERVE_WINDOW_BYTES").ok().as_deref(),
        DEFAULT_WINDOW_BYTES,
        1_024,
        MAX_WINDOW_BYTES,
    )
}

/// The agentic-edit / scaffold-write input-token budget (`KX_SERVE_EDIT_MAX_INPUT_TOKENS`).
pub(crate) fn edit_max_input_tokens() -> u32 {
    parse_cap_u32(
        std::env::var("KX_SERVE_EDIT_MAX_INPUT_TOKENS")
            .ok()
            .as_deref(),
        DEFAULT_EDIT_MAX_INPUT_TOKENS,
        MAX_EDIT_TOKENS,
    )
}

/// The agentic-edit / scaffold-write output-token budget (`KX_SERVE_EDIT_MAX_OUTPUT_TOKENS`).
pub(crate) fn edit_max_output_tokens() -> u32 {
    parse_cap_u32(
        std::env::var("KX_SERVE_EDIT_MAX_OUTPUT_TOKENS")
            .ok()
            .as_deref(),
        DEFAULT_EDIT_MAX_OUTPUT_TOKENS,
        MAX_EDIT_TOKENS,
    )
}

/// The chat-RAG top-k ceiling (`KX_SERVE_CHAT_RAG_MAX_K`).
pub(crate) fn chat_rag_max_k() -> usize {
    parse_cap(
        std::env::var("KX_SERVE_CHAT_RAG_MAX_K").ok().as_deref(),
        DEFAULT_CHAT_RAG_MAX_K,
        1,
        MAX_CHAT_RAG_K,
    )
}

/// Defensive bounds on the RC4a RAG knobs (chars / chunks-per-doc). Generous; an
/// out-of-range value falls back to the default (never silent garbage).
#[cfg(feature = "hnsw")]
const MAX_RAG_CHARS: usize = 100_000;
#[cfg(feature = "hnsw")]
const MAX_RAG_CHUNKS_PER_DOC: usize = 100_000;

/// Resolve a boolean knob: `1/true/yes/on` ⇒ true, `0/false/no/off` ⇒ false, else
/// the default. Pure + total.
#[cfg(feature = "hnsw")]
fn parse_bool(raw: Option<&str>, default: bool) -> bool {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if s == "1" || s == "true" || s == "yes" || s == "on" => true,
        Some(s) if s == "0" || s == "false" || s == "no" || s == "off" => false,
        _ => default,
    }
}

/// Resolve a basis-point knob (0..=10000), else the default.
#[cfg(feature = "hnsw")]
fn parse_bp(raw: Option<&str>, default: u32) -> u32 {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&v| v <= 10_000)
        .unwrap_or(default)
}

/// The operator RAG config (RC4a `KX_SERVE_RAG_*` knobs): retrieval mode, chunk
/// size/overlap, the per-doc chunk cap, RRF k, MMR lambda + on/off, and stopwords.
/// Each is additive + default-preserving (unset ⇒ [`RagConfig::default`]); all are
/// OPERATOR config, never client-chosen (SN-8).
#[cfg(feature = "hnsw")]
pub(crate) fn rag_config() -> crate::datasets::RagConfig {
    use kx_gateway_core::RetrievalMode;
    let mut c = crate::datasets::RagConfig::default();
    if let Ok(m) = std::env::var("KX_SERVE_RAG_MODE") {
        c.default_mode = match m.trim().to_ascii_lowercase().as_str() {
            "dense" => RetrievalMode::Dense,
            "hybrid" => RetrievalMode::Hybrid,
            _ => c.default_mode,
        };
    }
    c.chunk_max_chars = parse_cap(
        std::env::var("KX_SERVE_RAG_CHUNK_SIZE").ok().as_deref(),
        c.chunk_max_chars,
        1,
        MAX_RAG_CHARS,
    );
    c.chunk_overlap_chars = parse_cap(
        std::env::var("KX_SERVE_RAG_CHUNK_OVERLAP").ok().as_deref(),
        c.chunk_overlap_chars,
        0,
        MAX_RAG_CHARS,
    );
    c.max_chunks_per_doc = parse_cap(
        std::env::var("KX_SERVE_RAG_MAX_CHUNKS_PER_DOC")
            .ok()
            .as_deref(),
        c.max_chunks_per_doc,
        0,
        MAX_RAG_CHUNKS_PER_DOC,
    );
    c.rrf_k = parse_cap_u32(
        std::env::var("KX_SERVE_RAG_RRF_K").ok().as_deref(),
        c.rrf_k,
        10_000,
    );
    c.mmr_lambda_bp = parse_bp(
        std::env::var("KX_SERVE_RAG_MMR_LAMBDA").ok().as_deref(),
        c.mmr_lambda_bp,
    );
    c.rerank = parse_bool(
        std::env::var("KX_SERVE_RAG_RERANK").ok().as_deref(),
        c.rerank,
    );
    c.stopwords = parse_bool(
        std::env::var("KX_SERVE_RAG_STOPWORDS").ok().as_deref(),
        c.stopwords,
    );
    c
}

/// Whether to PRE-LOAD the dataset embed model in the background at serve start
/// (`KX_SERVE_WARM_EMBED`, default off). Probe-only — it fires one throwaway embed
/// to pull the model resident so the FIRST real ingest is already warm
/// (`T-OLLAMA-EMBED-COLD-TIMEOUT`); it never force-pulls a missing model and never
/// blocks startup.
#[cfg(all(feature = "hnsw", feature = "serve-engine"))]
pub(crate) fn warm_embed() -> bool {
    parse_bool(std::env::var("KX_SERVE_WARM_EMBED").ok().as_deref(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cap_unset_is_default() {
        assert_eq!(parse_cap(None, 256, 1, 1000), 256);
    }

    #[test]
    fn parse_cap_valid_override_applies() {
        assert_eq!(parse_cap(Some("512"), 256, 1, 1000), 512);
        assert_eq!(parse_cap(Some("  64 "), 256, 1, 1000), 64);
    }

    #[test]
    fn parse_cap_out_of_range_or_garbage_falls_back_to_default() {
        assert_eq!(parse_cap(Some("0"), 256, 1, 1000), 256); // below min
        assert_eq!(parse_cap(Some("99999"), 256, 1, 1000), 256); // above max
        assert_eq!(parse_cap(Some("not-a-number"), 256, 1, 1000), 256);
        assert_eq!(parse_cap(Some(""), 256, 1, 1000), 256);
        assert_eq!(parse_cap(Some("-5"), 256, 1, 1000), 256); // negative ⇒ usize parse fails
    }

    #[test]
    fn parse_cap_u32_rejects_zero_and_overflow() {
        assert_eq!(parse_cap_u32(Some("0"), 3072, 131_072), 3072);
        assert_eq!(parse_cap_u32(Some("4096"), 3072, 131_072), 4096);
        assert_eq!(parse_cap_u32(Some("999999"), 3072, 131_072), 3072); // above max
        assert_eq!(parse_cap_u32(None, 3072, 131_072), 3072);
    }
}
