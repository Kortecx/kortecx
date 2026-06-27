// SPDX-License-Identifier: Apache-2.0
//! `just test-connector [endpoint…]` — dial a connector through the gateway path and
//! run the bundled subset of the D167 Extension Acceptance Gate, printing a
//! per-check report (and the machine-readable JSON) and exiting non-zero on failure.
//!
//! Usage:
//!   cargo run -p kx-extension-sdk --example conformance                 # bundled echo
//!   cargo run -p kx-extension-sdk --example conformance -- ./kx-mcp-echo
//!   cargo run -p kx-extension-sdk --example conformance -- npx -y @modelcontextprotocol/server-filesystem /tmp/x
//!   cargo run -p kx-extension-sdk --example conformance -- https://mcp.example.com/rpc

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::doc_markdown
)]

use kx_extension_sdk::conformance::{reference_connector, run_conformance, ConnectorUnderTest};
use kx_extension_sdk::prelude::{SessionMode, TransportSpec};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cred = std::env::var("KX_CONNECTOR_CRED_REF").ok();

    let cut = if args.is_empty() {
        let Some(c) = reference_connector() else {
            eprintln!(
                "no endpoint given and the reference connector was not found.\n\
                 Build it (cargo build -p kx-extension-sdk) or pass an endpoint:\n  \
                 just test-connector ./my-mcp-server"
            );
            std::process::exit(2);
        };
        c
    } else if args[0].starts_with("http://") || args[0].starts_with("https://") {
        let url = args[0].clone();
        ConnectorUnderTest {
            name: "under-test".into(),
            transport: TransportSpec::Http {
                tls_required: url.starts_with("https://"),
                url,
            },
            credential_ref: cred,
            session_mode: SessionMode::Stateless,
        }
    } else {
        ConnectorUnderTest {
            name: "under-test".into(),
            transport: TransportSpec::Stdio {
                command: args[0].clone(),
                args: args[1..].to_vec(),
            },
            credential_ref: cred,
            session_mode: SessionMode::Stateless,
        }
    };

    let report = run_conformance(&cut);

    println!("connector: {}", report.connector);
    println!(
        "reachable: {}  discovered: {}",
        report.reachable, report.discovered
    );
    for c in &report.checks {
        let mark = if c.passed { "PASS" } else { "FAIL" };
        println!(
            "  [{mark}] gate {:>2}  {:<20} {}",
            c.gate_item, c.name, c.detail
        );
    }
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        println!("\n{json}");
    }

    if report.passed() {
        println!("\nCONFORMANCE: PASS");
    } else {
        eprintln!("\nCONFORMANCE: FAIL");
        std::process::exit(1);
    }
}
