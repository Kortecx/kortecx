// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The deterministic tokenizer (lowercase + Unicode alphanumeric runs).

/// The tokenizer version — a structural axis of the persisted cache header and an
/// input to the retrieval-index fingerprint. Bump on any change to lowercasing,
/// splitting, or stopword handling so a stale index is detected, not mis-rebuilt.
pub const TOKENIZER_VERSION: u32 = 1;

/// Tokenize `text` into lowercase terms: maximal runs of `char::is_alphanumeric`,
/// every other character is a delimiter. Unicode-aware via std tables
/// (`to_lowercase` + `is_alphanumeric`), locale-free, deterministic — no external
/// segmentation crate. When `stopwords` is set, a fixed English stoplist is
/// dropped (default off — language-neutral, avoids surprising recall loss).
pub(crate) fn tokenize(text: &str, stopwords: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            // `to_lowercase` can expand one char to several (e.g. ẞ → ss); that is
            // deterministic and fine.
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            push_token(&mut out, std::mem::take(&mut cur), stopwords);
        }
    }
    if !cur.is_empty() {
        push_token(&mut out, cur, stopwords);
    }
    out
}

fn push_token(out: &mut Vec<String>, tok: String, stopwords: bool) {
    if stopwords && is_stopword(&tok) {
        return;
    }
    out.push(tok);
}

/// A compact, fixed English stoplist (only consulted when `stopwords` is enabled).
fn is_stopword(tok: &str) -> bool {
    matches!(
        tok,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "but"
            | "by"
            | "for"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "no"
            | "not"
            | "of"
            | "on"
            | "or"
            | "such"
            | "that"
            | "the"
            | "their"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "to"
            | "was"
            | "will"
            | "with"
    )
}
