//! D66 — MCP effects are world-mutating by default: `McpCapability` honors ONLY
//! `StageThenCommit`, and the broker gate refuses any other pattern.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{echo_capability, effect, sample_mote, tool, warrant_granting};
use kx_capability::{BrokerError, Capability, CapabilityBroker, LocalCapabilityBroker};
use kx_content::InMemoryContentStore;
use kx_mote::EffectPattern;

#[test]
fn supports_only_stage_then_commit() {
    let (name, version) = tool();
    let cap = echo_capability(name, version);
    assert_eq!(
        cap.supported_patterns(),
        &[EffectPattern::StageThenCommit][..],
        "MCP effects are world-mutating by default (D66)"
    );
}

#[test]
fn unsupported_pattern_is_refused_by_the_gate() {
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(echo_capability(name.clone(), version.clone())));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version);
    let mut req = effect("{}");
    req.pattern = EffectPattern::IdempotentByConstruction; // not in supported_patterns

    let err = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect_err("an unsupported pattern must be refused");
    assert!(matches!(err, BrokerError::UnsupportedPattern { .. }));
}
