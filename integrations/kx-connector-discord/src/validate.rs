// SPDX-License-Identifier: Apache-2.0
//! Fail-closed argument guards for the Discord REST surface.
//!
//! The connector interpolates ids into URL path segments (`/channels/{id}/...`),
//! so ids are validated as bare Discord **snowflakes** (ASCII digits only) BEFORE
//! any request is built — a `/`, `?`, `#`, or `..` can never smuggle into the path.
//! Message content is bounded to Discord's 2000-character limit. Both reject
//! fail-closed with [`DiscordError::BadArgs`] and never carry the credential.

use crate::discord::DiscordError;

/// Discord's per-message content limit (characters, not bytes).
pub const MAX_CONTENT_CHARS: usize = 2000;

/// Validate a Discord snowflake id (a channel/guild id): non-empty and ASCII digits
/// only. This is also the path-injection guard — a digits-only id cannot contain a
/// path separator or query/fragment delimiter.
///
/// # Errors
/// [`DiscordError::BadArgs`] when `id` is empty or contains a non-digit byte.
pub fn check_snowflake(field: &str, id: &str) -> Result<(), DiscordError> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DiscordError::BadArgs(format!(
            "`{field}` must be a Discord id (digits only)"
        )));
    }
    Ok(())
}

/// Validate message content: non-empty and within Discord's 2000-char limit.
///
/// # Errors
/// [`DiscordError::BadArgs`] when `content` is empty or exceeds [`MAX_CONTENT_CHARS`].
pub fn check_content(content: &str) -> Result<(), DiscordError> {
    if content.is_empty() {
        return Err(DiscordError::BadArgs(
            "`content` must not be empty".to_string(),
        ));
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(DiscordError::BadArgs(format!(
            "`content` exceeds Discord's {MAX_CONTENT_CHARS}-character limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_content, check_snowflake, MAX_CONTENT_CHARS};

    #[test]
    fn snowflake_accepts_digits_only() {
        assert!(check_snowflake("channel_id", "123456789012345678").is_ok());
    }

    #[test]
    fn snowflake_rejects_empty_and_non_digits() {
        assert!(check_snowflake("channel_id", "").is_err());
        assert!(check_snowflake("channel_id", "12/34").is_err());
        assert!(check_snowflake("channel_id", "../secrets").is_err());
        assert!(check_snowflake("channel_id", "abc").is_err());
    }

    #[test]
    fn content_bounds_are_enforced() {
        assert!(check_content("hi").is_ok());
        assert!(check_content("").is_err());
        let too_long: String = "x".repeat(MAX_CONTENT_CHARS + 1);
        assert!(check_content(&too_long).is_err());
        let at_limit: String = "x".repeat(MAX_CONTENT_CHARS);
        assert!(check_content(&at_limit).is_ok());
    }
}
