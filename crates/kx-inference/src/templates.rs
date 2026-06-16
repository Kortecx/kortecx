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
#[must_use]
pub(crate) fn builtin_render(model_desc: &str, messages: &[ChatMessage]) -> String {
    let arch = model_desc.split_whitespace().next().unwrap_or_default();
    if arch.starts_with("gemma") {
        gemma(messages)
    } else {
        // ChatML is the broad default (Qwen / Yi / many Mistral GGUFs). Reached
        // only when the embedded template is absent or unrenderable AND the model
        // is not Gemma — a deliberately conservative last resort.
        chatml(messages)
    }
}

/// Gemma-4 turn format (`<|turn>{role}\n…<turn|>`) + the answer-channel
/// generation prompt the embedded template emits for `add_generation_prompt` with
/// `enable_thinking=false`. Gemma's assistant role token is `model`.
fn gemma(messages: &[ChatMessage]) -> String {
    let mut s = String::new();
    for m in messages {
        let role = if m.role == "assistant" {
            "model"
        } else {
            m.role.as_str()
        };
        s.push_str("<|turn>");
        s.push_str(role);
        s.push('\n');
        s.push_str(&m.content);
        s.push_str("<turn|>\n");
    }
    s.push_str("<|turn>model\n<|channel>thought\n<channel|>");
    s
}

/// Qwen / `ChatML` format (`<|im_start|>{role}\n…<|im_end|>`) + the assistant
/// prefix. Byte-identical to the long-standing hand-rolled `chatml()` shape.
fn chatml(messages: &[ChatMessage]) -> String {
    let mut s = String::new();
    for m in messages {
        s.push_str("<|im_start|>");
        s.push_str(&m.role);
        s.push('\n');
        s.push_str(&m.content);
        s.push_str("<|im_end|>\n");
    }
    s.push_str("<|im_start|>assistant\n");
    s
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

    #[test]
    fn gemma3_prefix_also_matches() {
        assert!(builtin_render("gemma3 4B", &msgs()).starts_with("<|turn>system\n"));
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
        assert!(gemma(&m).starts_with("<|turn>model\nprior<turn|>\n"));
    }

    #[test]
    fn deterministic_same_input_same_output() {
        assert_eq!(
            builtin_render("gemma4", &msgs()),
            builtin_render("gemma4", &msgs())
        );
    }
}
