//! Secret WRITES are refused unless the gateway is loopback-bound; secret READS are not.
//!
//! ## Why this needed a test at all
//!
//! Planting or removing host credential material is the most privileged thing this
//! surface does, and the rule that limits it is one boolean read at the top of two
//! handlers. It was asserted nowhere. What existed was a table in `control_surface.rs`
//! declaring `PutSecret` and `DeleteSecret` as loopback-only — a statement of intent that
//! the handlers are free to disagree with, since nothing compares them. A behavioural
//! test of a gateway built the normal way cannot see the rule either: the host computes
//! the flag from its own bind address, so every ordinary serve is loopback and the
//! refusal branch is unreachable from a test that goes through `start`.
//!
//! Injecting the seam directly is the only way to build the un-narrowed case, which is
//! why this lives here rather than beside the other secret tests.
//!
//! ## The asymmetry is the point
//!
//! Each arm carries the OTHER outcome as its control:
//!
//! - a non-loopback gateway refuses both writes AND still serves the read, so the refusal
//!   is about writes and about loopback — not a store that is simply broken;
//! - a loopback gateway accepts the identical writes, so the refusal above is not the
//!   only thing this store can do.
//!
//! Without both, "PutSecret returned an error" would be consistent with an unwired seam,
//! a bad party, or a store that fails everything.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{build_run, spawn, spawn_with_party, MockSubmitter};
use kx_gateway_core::{
    ContentReader, GatewayService, JournalReader, ReadOnly, RunSubmitter, SecretAdmin,
    SecretAdminError, SecretNameView,
};
use kx_proto::proto;

const PARTY: &str = "alice";

/// An in-memory `SecretAdmin`. The refusal under test happens in the HANDLER, above this
/// seam, so a store that would happily accept every call is exactly the right stand-in:
/// if a write ever reaches here on a non-loopback gateway, the assertion fails because
/// the call SUCCEEDED, not because a mock objected.
#[derive(Default)]
struct MemSecrets(Mutex<BTreeMap<String, (u64, u64)>>);

impl SecretAdmin for MemSecrets {
    fn put(&self, name: &str, _value: &str) -> Result<(), SecretAdminError> {
        self.0.lock().unwrap().insert(name.to_string(), (1, 1));
        Ok(())
    }

    fn list_names(
        &self,
        _limit: u32,
        _after_name: &str,
    ) -> Result<(Vec<SecretNameView>, bool), SecretAdminError> {
        let rows = self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(name, (c, u))| SecretNameView {
                name: name.clone(),
                created_unix_ms: *c,
                updated_unix_ms: *u,
            })
            .collect();
        Ok((rows, false))
    }

    fn delete(&self, name: &str) -> Result<bool, SecretAdminError> {
        Ok(self.0.lock().unwrap().remove(name).is_some())
    }
}

fn service(writes_loopback_ok: bool) -> GatewayService {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let submitter: Arc<dyn RunSubmitter> = Arc::new(MockSubmitter::default());
    GatewayService::new(reader, submitter, content)
        .with_secret_admin(Arc::new(MemSecrets::default()), writes_loopback_ok)
}

fn put() -> proto::PutSecretRequest {
    proto::PutSecretRequest {
        name: "GITHUB_TOKEN".to_string(),
        value: "ghp_value".to_string(),
    }
}

fn list() -> proto::ListSecretNamesRequest {
    proto::ListSecretNamesRequest {
        limit: 0,
        after_name: String::new(),
    }
}

fn del() -> proto::DeleteSecretRequest {
    proto::DeleteSecretRequest {
        name: "GITHUB_TOKEN".to_string(),
    }
}

/// ★ A network-exposed gateway refuses to plant or remove credential material, and still
/// answers the read.
#[tokio::test]
async fn a_non_loopback_gateway_refuses_secret_writes_but_still_serves_the_read() {
    let mut c = spawn_with_party(service(false), PARTY).await;

    let err = c.put_secret(put()).await.unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "planting a credential over a network-exposed bind must be refused, and refused \
         as a permission decision rather than as a storage failure; got {err:?}"
    );
    assert!(
        err.message().contains("loopback"),
        "the refusal must say WHY, or an operator cannot tell it from an unwired store; \
         got {:?}",
        err.message()
    );

    let err = c.delete_secret(del()).await.unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "removing a credential is as privileged as planting one — a remote caller who \
         can delete can disable every credentialed run; got {err:?}"
    );

    // THE CONTROL, one variable: the READ on the SAME gateway succeeds. Without it, both
    // refusals above would be equally consistent with a seam that was never wired.
    let names = c
        .list_secret_names(list())
        .await
        .expect("the read is allowed");
    assert!(
        names.into_inner().names.is_empty(),
        "the governance read is served — and reports nothing, because both writes above \
         were refused before they reached the store"
    );
}

/// ★ The accepting control for the file: the identical writes on a loopback-bound
/// gateway succeed and take effect.
#[tokio::test]
async fn a_loopback_gateway_accepts_the_identical_writes() {
    let mut c = spawn_with_party(service(true), PARTY).await;

    assert!(
        c.put_secret(put())
            .await
            .expect("stored")
            .into_inner()
            .stored,
        "the same call refused above succeeds when the gateway is loopback-bound"
    );
    assert_eq!(
        c.list_secret_names(list())
            .await
            .expect("list")
            .into_inner()
            .names
            .len(),
        1,
        "and it actually reached the store, which is what makes the refusal above a \
         refusal rather than a no-op"
    );
    assert!(
        c.delete_secret(del())
            .await
            .expect("deleted")
            .into_inner()
            .removed,
        "and the delete that was refused above also succeeds here"
    );
}

/// ★ Authority is checked before the loopback rule: an unauthenticated caller is told it
/// has no identity, not that it is on the wrong interface.
///
/// The ordering matters for diagnosis. `permission_denied … loopback` sent to a caller
/// who simply has no token would send an operator to reconfigure their bind address to
/// fix an auth problem.
#[tokio::test]
async fn an_unauthenticated_caller_is_refused_for_identity_not_for_the_bind() {
    // `spawn` installs no CallerParty, unlike `spawn_with_party`.
    let mut c = spawn(service(false)).await;

    let err = c.put_secret(put()).await.unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::Unauthenticated,
        "no resolved identity is an authentication failure, and it is decided FIRST — \
         reporting the loopback rule here would misdirect the fix; got {err:?}"
    );
}
