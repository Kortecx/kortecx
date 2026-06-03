//! M5.2b — D81 over the HTTP header path. A credential bound to the transport is
//! injected as an `Authorization` header (genuinely in-play: the server reports it
//! saw the header), yet the secret VALUE reaches NONE of the runtime sinks: the
//! `EffectRequest.payload`, the `BrokerHandle`, the staged result (journal/content
//! store), the `MoteId`, or any `Debug`. The server reports only WHETHER the header
//! was present, never its value.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::{effect_egress, sample_mote, tool, warrant_granting_egress, HttpMode, MockHttpServer};
use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, InMemoryContentStore};
use kx_mcp::{CredentialRef, HttpTransport, McpCapability, SecretRef};
use kx_tool_registry::McpEndpointId;
use kx_warrant::SecretScope;

const SECRET: &str = "SUPER_SECRET_sk-HTTP-DEADBEEF-do-not-leak-0123456789";
const CRED_VAR: &str = "KX_MCP_TEST_CRED_HTTP_SECRETS_LEAK";

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn credential_ref_redacts_in_debug_and_display() {
    let cred = CredentialRef::from_env_var(CRED_VAR);
    assert!(!format!("{cred:?}").contains(SECRET));
    assert!(!format!("{cred}").contains(SECRET));
}

#[test]
fn http_secret_reaches_no_runtime_sink() {
    std::env::set_var(CRED_VAR, SECRET);

    let server = MockHttpServer::start(HttpMode::AuthProbe);
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());

    let transport = HttpTransport::new(&server.url(), &server.net_scope(), false)
        .unwrap()
        .header_credential("Authorization", CredentialRef::from_env_var(CRED_VAR));
    let cap = McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId(server.url()),
        "echo",
        Box::new(transport),
    );
    // The transport's Debug must not carry the secret value (only the var name).
    assert!(!format!("{cap:?}").contains(SECRET));
    broker.register_capability(Box::new(cap));

    let mote = sample_mote(&name, &version);
    // The role grants the secret the header credential needs (D110.3).
    let mut warrant = warrant_granting_egress(&name, &version);
    warrant.secret_scope =
        SecretScope::AllowList(BTreeSet::from([SecretRef(CRED_VAR.to_string())]));
    let req = effect_egress(r#"{"q":"hello"}"#);
    assert!(
        !contains(&req.payload, SECRET.as_bytes()),
        "payload free of secret"
    );

    let handle = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect("dispatch succeeds with the credential injected");

    // PROOF the credential was genuinely in-play: the server saw the Authorization
    // header (it reports presence, never the value).
    let staged = store.get(&handle.staged_ref).unwrap();
    assert_eq!(
        &*staged, br#"{"saw_auth":true}"#,
        "the server received the injected Authorization header"
    );

    // The secret VALUE appears in NO sink.
    assert!(
        !contains(&staged, SECRET.as_bytes()),
        "staged bytes free of secret"
    );
    assert!(
        !format!("{handle:?}").contains(SECRET),
        "BrokerHandle free of secret"
    );
    assert!(
        !contains(mote.id.as_bytes(), SECRET.as_bytes()),
        "MoteId free of secret"
    );

    std::env::remove_var(CRED_VAR);
}
