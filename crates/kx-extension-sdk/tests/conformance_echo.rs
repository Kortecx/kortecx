//! The conformance harness, exercised against the reference connector
//! (`kx-connector-example`, the deterministic positive control) plus the
//! leak-scanner negative control.
//!
//! The reference connector is a `[[bin]]` of THIS crate, so
//! `CARGO_BIN_EXE_kx-connector-example` is always set for these integration tests —
//! no skip path is needed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_extension_sdk::conformance::{contains_secret, run_conformance, ConnectorUnderTest};
use kx_extension_sdk::prelude::{SessionMode, TransportSpec};

/// Path to the reference connector binary (built for this crate's integration tests).
fn reference_connector(name: &str, credential_ref: Option<String>) -> ConnectorUnderTest {
    ConnectorUnderTest {
        name: name.into(),
        transport: TransportSpec::Stdio {
            command: env!("CARGO_BIN_EXE_kx-connector-example").to_string(),
            args: vec![],
        },
        credential_ref,
        session_mode: SessionMode::Stateless,
    }
}

/// The leak scanner must catch a planted secret and clear a clean buffer. This is
/// the negative control that proves item 7 actually bites — it needs no binary.
#[test]
fn leak_scanner_detects_and_clears() {
    assert!(contains_secret(b"left-SUPER_SECRET-right", b"SUPER_SECRET"));
    assert!(contains_secret(b"SUPER_SECRET", b"SUPER_SECRET"));
    assert!(!contains_secret(b"nothing to see here", b"SUPER_SECRET"));
    // An empty needle never "matches" (a connector with no credential is not a leak).
    assert!(!contains_secret(b"anything", b""));
}

/// The reference connector passes every gate item (out-of-process · warrant · secret · on/off).
#[test]
fn reference_connector_passes_conformance() {
    let cut = reference_connector("example", None);
    let report = run_conformance(&cut);
    assert!(
        report.reachable,
        "connector should be reachable: {report:#?}"
    );
    assert!(
        report.discovered >= 1,
        "connector should expose >= 1 tool: {report:#?}"
    );
    assert!(report.passed(), "connector failed conformance: {report:#?}");
    // Every gate item is represented + passed.
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

/// A credential supplied out-of-band reaches NO sink — the reference connector
/// "never echoes its environment" (D81). Drives the item-7 leak scan with a real
/// secret genuinely in play.
#[test]
fn credentialed_connector_does_not_leak_the_secret() {
    const SECRET: &str = "SUPER_SECRET_sk-DEADBEEF-do-not-leak-0123456789";
    const VAR: &str = "KX_EXTENSION_SDK_CONFORMANCE_CRED";
    std::env::set_var(VAR, SECRET);

    let cut = reference_connector("example", Some(VAR.to_string()));
    let report = run_conformance(&cut);
    std::env::remove_var(VAR);

    let secret_check = report
        .checks
        .iter()
        .find(|c| c.gate_item == 7)
        .expect("a secret-by-ref check");
    assert!(
        secret_check.passed,
        "the credentialed connector leaked its secret: {report:#?}"
    );
    assert!(
        report.passed(),
        "credentialed connector failed conformance: {report:#?}"
    );
}
