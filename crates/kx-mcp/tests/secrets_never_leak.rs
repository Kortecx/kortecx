//! D81 / IMP-15 — a credential supplied out-of-band reaches NONE of the runtime
//! sinks: the `EffectRequest.payload`, the `BrokerHandle` provenance, the staged
//! result bytes (the journal/content store), or the `MoteId`. The credential
//! reference itself also never prints the secret value.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::{effect, sample_mote, tool, warrant_granting, MOCK_SERVER};
use kx_capability::{BrokerError, CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, InMemoryContentStore};
use kx_mcp::{CredentialRef, McpCapability, SecretRef, StdioTransport};
use kx_tool_registry::McpEndpointId;
use kx_warrant::{SecretScope, WarrantField};

/// Distinctive secret value that must never appear in any runtime sink.
const SECRET: &str = "SUPER_SECRET_sk-DEADBEEF-do-not-leak-0123456789";
/// The env var that "holds" the secret (the credential identity).
const CRED_VAR: &str = "KX_MCP_TEST_CRED_SECRETS_LEAK";

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A `secret_scope` granting exactly the test credential (D110.3).
fn grants_secret() -> SecretScope {
    SecretScope::AllowList(BTreeSet::from([SecretRef(CRED_VAR.to_string())]))
}

#[test]
fn credential_ref_redacts_the_secret_in_debug_and_display() {
    // The reference prints only its identity (the var name), never the value.
    let cred = CredentialRef::from_env_var(CRED_VAR);
    assert_eq!(cred.identity(), CRED_VAR);
    assert!(!format!("{cred:?}").contains(SECRET));
    assert!(!format!("{cred}").contains(SECRET));
}

#[test]
fn secret_reaches_no_runtime_sink() {
    // The secret lives in the runtime's environment and is referenced by the
    // capability's credential — it is genuinely "in play" for this dispatch.
    std::env::set_var(CRED_VAR, SECRET);

    let (name, version) = tool();
    let transport = Box::new(
        StdioTransport::new(MOCK_SERVER).credential(CredentialRef::from_env_var(CRED_VAR)),
    );
    let cap = McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId("stdio://mock".into()),
        "echo",
        transport,
    );

    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(cap));

    let mote = sample_mote(&name, &version);
    // The role grants the secret the capability needs (D110.3); without this the
    // broker would refuse the dispatch (see the gate test below).
    let mut warrant = warrant_granting(&name, &version);
    warrant.secret_scope = grants_secret();
    let req = effect(r#"{"q":"hi"}"#);
    let payload = req.payload.clone();

    let handle = broker.dispatch(&mote, &warrant, &name, req).unwrap();
    let staged = store.get(&handle.staged_ref).unwrap();

    let secret = SECRET.as_bytes();
    // (1) EffectRequest.payload (the tool args) — never the secret.
    assert!(
        !contains(&payload, secret),
        "secret leaked into EffectRequest.payload"
    );
    // (2) BrokerHandle provenance — records the capability identity, never the secret.
    assert!(
        !format!("{handle:?}").contains(SECRET),
        "secret leaked into BrokerHandle provenance"
    );
    // (3) The staged result bytes (what the journal commits / the content store holds).
    assert!(
        !contains(&staged, secret),
        "secret leaked into the staged result"
    );
    // (4) The MoteId.
    assert!(
        !contains(mote.id.as_bytes(), secret),
        "secret leaked into the MoteId"
    );

    std::env::remove_var(CRED_VAR);
}

/// D110.3 — data minimization: a role that does NOT grant the capability's
/// secret is refused at dispatch (the capability declares `required_secret_scope`
/// = its configured credential; the broker gates it `⊆ warrant.secret_scope`).
/// The secret is never resolved, and the model never sees it.
#[test]
fn dispatch_refused_when_warrant_does_not_grant_the_secret() {
    let (name, version) = tool();
    let transport = Box::new(
        StdioTransport::new(MOCK_SERVER).credential(CredentialRef::from_env_var(CRED_VAR)),
    );
    let cap = McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId("stdio://mock".into()),
        "echo",
        transport,
    );

    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(cap));

    let mote = sample_mote(&name, &version);
    // The role grants the tool but NOT the secret (`secret_scope: None`).
    let warrant = warrant_granting(&name, &version);
    assert_eq!(warrant.secret_scope, SecretScope::None);

    let err = broker
        .dispatch(&mote, &warrant, &name, effect(r#"{"q":"hi"}"#))
        .expect_err("a role that does not grant the secret must be refused");
    assert!(matches!(
        err,
        BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::SecretScope
        }
    ));
}
