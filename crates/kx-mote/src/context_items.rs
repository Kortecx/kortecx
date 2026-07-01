//! PR-7 context-bundle item refs carried in a Mote's `config_subset` under
//! [`crate::CONTEXT_ITEMS_KEY`].
//!
//! The bind layer resolves a run's attached bundle handles to their item
//! `(name, content_ref)` pairs and encodes them HERE — canonical (sorted +
//! de-duplicated) — into the ENTRY Mote's identity-bearing `config_subset`, so a
//! different attached context yields a different `MoteId` (exactly-once-per-
//! `(input + context)`). The context-assembler decodes the SAME bytes to fetch +
//! label the blobs for the model. A Mote WITHOUT the key is byte-identical to
//! pre-PR-7 — the canonical projection digest is untouched (the reference run
//! attaches no bundle, so the key never appears and this codec is never invoked
//! on the identity path).
//!
//! Encoding is a dependency-free, deterministic length-prefixed concatenation
//! (NOT JSON — `kx-mote` stays off `serde_json`): for each item, in canonical
//! order, `u32-le(name.len()) ‖ name bytes ‖ 32-byte content_ref`.

/// One context item: an advisory label + the 32-byte content-store ref of a blob
/// already in the content store (a `PutContent` ref). The label is display-only;
/// identity is the ref + label set the bind layer canonicalises here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextItemRef {
    /// Advisory label / context heading (display only).
    pub name: String,
    /// The 32-byte blake3 content-store ref of the item's blob.
    pub content_ref: [u8; 32],
}

/// Encode context items into the canonical `config_subset[CONTEXT_ITEMS_KEY]`
/// value: items are sorted by `(content_ref, name)` and de-duplicated, then each
/// is emitted as `u32-le(name.len()) ‖ name ‖ content_ref[32]`. Deterministic ⇒
/// identical context yields identical bytes ⇒ identical entry `MoteId`.
#[must_use]
pub fn encode_context_items(items: &[ContextItemRef]) -> Vec<u8> {
    let mut sorted: Vec<&ContextItemRef> = items.iter().collect();
    sorted.sort_by(|a, b| {
        a.content_ref
            .cmp(&b.content_ref)
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted.dedup_by(|a, b| a.content_ref == b.content_ref && a.name == b.name);
    let mut out = Vec::new();
    for it in sorted {
        let name = it.name.as_bytes();
        out.extend_from_slice(&u32::try_from(name.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&it.content_ref);
    }
    out
}

/// Encode context items PRESERVING their input order (RC4c-2b) — the SAME wire format
/// as [`encode_context_items`] (`u32-le(name.len()) ‖ name ‖ content_ref[32]` per item)
/// but WITHOUT the canonical sort/dedup, so a RERANKED order survives round-trip
/// ([`decode_context_items`] returns items in encoded order, and the serve assembler
/// renders in that order). Used ONLY for the off-digest, OUT-OF-BAND reranked-delivery
/// bundle (the RC4c-2b chat-rag suppression gate) — NEVER for an identity-bearing entry
/// config, which must stay canonical (a different order must not change a `MoteId`).
#[must_use]
pub fn encode_context_items_ordered(items: &[ContextItemRef]) -> Vec<u8> {
    let mut out = Vec::new();
    for it in items {
        let name = it.name.as_bytes();
        out.extend_from_slice(&u32::try_from(name.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&it.content_ref);
    }
    out
}

/// Decode the canonical value back into items, in canonical (encoded) order. A
/// malformed / truncated buffer decodes the items it can and stops (total +
/// panic-free); the assembler treats the result as advisory context (the
/// content store still gates every fetch by ref).
#[must_use]
pub fn decode_context_items(bytes: &[u8]) -> Vec<ContextItemRef> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        let len = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        i += 4;
        if i + len + 32 > bytes.len() {
            break; // truncated — stop fail-soft.
        }
        let name = String::from_utf8_lossy(&bytes[i..i + len]).into_owned();
        i += len;
        let mut content_ref = [0u8; 32];
        content_ref.copy_from_slice(&bytes[i..i + 32]);
        i += 32;
        out.push(ContextItemRef { name, content_ref });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(tag: u8, name: &str) -> ContextItemRef {
        ContextItemRef {
            name: name.to_string(),
            content_ref: [tag; 32],
        }
    }

    #[test]
    fn round_trips_in_canonical_order() {
        let items = vec![item(0x22, "b"), item(0x11, "a")];
        let enc = encode_context_items(&items);
        let dec = decode_context_items(&enc);
        // Canonical order = sorted by ref ⇒ a (0x11) before b (0x22).
        assert_eq!(dec, vec![item(0x11, "a"), item(0x22, "b")]);
    }

    #[test]
    fn encoding_is_order_independent_and_dedups() {
        let a = encode_context_items(&[item(0x11, "a"), item(0x22, "b")]);
        let b = encode_context_items(&[item(0x22, "b"), item(0x11, "a")]);
        assert_eq!(a, b, "input order does not change the canonical encoding");
        let with_dupe = encode_context_items(&[item(0x11, "a"), item(0x11, "a"), item(0x22, "b")]);
        assert_eq!(with_dupe, a, "exact duplicates collapse");
    }

    #[test]
    fn distinct_context_yields_distinct_bytes() {
        assert_ne!(
            encode_context_items(&[item(0x11, "a")]),
            encode_context_items(&[item(0x11, "a"), item(0x22, "b")]),
        );
        assert_ne!(
            encode_context_items(&[item(0x11, "a")]),
            encode_context_items(&[item(0x11, "b")]),
            "a different label is a different context",
        );
    }

    #[test]
    fn empty_is_empty() {
        assert!(encode_context_items(&[]).is_empty());
        assert!(decode_context_items(&[]).is_empty());
    }

    #[test]
    fn truncated_buffer_decodes_fail_soft() {
        let enc = encode_context_items(&[item(0x11, "abc")]);
        // Drop the last byte — the item is incomplete, decode yields nothing.
        assert!(decode_context_items(&enc[..enc.len() - 1]).is_empty());
    }

    #[test]
    fn handles_unicode_and_long_labels() {
        let items = vec![item(0x05, "café — notes"), item(0x06, &"x".repeat(2000))];
        assert_eq!(decode_context_items(&encode_context_items(&items)).len(), 2);
    }
}
