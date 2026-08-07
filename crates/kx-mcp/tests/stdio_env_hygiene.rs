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

use kx_mcp::{CredentialRef, McpTransport, StdioTransport};
use kx_warrant::SecretScope;

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
    assert!(
        !text.contains("home=none"),
        "substrate vars survive: {text}"
    );
}

/// SEVERAL credentials arrive, each under the name the SERVER expects rather than the name
/// its secret is stored under — the shape any real server needing more than one variable
/// requires, and the one a single credential whose name IS the ref cannot express.
///
/// `/bin/sh` reports its own environment back, so the assertion is on what the child
/// actually received, not on what the builder was told.
#[test]
fn several_credentials_arrive_under_the_names_the_server_expects() {
    // Stored under operator names; read by the server under entirely different ones. If
    // the two namespaces were the same string this test would pass with the target-name
    // plumbing removed, so the difference is the point.
    std::env::set_var("KX_MCP_TEST_STORED_TOKEN", "tok-value-1");
    std::env::set_var("KX_MCP_TEST_STORED_URL", "url-value-2");

    let transport = StdioTransport::new("/bin/sh")
        .arg("-c")
        .arg("read _; echo \"a=${SERVER_TOKEN:-missing} b=${SERVER_API_URL:-missing} stored=${KX_MCP_TEST_STORED_TOKEN:-absent}\"")
        .credential_as(
            "SERVER_TOKEN",
            CredentialRef::from_env_var("KX_MCP_TEST_STORED_TOKEN"),
        )
        .credential_as(
            "SERVER_API_URL",
            CredentialRef::from_env_var("KX_MCP_TEST_STORED_URL"),
        );

    let reply = transport
        .round_trip(b"{}", 4096, 5_000, None)
        .expect("the shell server answers");
    let text = String::from_utf8(reply).unwrap();

    assert!(
        text.contains("a=tok-value-1"),
        "the first credential lands under the server's name: {text}"
    );
    assert!(
        text.contains("b=url-value-2"),
        "and so does the second — TWO variables, which is what was impossible: {text}"
    );
    // The ref's OWN name is not also exported: injection targets the declared variable and
    // nothing else, so a server cannot read the operator's naming by accident.
    assert!(
        text.contains("stored=absent"),
        "the ref's own name is not leaked into the child: {text}"
    );
}

/// The declared secret scope covers every ref, whatever variable each lands in.
///
/// The broker refuses a dispatch whose declared scope exceeds its warrant, so a transport
/// under-declaring here would fail at the FIRE with `SecretScope` — an axis naming neither
/// the connector nor the variable. (That is exactly what happened when the diagnostic
/// fire path derived its warrant from the single legacy credential.)
#[test]
fn the_declared_secret_scope_covers_every_ref() {
    let transport = StdioTransport::new("/bin/sh")
        .credential_as("SERVER_TOKEN", CredentialRef::from_env_var("REF_ONE"))
        .credential_as("SERVER_API_URL", CredentialRef::from_env_var("REF_TWO"));

    match transport.declared_secret_scope() {
        SecretScope::AllowList(refs) => {
            let names: Vec<&str> = refs.iter().map(|r| r.0.as_str()).collect();
            assert!(
                names.contains(&"REF_ONE") && names.contains(&"REF_TWO"),
                "both refs are declared, not just the first: {names:?}"
            );
        }
        // Named rather than a wildcard: `SecretScope` has exactly these two variants, and a
        // wildcard would silently absorb a third that this assertion has not considered.
        SecretScope::None => {
            panic!("a transport carrying two credentials declared NO secret scope at all")
        }
    }
}

/// `Debug` prints NAMES and never a value.
///
/// `envs` carries raw values and is elided only by `finish_non_exhaustive()`; that is an
/// easy thing to undo while adding a field, and nothing would have failed. A planted value
/// makes the elision an assertion — and the ref identities that DO print are checked as
/// present, so this cannot pass by `Debug` having quietly become empty.
#[test]
fn debug_prints_names_and_never_a_value() {
    const PLANTED: &str = "PLAINTEXT-VALUE-THAT-MUST-NOT-PRINT-0123456789";
    let transport = StdioTransport::new("/bin/sh")
        .arg("-c")
        .env("SOME_PLAIN_VAR", PLANTED)
        .credential_as("SERVER_TOKEN", CredentialRef::from_env_var("KX_SOME_REF"));

    let rendered = format!("{transport:?}");
    assert!(
        !rendered.contains(PLANTED),
        "no env VALUE reaches Debug: {rendered}"
    );
    assert!(
        rendered.contains("KX_SOME_REF") && rendered.contains("SERVER_TOKEN"),
        "…while the names that are safe to print DO appear — so the assertion above is \
         not passing on an empty Debug: {rendered}"
    );
}
