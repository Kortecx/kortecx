// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Build an RFC-2822 message and base64url-encode it for the Gmail `raw` field.

use base64::Engine;

use crate::gmail::GmailError;

/// Build a minimal RFC-2822 `text/plain` message and return its base64url encoding
/// (the shape Gmail's `drafts.create` / `messages.send` want in the `raw` field).
///
/// # Errors
/// Returns [`GmailError::BadArgs`] if `to` or `subject` contain a CR or LF byte
/// (header injection — a newline in a header field could smuggle extra headers).
pub fn build_raw(to: &str, subject: &str, body: &str) -> Result<String, GmailError> {
    if has_crlf(to) || has_crlf(subject) {
        return Err(GmailError::BadArgs(
            "the `to` and `subject` fields must not contain CR or LF".to_string(),
        ));
    }
    let msg = format!(
        "To: {to}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=\"UTF-8\"\r\n\r\n{body}"
    );
    Ok(base64::engine::general_purpose::URL_SAFE.encode(msg.as_bytes()))
}

fn has_crlf(s: &str) -> bool {
    s.contains('\r') || s.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::build_raw;
    use base64::Engine;

    #[test]
    fn round_trips_headers_and_body() {
        let raw = build_raw("a@b.com", "Hello", "Body text").unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(raw)
            .unwrap();
        let msg = String::from_utf8(bytes).unwrap();
        assert!(msg.contains("To: a@b.com"));
        assert!(msg.contains("Subject: Hello"));
        assert!(msg.ends_with("Body text"));
    }

    #[test]
    fn rejects_header_injection() {
        assert!(build_raw("a@b.com\r\nBcc: evil@x.com", "Hi", "x").is_err());
        assert!(build_raw("a@b.com", "Hi\nX-Injected: 1", "x").is_err());
    }
}
