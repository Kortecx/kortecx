//! The ONE deliberate client write seam — and it writes the **content store
//! only**, never the journal. Kept out of `reader.rs` on purpose: that module's
//! contract is "a write cannot type-check", and it stays true — [`ContentWriter`]
//! is a separate capability the host wires explicitly for the Batch A
//! `PutContent` path. The journal seam ([`crate::JournalReader`]) still has no
//! `append`, so a journal write remains unrepresentable in gateway-core
//! (illegal-states-unrepresentable, Rule 5.2; the dep-wall test is the second
//! proof).
//!
//! SN-8 holds: the returned ref is **server-derived** (`ContentRef::of` over the
//! payload bytes inside the store) — the client never names an identity. Size
//! caps + auth live above this seam (the service handler caps fail-closed
//! BEFORE touching the store; the host's interceptor authenticates).

use kx_content::{ContentRef, ContentStore};

use crate::error::{internal, GatewayError};

/// The content-store write seam behind `PutContent`. Deliberately minimal: store
/// bytes, report the server-derived ref + whether the blob already existed.
/// There is no delete, no enumeration, and no journal surface on this trait.
pub trait ContentWriter: Send + Sync {
    /// Store `bytes`; returns the server-derived 32-byte blake3 ref and
    /// `true` iff an identical blob was already present (dedup — `put` is
    /// idempotent on the bytes, so this is advisory display state, never
    /// identity).
    ///
    /// # Errors
    /// [`GatewayError::Internal`] on a store write failure.
    fn put(&self, bytes: &[u8]) -> Result<([u8; 32], bool), GatewayError>;
}

/// Blanket impl: any [`ContentStore`] can serve the write seam (the host decides
/// WHICH store instance to expose by what it wires into the service).
impl<S: ContentStore + Send + Sync> ContentWriter for S {
    fn put(&self, bytes: &[u8]) -> Result<([u8; 32], bool), GatewayError> {
        // `put` is idempotent on the bytes, so pre-checking membership is purely
        // advisory (a concurrent identical put may race this to `false`; the
        // stored object is identical either way).
        let deduplicated = ContentStore::contains(self, &ContentRef::of(bytes));
        let stored = ContentStore::put(self, bytes).map_err(internal)?;
        Ok((stored.0, deduplicated))
    }
}

#[cfg(test)]
mod tests {
    use kx_content::{ContentRef, InMemoryContentStore};

    use super::ContentWriter;

    #[test]
    fn put_returns_server_derived_ref_and_dedup_flag() {
        let store = InMemoryContentStore::new();
        let (r1, dedup1) = ContentWriter::put(&store, b"payload").unwrap();
        assert_eq!(r1, ContentRef::of(b"payload").0, "ref is server-derived");
        assert!(!dedup1, "first put is not a duplicate");

        let (r2, dedup2) = ContentWriter::put(&store, b"payload").unwrap();
        assert_eq!(r1, r2, "idempotent: same bytes, same ref");
        assert!(dedup2, "second identical put reports dedup");
    }
}
