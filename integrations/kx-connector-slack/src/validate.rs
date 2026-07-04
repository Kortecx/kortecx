// SPDX-License-Identifier: Apache-2.0
//! Fail-closed argument guards for the Slack Web API surface.
//!
//! The connector interpolates channel ids into URL query parameters
//! (`?channel={id}`), so ids are validated as bare Slack **alphanumeric** ids
//! (e.g. `C0123ABCD`, `G0…`, `D0…`) BEFORE any request is built — a `/`, `?`, `#`,
//! or `..` can never smuggle into the URL. Message text is bounded to Slack's
//! ~40 000-character limit. Both reject fail-closed with [`SlackError::BadArgs`] and
//! never carry the credential.

use crate::slack::SlackError;

/// Slack's per-message text limit (characters, not bytes; the Web API rejects
/// beyond ~40 000). We bound conservatively at that ceiling.
pub const MAX_TEXT_CHARS: usize = 40_000;

/// Validate a Slack channel id (`C…`/`G…`/`D…`): non-empty and ASCII alphanumeric
/// only. This is also the injection guard — an alphanumeric id cannot contain a path
/// separator or query/fragment delimiter.
///
/// # Errors
/// [`SlackError::BadArgs`] when `id` is empty or contains a non-alphanumeric byte.
pub fn check_channel_id(field: &str, id: &str) -> Result<(), SlackError> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(SlackError::BadArgs(format!(
            "`{field}` must be a Slack id (letters and digits only)"
        )));
    }
    Ok(())
}

/// Validate message text: non-empty and within Slack's ~40 000-char limit.
///
/// # Errors
/// [`SlackError::BadArgs`] when `text` is empty or exceeds [`MAX_TEXT_CHARS`].
pub fn check_text(text: &str) -> Result<(), SlackError> {
    if text.is_empty() {
        return Err(SlackError::BadArgs("`text` must not be empty".to_string()));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(SlackError::BadArgs(format!(
            "`text` exceeds Slack's {MAX_TEXT_CHARS}-character limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_channel_id, check_text, MAX_TEXT_CHARS};

    #[test]
    fn channel_id_accepts_alphanumeric() {
        assert!(check_channel_id("channel_id", "C0123ABCD").is_ok());
        assert!(check_channel_id("channel_id", "G0ABCDEF1").is_ok());
    }

    #[test]
    fn channel_id_rejects_empty_and_non_alnum() {
        assert!(check_channel_id("channel_id", "").is_err());
        assert!(check_channel_id("channel_id", "C0/12").is_err());
        assert!(check_channel_id("channel_id", "../secrets").is_err());
        assert!(check_channel_id("channel_id", "chan#1").is_err());
        assert!(check_channel_id("channel_id", "a b").is_err());
    }

    #[test]
    fn text_bounds_are_enforced() {
        assert!(check_text("hi").is_ok());
        assert!(check_text("").is_err());
        let too_long: String = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(check_text(&too_long).is_err());
        let at_limit: String = "x".repeat(MAX_TEXT_CHARS);
        assert!(check_text(&at_limit).is_ok());
    }
}
