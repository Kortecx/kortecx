// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Conformance: the bundled Slack connector passes the Extension Acceptance Gate
//! subset (out-of-process · warrant/SN-8 · secret-by-ref · on/off), driven OFFLINE
//! (`KX_SLACK_FAKE`) so it needs no Slack credentials and no network.
//!
//! The connector is a `[[bin]]` of THIS crate, so `CARGO_BIN_EXE_kx-connector-slack`
//! is always set for these integration tests — no skip path is needed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_extension_sdk::conformance::{run_conformance, ConnectorUnderTest};
use kx_extension_sdk::prelude::{SessionMode, TransportSpec};

fn slack_connector(credential_ref: Option<String>) -> ConnectorUnderTest {
    ConnectorUnderTest {
        name: "slack".into(),
        transport: TransportSpec::Stdio {
            command: env!("CARGO_BIN_EXE_kx-connector-slack").to_string(),
            args: vec![],
        },
        credential_ref,
        session_mode: SessionMode::Stateless,
    }
}

/// Offline, with a distinctive credential in play: the Slack connector is reachable,
/// exposes its four tools, fires under a correct warrant (and is refused without
/// one), and never echoes the injected secret. Combined into one test so the
/// process-global `KX_SLACK_FAKE` / credential env vars are set/cleared serially.
#[test]
fn slack_connector_passes_conformance_offline() {
    const SECRET: &str = "SEKRET-xoxb-DEADBEEF-do-not-leak-0123456789";
    const CRED_VAR: &str = "KX_SLACK_CREDENTIAL";

    std::env::set_var("KX_SLACK_FAKE", "1");
    std::env::set_var(CRED_VAR, SECRET);

    let cut = slack_connector(Some(CRED_VAR.to_string()));
    let report = run_conformance(&cut);

    std::env::remove_var(CRED_VAR);
    std::env::remove_var("KX_SLACK_FAKE");

    assert!(
        report.reachable,
        "connector should be reachable: {report:#?}"
    );
    assert!(
        report.discovered >= 3,
        "expected the 4 slack tools: {report:#?}"
    );
    assert!(
        report.passed(),
        "slack connector failed conformance: {report:#?}"
    );

    // Every gate item (out-of-process · warrant · secret-by-ref · on/off) present + passed.
    for item in [3u8, 5, 7, 10] {
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.gate_item == item && c.passed),
            "gate item {item} missing or failed: {report:#?}"
        );
    }
}
