// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Fail-closed argument guards for the Notion REST surface.
//!
//! The connector interpolates page/block ids into URL path segments
//! (`/pages/{id}`, `/blocks/{id}/children`), so ids are validated as Notion **UUIDs**
//! (ASCII hex digits + `-`, dashed or bare) BEFORE any request is built — a `/`, `?`,
//! `#`, or `..` can never smuggle into the path. Page/block text is bounded to
//! Notion's 2000-character rich-text limit. Both reject fail-closed with
//! [`NotionError::BadArgs`] and never carry the credential.

use crate::notion::NotionError;

/// Notion's per-rich-text-object content limit (characters, not bytes).
pub const MAX_TEXT_CHARS: usize = 2000;

/// Validate a Notion id (a page/block/parent id): non-empty and composed only of
/// ASCII hex digits and `-` (Notion accepts both the dashed UUID form and the bare
/// 32-hex form). This is also the path-injection guard — such an id cannot contain a
/// path separator or query/fragment delimiter.
///
/// # Errors
/// [`NotionError::BadArgs`] when `id` is empty or contains a non-hex, non-`-` byte.
pub fn check_uuid(field: &str, id: &str) -> Result<(), NotionError> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(NotionError::BadArgs(format!(
            "`{field}` must be a Notion id (hex digits and dashes only)"
        )));
    }
    Ok(())
}

/// Validate page/block text: non-empty and within Notion's 2000-char limit.
///
/// # Errors
/// [`NotionError::BadArgs`] when `text` is empty or exceeds [`MAX_TEXT_CHARS`].
pub fn check_text(field: &str, text: &str) -> Result<(), NotionError> {
    if text.is_empty() {
        return Err(NotionError::BadArgs(format!("`{field}` must not be empty")));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(NotionError::BadArgs(format!(
            "`{field}` exceeds Notion's {MAX_TEXT_CHARS}-character limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_text, check_uuid, MAX_TEXT_CHARS};

    #[test]
    fn uuid_accepts_dashed_and_bare_hex() {
        assert!(check_uuid("page_id", "0123456789abcdef0123456789abcdef").is_ok());
        assert!(check_uuid("page_id", "01234567-89ab-cdef-0123-456789abcdef").is_ok());
    }

    #[test]
    fn uuid_rejects_empty_and_non_hex() {
        assert!(check_uuid("page_id", "").is_err());
        assert!(check_uuid("page_id", "01/23").is_err());
        assert!(check_uuid("page_id", "../secrets").is_err());
        assert!(check_uuid("page_id", "not-a-uuid-zzzz").is_err());
    }

    #[test]
    fn text_bounds_are_enforced() {
        assert!(check_text("text", "hi").is_ok());
        assert!(check_text("text", "").is_err());
        let too_long: String = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(check_text("text", &too_long).is_err());
        let at_limit: String = "x".repeat(MAX_TEXT_CHARS);
        assert!(check_text("text", &at_limit).is_ok());
    }
}
