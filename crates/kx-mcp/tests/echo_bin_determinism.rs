//! PR-2d-2 — the BUNDLED `kx-mcp-echo` production tool (the bin the live
//! `kx serve` `ReAct` loop fires) is deterministic in the request args: identical
//! args ⇒ identical reply bytes ⇒ identical content-addressed `staged_ref`
//! (the exactly-once contract at the world boundary, D58 §7). Distinct args ⇒
//! distinct bytes. Exercised through the REAL `StdioTransport` + the REAL
//! `LocalCapabilityBroker` warrant gate — the exact path the live worker drives.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{effect, sample_mote, tool, warrant_granting};
use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, InMemoryContentStore};
use kx_mcp::{McpCapability, StdioTransport};
use kx_tool_registry::McpEndpointId;

/// Absolute path to the SHIPPED echo tool (a `[[bin]]` of this crate, so Cargo
/// exports its path to the integration tests).
const ECHO_BIN: &str = env!("CARGO_BIN_EXE_kx-mcp-echo");

fn shipped_echo_capability() -> McpCapability {
    let (name, version) = tool();
    McpCapability::new(
        name,
        version,
        McpEndpointId("stdio://kx-mcp-echo".into()),
        "echo",
        Box::new(StdioTransport::new(ECHO_BIN)),
    )
}

#[test]
fn identical_args_stage_identical_bytes_and_ref() {
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(shipped_echo_capability()));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version);

    let first = broker
        .dispatch(&mote, &warrant, &name, effect(r#"{"q":"x"}"#))
        .expect("first dispatch");
    let second = broker
        .dispatch(&mote, &warrant, &name, effect(r#"{"q":"x"}"#))
        .expect("second dispatch (the crash-recovery re-dispatch shape)");

    // Deterministic-in-args: byte-identical reply ⇒ the SAME content ref (the
    // content-addressed dedup that makes a re-dispatch exactly-once).
    assert_eq!(first.staged_ref, second.staged_ref);
    let staged = store.get(&first.staged_ref).unwrap();
    assert_eq!(&*staged, br#"{"echoed":{"q":"x"}}"#);
}

#[test]
fn distinct_args_stage_distinct_bytes() {
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(shipped_echo_capability()));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version);

    let a = broker
        .dispatch(&mote, &warrant, &name, effect(r#"{"q":"a"}"#))
        .expect("dispatch a");
    let b = broker
        .dispatch(&mote, &warrant, &name, effect(r#"{"q":"b"}"#))
        .expect("dispatch b");

    assert_ne!(a.staged_ref, b.staged_ref);
    assert_eq!(
        &*store.get(&a.staged_ref).unwrap(),
        br#"{"echoed":{"q":"a"}}"#
    );
    assert_eq!(
        &*store.get(&b.staged_ref).unwrap(),
        br#"{"echoed":{"q":"b"}}"#
    );
}
