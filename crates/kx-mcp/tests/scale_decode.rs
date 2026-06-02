//! Scale-smoke (run explicitly: `cargo test -p kx-mcp --release -- --ignored`).
//! The decoder stays linear on a near-cap valid result and rejects an over-cap
//! body in O(1) (the size check precedes any parse).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_mcp::decode_tool_result;

#[test]
#[ignore = "scale-smoke: run with --release --ignored"]
fn decode_handles_near_cap_and_rejects_over_cap() {
    let cap = 8 * 1024 * 1024; // 8 MiB
    let filler = "x".repeat(cap - 1024);
    let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"t":"{filler}"}}}}"#);
    assert!(body.len() < cap, "near-cap body fits the cap");

    let out = decode_tool_result(body.as_bytes(), cap).expect("near-cap valid result decodes");
    assert!(out.len() > cap - 2048, "the full result object is returned");

    // Over-cap: rejected on the length check, before any allocation/parse (O(1)).
    let over = vec![b'x'; cap + 1];
    assert!(decode_tool_result(&over, cap).is_err());
}
