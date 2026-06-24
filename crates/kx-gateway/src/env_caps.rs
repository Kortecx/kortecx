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
#[cfg(feature = "inference")]
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
#[cfg(feature = "inference")]
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
#[cfg(feature = "inference")]
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
