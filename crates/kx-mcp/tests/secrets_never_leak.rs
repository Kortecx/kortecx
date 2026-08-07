//! A credential supplied out-of-band reaches NONE of the runtime
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

/// The same sweep with SEVERAL credentials, each landing in a variable named differently
/// from the ref that supplies it — the environment-map shape.
///
/// Two things could have gone wrong once a target name sits beside each credential, and
/// neither would have failed the single-credential sweep above: a second secret could reach
/// a sink the first does not, and the target NAMES could travel with their values. Both are
/// checked here, and the scanner is proved able to see a secret at all before either
/// assertion is trusted.
#[test]
fn several_secrets_under_other_names_reach_no_runtime_sink() {
    const SECRET_A: &str = "SUPER_SECRET_sk-AAAA-multi-do-not-leak-1111111111";
    const SECRET_B: &str = "SUPER_SECRET_sk-BBBB-multi-do-not-leak-2222222222";
    const REF_A: &str = "KX_MCP_TEST_MULTI_REF_A";
    const REF_B: &str = "KX_MCP_TEST_MULTI_REF_B";

    std::env::set_var(REF_A, SECRET_A);
    std::env::set_var(REF_B, SECRET_B);

    let (name, version) = tool();
    let transport = Box::new(
        StdioTransport::new(MOCK_SERVER)
            .credential_as("SERVER_TOKEN", CredentialRef::from_env_var(REF_A))
            .credential_as("SERVER_API_URL", CredentialRef::from_env_var(REF_B)),
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
    let mut warrant = warrant_granting(&name, &version);
    warrant.secret_scope = SecretScope::AllowList(BTreeSet::from([
        SecretRef(REF_A.to_string()),
        SecretRef(REF_B.to_string()),
    ]));
    let req = effect(r#"{"q":"hi"}"#);
    let payload = req.payload.clone();

    let handle = broker.dispatch(&mote, &warrant, &name, req).unwrap();
    let staged = store.get(&handle.staged_ref).unwrap();
    let provenance = format!("{handle:?}");

    // ⚠ THE CONTROL FIRST. Every assertion below is an ABSENCE, and an absence proves
    // nothing unless the same search can find the thing when it IS there. Plant each
    // secret in a buffer of the same shape and require the scan to catch it — otherwise a
    // `contains` that never matches anything would read as perfect hygiene.
    for planted in [SECRET_A, SECRET_B] {
        let bait = format!("prefix {planted} suffix").into_bytes();
        assert!(
            contains(&bait, planted.as_bytes()),
            "the scanner can see {planted} when it is present — without this the \
             absences below are vacuous"
        );
    }

    for secret in [SECRET_A, SECRET_B] {
        let bytes = secret.as_bytes();
        assert!(
            !contains(&payload, bytes),
            "{secret} leaked into EffectRequest.payload"
        );
        assert!(
            !provenance.contains(secret),
            "{secret} leaked into BrokerHandle provenance"
        );
        assert!(
            !contains(&staged, bytes),
            "{secret} leaked into the staged result"
        );
        assert!(
            !contains(mote.id.as_bytes(), bytes),
            "{secret} leaked into the MoteId"
        );
    }

    // The variable NAMES are not secrets, but they must not smuggle values along with
    // them: assert the staged bytes carry neither name paired with its value.
    for (var, secret) in [("SERVER_TOKEN", SECRET_A), ("SERVER_API_URL", SECRET_B)] {
        assert!(
            !contains(&staged, format!("{var}={secret}").as_bytes()),
            "{var} was staged with its value attached"
        );
    }

    std::env::remove_var(REF_A);
    std::env::remove_var(REF_B);
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
