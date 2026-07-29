//! An MCP stdio server never inherits the HOST's environment.
//!
//! The gateway's env can carry operator secrets an MCP server was never granted
//! — D81 exists precisely so a server sees only the credentials DECLARED for it,
//! yet the spawn used to hand every server the whole parent environment. The
//! transport now clears the child env to a minimal substrate allowlist; what a
//! server legitimately needs arrives EXPLICITLY (the connection's own `env`
//! entries and its injected credentials).
//!
//! Driven through the REAL `StdioTransport::round_trip` with `/bin/sh` as the
//! server so the child can report its own environment verbatim.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

use kx_mcp::{McpTransport, StdioTransport};

#[test]
fn a_parent_canary_never_reaches_the_child_but_an_explicit_env_does() {
    // Uniquely named; nothing else reads it, so the process-global set is benign.
    std::env::set_var("KX_MCP_TEST_CANARY", "leak-me-if-you-can");

    let transport = StdioTransport::new("/bin/sh")
        .arg("-c")
        // Read a line from stdin (the request), then report both variables.
        .arg("read _; echo \"canary=${KX_MCP_TEST_CANARY:-absent} explicit=${KX_MCP_EXPLICIT:-missing} home=${HOME:-none}\"")
        .env("KX_MCP_EXPLICIT", "granted");

    let reply = transport
        .round_trip(b"{}", 4096, 5_000, None)
        .expect("the shell server answers");
    let text = String::from_utf8(reply).unwrap();

    assert!(
        text.contains("canary=absent"),
        "the parent's env must NOT reach the server: {text}"
    );
    assert!(
        text.contains("explicit=granted"),
        "the connection's DECLARED env must reach it: {text}"
    );
    // The substrate allowlist keeps interpreters bootable (HOME survives).
    assert!(!text.contains("home=none"), "substrate vars survive: {text}");
}
