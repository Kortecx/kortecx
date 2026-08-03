//! Built-in chat templates — the FALLBACK for models whose embedded GGUF
//! `tokenizer.chat_template` llama.cpp's `minja` engine cannot render.
//!
//! The PRIMARY templating path is the model's OWN embedded template, applied via
//! [`kx_llamacpp::Model::apply_chat_template`] (what `llama-server` does) — that
//! is model-agnostic and correct for any model llama.cpp can template (Qwen,
//! Mistral, Llama, …). These built-ins cover the gaps: notably **Gemma-4**, whose
//! 17 KB tool-calling jinja template `minja` rejects (`rc = -1`), so a faithful
//! hand-rolled fallback is required.
//!
//! Both renders are **pure + deterministic** (no date / random injection), which
//! preserves the greedy + content-addressed replay contract (R49): the same
//! messages always produce the same prompt, hence a byte-reproducible completion.
//!
//! The render produces a string with the model's control tokens AS TEXT; the
//! dispatch tokenizer parses them as special tokens (`parse_special = true`),
//! exactly as the existing hand-rolled `ChatML` path always has — so there is no
//! new BOS / special-token handling here.

use kx_llamacpp::ChatMessage;

/// Render `messages` with a built-in template keyed on `model_desc`'s leading
/// architecture token. `model_desc` is `kx_llamacpp::Model::desc()` (llama.cpp's
/// `llama_model_desc`, e.g. `"gemma4 12B Q4_K - Medium"` → arch `"gemma4"`).
///
/// The per-family templates come from [`crate::model_family`] — the same table the
/// FFI-free Ollama backend reads, so the two backends cannot prompt one model two ways.
///
/// ⚠ Resolution is EXACT, and used to be a prefix (`arch.starts_with("gemma")`). That
/// prefix handed **Gemma-3 the Gemma-4 vocabulary**: the families share a name stem and
/// share no control tokens (`<start_of_turn>` vs `<|turn>`). Gemma-3 now resolves to its
/// own entry and renders correctly; an arch in no entry still falls back to `ChatML`.
#[must_use]
pub(crate) fn builtin_render(model_desc: &str, messages: &[ChatMessage]) -> String {
    let template = crate::model_family::resolve_from_desc(model_desc)
        .map_or(crate::model_family::CHATML, |f| f.template);
    // ChatML is the broad default (Qwen / Yi / many Mistral GGUFs) — reached when the
    // embedded template is absent or unrenderable AND the arch is unregistered, a
    // deliberately conservative last resort.
    template.render_messages(
        messages
            .iter()
            .map(|m| (m.role.as_str(), m.content.as_str())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs() -> Vec<ChatMessage> {
        vec![ChatMessage::system("be precise"), ChatMessage::user("hi")]
    }

    #[test]
    fn gemma_arch_uses_turn_format_with_answer_channel() {
        let out = builtin_render("gemma4 12B Q4_K - Medium", &msgs());
        assert_eq!(
            out,
            "<|turn>system\nbe precise<turn|>\n<|turn>user\nhi<turn|>\n\
             <|turn>model\n<|channel>thought\n<channel|>"
        );
    }

    /// ★ CORRECTED. This test used to assert that `gemma3` rendered in GEMMA-4's
    /// vocabulary, because the dispatch was a `starts_with("gemma")` prefix. That was
    /// the bug, asserted as if it were the intent: the two families share a name stem
    /// and share NO control tokens, so a Gemma-3 model whose embedded template failed to
    /// render was handed `<|turn>` markers it has never been trained on.
    ///
    /// Gemma-3 now resolves to its own registry entry — `<start_of_turn>`, no system
    /// role, no thought channel.
    #[test]
    fn gemma3_renders_its_own_vocabulary_not_gemma4s() {
        let out = builtin_render("gemma3 4B", &msgs());
        assert!(out.starts_with("<start_of_turn>user\n"), "{out:?}");
        for gemma4_marker in ["<|turn>", "<turn|>", "<|channel>thought"] {
            assert!(
                !out.contains(gemma4_marker),
                "gemma3 must not inherit gemma4's {gemma4_marker:?}: {out:?}"
            );
        }
    }

    #[test]
    fn non_gemma_falls_back_to_chatml() {
        let out = builtin_render("qwen3 0.6B Q4_K - Medium", &msgs());
        assert_eq!(
            out,
            "<|im_start|>system\nbe precise<|im_end|>\n\
             <|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n"
        );
    }

    #[test]
    fn unknown_arch_defaults_to_chatml() {
        assert!(builtin_render("", &msgs()).starts_with("<|im_start|>"));
    }

    #[test]
    fn assistant_role_renders_as_model_for_gemma() {
        let m = vec![ChatMessage::assistant("prior")];
        assert!(builtin_render("gemma4 12B", &m).starts_with("<|turn>model\nprior<turn|>\n"));
    }

    #[test]
    fn deterministic_same_input_same_output() {
        assert_eq!(
            builtin_render("gemma4", &msgs()),
            builtin_render("gemma4", &msgs())
        );
    }
}
