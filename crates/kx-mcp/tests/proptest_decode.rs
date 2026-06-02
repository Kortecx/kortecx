//! IMP-5 / IMP-16 — the fail-closed inbound decoder is TOTAL over arbitrary +
//! truncated bytes (never panics) and always size-capped. SN-4: ≥64 cases.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_mcp::{decode_tool_result, DecodeError};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary bytes never panic; the result is always `Ok` or a typed `Err`.
    #[test]
    fn decode_is_total_over_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..1024),
        cap in 0usize..2048,
    ) {
        let _ = decode_tool_result(&bytes, cap);
    }

    /// Any cap below the input length is caught as `Oversize`, content-independent.
    #[test]
    fn oversize_is_always_caught(bytes in proptest::collection::vec(any::<u8>(), 1..512)) {
        let cap = bytes.len() - 1;
        let got = decode_tool_result(&bytes, cap);
        prop_assert!(
            matches!(got, Err(DecodeError::Oversize { .. })),
            "any cap below the input length must be caught as Oversize"
        );
    }

    /// Every prefix of a well-formed response decodes without panicking (the full
    /// string is valid; truncations are refused fail-closed, never crash).
    #[test]
    fn every_prefix_of_a_valid_response_is_safe(n in 0usize..90) {
        const FULL: &[u8] =
            br#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ok"}]}}"#;
        let end = n.min(FULL.len());
        let _ = decode_tool_result(&FULL[..end], 4096);
    }
}
