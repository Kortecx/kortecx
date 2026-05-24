//! `ChatMessage` + chat-template application.
//!
//! Wraps `llama_chat_apply_template`. The HF-shaped path is:
//!
//! ```ignore
//! let prompt = model.apply_chat_template(None, &[
//!     ChatMessage::system("You are concise."),
//!     ChatMessage::user("Capital of France?"),
//! ], /* add_assistant */ true)?;
//! // prompt is now a model-specific formatted string ready to tokenize.
//! ```
//!
//! Without this, every caller has to hand-write the model's prompt format
//! (Llama-2's `[INST] ... [/INST]`, ChatML's `<|im_start|>...<|im_end|>`,
//! etc.) — brittle and tightly coupled to the model.
//!
//! The same `ChatMessage` type + `apply_chat_template` shape is intended to
//! be implemented by every future `InferenceBackend` adapter so agent code is
//! portable across in-process and out-of-process inference engines.

use std::ffi::CString;

use kx_llamacpp_sys as sys;

use crate::error::LlamaError;
use crate::model::Model;

/// One message in a chat conversation.
///
/// `role` is the speaker tag the template expects ("system", "user",
/// "assistant", "tool", etc. — model-specific). `content` is the message
/// body.
///
/// # Examples
///
/// ```
/// use kx_llamacpp::ChatMessage;
///
/// let m = ChatMessage::user("Hello, world");
/// assert_eq!(m.role, "user");
/// assert_eq!(m.content, "Hello, world");
///
/// let custom = ChatMessage::new("tool", "{\"result\": 42}");
/// assert_eq!(custom.role, "tool");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Speaker tag (e.g. "system", "user", "assistant").
    pub role: String,
    /// Message text.
    pub content: String,
}

impl ChatMessage {
    /// Construct a message with a custom role.
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }

    /// System-role message (typically: top-level instructions).
    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }

    /// User-role message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }

    /// Assistant-role message (the model's prior response in a
    /// multi-turn conversation).
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }
}

impl<'b> Model<'b> {
    /// Look up the model's chat template by name (or default if `name=None`).
    /// Returns `None` if the model does not carry the requested template
    /// (typical for non-chat models like stories260K).
    pub fn chat_template(&self, name: Option<&str>) -> Option<String> {
        let c_name = name.and_then(|n| CString::new(n).ok());
        let name_ptr = c_name.as_ref().map_or(core::ptr::null(), |c| c.as_ptr());
        // SAFETY: model ptr is valid; name is NUL-terminated or null.
        let raw = unsafe { sys::llama_model_chat_template(self.ptr.as_ptr(), name_ptr) };
        if raw.is_null() {
            None
        } else {
            // SAFETY: returned pointer is owned by the model and lives as long
            // as the model does; we copy to an owned String immediately.
            let s = unsafe { core::ffi::CStr::from_ptr(raw) };
            Some(s.to_string_lossy().into_owned())
        }
    }

    /// Apply a chat template to a sequence of messages, producing the model's
    /// expected prompt string. This is the HF `apply_chat_template` analog.
    ///
    /// * `template` — `None` to use the model's default template
    ///   (recommended); `Some("name")` to use a named one if the model
    ///   exposes one; `Some(raw_jinja)` to pass an inline template string
    ///   (matches llama.cpp's CLI flag behavior).
    /// * `messages` — the conversation so far.
    /// * `add_assistant` — append the assistant-turn prefix so the model
    ///   knows to start generating (the typical case before a `generate`
    ///   call).
    ///
    /// # Errors
    /// - [`LlamaError::ChatTemplateFailed`] if llama.cpp returns a negative
    ///   status (malformed template, unknown variable, etc.). The model
    ///   lacks the template entirely if `chat_template(None)` returns
    ///   `None` — call that first to check.
    #[tracing::instrument(level = "debug", skip(self, messages), fields(n_msg = messages.len()))]
    pub fn apply_chat_template(
        &self,
        template: Option<&str>,
        messages: &[ChatMessage],
        add_assistant: bool,
    ) -> Result<String, LlamaError> {
        // Resolve the template string. If caller passed an inline template,
        // use it verbatim. Otherwise fetch the model's default (or named) one.
        let tmpl_string: Option<String> = match template {
            Some(t) => Some(t.to_string()),
            None => self.chat_template(None),
        };
        let c_tmpl = tmpl_string
            .as_deref()
            .map(|s| CString::new(s).map_err(|_| LlamaError::TokenizeFailed(0)))
            .transpose()?;
        let tmpl_ptr = c_tmpl.as_ref().map_or(core::ptr::null(), |c| c.as_ptr());

        // Build the C-side chat-message array. The C structs hold borrowed
        // pointers, so the owning CStrings must outlive the call — keep them
        // in a Vec.
        let role_cstrings: Vec<CString> = messages
            .iter()
            .map(|m| CString::new(m.role.as_str()).map_err(|_| LlamaError::TokenizeFailed(0)))
            .collect::<Result<Vec<_>, _>>()?;
        let content_cstrings: Vec<CString> = messages
            .iter()
            .map(|m| CString::new(m.content.as_str()).map_err(|_| LlamaError::TokenizeFailed(0)))
            .collect::<Result<Vec<_>, _>>()?;

        let chat: Vec<sys::llama_chat_message> = role_cstrings
            .iter()
            .zip(content_cstrings.iter())
            .map(|(r, c)| sys::llama_chat_message {
                role: r.as_ptr(),
                content: c.as_ptr(),
            })
            .collect();

        // First call to size the buffer; resize and re-call if needed.
        let mut buf =
            vec![0i8; 1024.max(messages.iter().map(|m| m.content.len()).sum::<usize>() * 2)];

        // SAFETY: chat array elements borrow from role_cstrings + content_cstrings
        // which are alive for the duration of the FFI call; buf is owned and
        // mutable for buf.len() bytes.
        let n = unsafe {
            sys::llama_chat_apply_template(
                tmpl_ptr,
                chat.as_ptr(),
                chat.len(),
                add_assistant,
                buf.as_mut_ptr().cast::<core::ffi::c_char>(),
                buf.len() as i32,
            )
        };

        let written: i32 = if n > buf.len() as i32 {
            // Resize and retry.
            buf.resize(n as usize, 0);
            let n2 = unsafe {
                sys::llama_chat_apply_template(
                    tmpl_ptr,
                    chat.as_ptr(),
                    chat.len(),
                    add_assistant,
                    buf.as_mut_ptr().cast::<core::ffi::c_char>(),
                    buf.len() as i32,
                )
            };
            if n2 < 0 {
                return Err(LlamaError::ChatTemplateFailed(n2));
            }
            n2
        } else if n < 0 {
            return Err(LlamaError::ChatTemplateFailed(n));
        } else {
            n
        };

        buf.truncate(written.max(0) as usize);
        let bytes: Vec<u8> = buf.into_iter().map(|c| c as u8).collect();
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
