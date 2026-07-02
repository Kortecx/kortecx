// SPDX-License-Identifier: Apache-2.0
//! G2 host wiring for the App-pointer → run resolution seam ([`kx_gateway_core::AppAuthor`]).
//!
//! [`HostAppAuthor`] resolves a caller-owned `kortecx.app/v1` App into a runnable
//! [`BoundRecipe`] whose tool-firing warrants carry the App's declared secret scope:
//!
//! 1. read the validated stored envelope from the off-journal `apps.db` (server-owned);
//! 2. resolve `references.connections` against the caller's OWN registered connections
//!    by name (via the connections sidecar) — a referenced-but-unregistered connection
//!    is a `MissingIntegration`;
//! 3. lower the blueprint through the SAME canonical `kx-blueprint` path the client
//!    uses (so a server-side App run and a client-authored `SubmitWorkflow` of the same
//!    blueprint produce byte-identical wire bytes — the digest no-op proof), optionally
//!    folding entry `args` into the first model step's prompt (the server-side analogue
//!    of the SDK `_inject_app_args`);
//! 4. author SERVER-SIDE (every warrant resolved from the party's grants, never a
//!    client warrant — SN-8 / BLOCKER #5), reusing the live [`HostWorkflowAuthor`];
//! 5. set the tool-firing warrants' `SecretScope::AllowList` to the App's
//!    `guards.secret_scope` (bounded by the referenced connections' credentials) so the
//!    broker precheck lets a credentialed connector (Gmail/Discord) be dialed inside the
//!    agentic loop. This is a FRESH construction on the resolved warrant (not a narrow):
//!    the operator registered the connection + secret, so granting the App its declared
//!    scope is server-authorized; it is deterministic (a sorted `BTreeSet`) ⇒ recovery
//!    replays the journaled `warrant_ref` byte-identically.
//!
//! Gated behind `mcp-gateway` (it needs the connections store); without it `RunApp`
//! returns `unimplemented` and clients fall back to `GetApp` → `SubmitWorkflow`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_app::{AppEnvelope, ConnectionRef};
use kx_blueprint::{to_request, DagSpec, StepSpec};
use kx_gateway_core::{
    author_steps_from_proto, AppAuthor, AppCatalog, AppRunError, BinderError, BoundRecipe,
    WorkflowAuthor,
};
use kx_mcp_gateway::SqliteConnectionStore;
use kx_warrant::{SecretRef, SecretScope};

/// The host [`AppAuthor`] impl. Holds the App catalog seam (envelope source; the
/// `apps.db`-backed `AppCatalog`), the caller's connection registry (name/credential
/// resolution), and the live workflow author (server-side warrant resolution). All
/// `Arc` — cheaply shared with the gateway.
pub(crate) struct HostAppAuthor {
    apps: Arc<dyn AppCatalog>,
    connections: Arc<SqliteConnectionStore>,
    author: Arc<dyn WorkflowAuthor>,
}

impl HostAppAuthor {
    /// Wire the App-run resolver over the App catalog, the connection registry, and the
    /// live workflow author.
    #[must_use]
    pub(crate) fn new(
        apps: Arc<dyn AppCatalog>,
        connections: Arc<SqliteConnectionStore>,
        author: Arc<dyn WorkflowAuthor>,
    ) -> Self {
        Self {
            apps,
            connections,
            author,
        }
    }
}

/// Resolve an App's `references.connections` + `guards.secret_scope` against the
/// caller's OWN registered connections into the run's secret scope. A pure function
/// (Rule 5.2 — unit-testable without a store): `registered_credentials` /
/// `registered_endpoints` are the credential-ref names / transport endpoints of the
/// caller's registered connections.
///
/// - A referenced connection with no matching registered connection ⇒
///   [`AppRunError::MissingIntegration`] (matched by credential ref when it carries
///   one, else by transport endpoint). The App is owned, so this is an actionable
///   error, not an existence oracle.
/// - A `guards.secret_scope` name that no referenced connection provides ⇒
///   [`AppRunError::InvalidArgs`] (the loud mis-authoring guard — avoids a confusing
///   downstream broker `CapabilityExceedsWarrant`).
/// - Otherwise the scope is exactly the declared names (bounded to the referenced
///   connections' credentials); empty ⇒ `None` (fail-closed — a credentialed tool then
///   refuses at the broker, by design).
fn resolve_secret_scope(
    refs: &[ConnectionRef],
    scope_names: &[String],
    registered_credentials: &BTreeSet<String>,
    registered_endpoints: &BTreeSet<String>,
) -> Result<Option<SecretScope>, AppRunError> {
    for cref in refs {
        let satisfied = if cref.credential_ref.is_empty() {
            registered_endpoints.contains(&cref.descriptor)
        } else {
            registered_credentials.contains(&cref.credential_ref)
        };
        if !satisfied {
            let name = if cref.credential_ref.is_empty() {
                cref.descriptor.clone()
            } else {
                cref.credential_ref.clone()
            };
            return Err(AppRunError::MissingIntegration(name));
        }
    }

    // The credentials the App's referenced connections provide (the ceiling on the scope).
    let declared: BTreeSet<&str> = refs
        .iter()
        .map(|c| c.credential_ref.as_str())
        .filter(|s| !s.is_empty())
        .collect();

    for name in scope_names {
        if !declared.contains(name.as_str()) {
            return Err(AppRunError::InvalidArgs(format!(
                "guards.secret_scope names {name:?} but no referenced connection provides \
                 that credential"
            )));
        }
    }

    let allowed: BTreeSet<SecretRef> = scope_names.iter().cloned().map(SecretRef).collect();
    Ok(if allowed.is_empty() {
        None
    } else {
        Some(SecretScope::AllowList(allowed))
    })
}

/// `true` when a blueprint step is a MODEL step (mirrors `kx_blueprint`'s
/// `resolve_kind` inference: an explicit `kind`, else model fields ⇒ model).
fn is_model_step(s: &StepSpec) -> bool {
    match s.kind.as_deref() {
        Some(k) => k == "model",
        None => !s.model_id.is_empty() || !s.prompt.is_empty(),
    }
}

/// Fold optional entry `args` (a JSON object of string→string) into the ENTRY (first)
/// model step's prompt as a canonical, sorted "Inputs" block — the server-side analogue
/// of the SDK `_inject_app_args`. A NO-OP when `args` is empty OR the blueprint has no
/// model step ⇒ byte-identical to a no-args lowering. Sorted keys ⇒ deterministic
/// (recovery-stable; the args are steer-only DATA — they never grant, SN-8).
fn inject_app_args(dag: &mut DagSpec, args: &[u8]) -> Result<(), AppRunError> {
    if args.is_empty() {
        return Ok(());
    }
    let parsed: BTreeMap<String, String> = serde_json::from_slice(args).map_err(|e| {
        AppRunError::InvalidArgs(format!("app args must be a JSON object of strings: {e}"))
    })?;
    if parsed.is_empty() {
        return Ok(());
    }
    let Some(step) = dag.steps.iter_mut().find(|s| is_model_step(s)) else {
        return Ok(());
    };
    let block: String = parsed
        .iter()
        .map(|(k, v)| format!("- {k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    step.prompt = format!("{}\n\nInputs:\n{block}", step.prompt)
        .trim()
        .to_string();
    Ok(())
}

#[tonic::async_trait]
impl AppAuthor for HostAppAuthor {
    async fn author_app(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
    ) -> Result<BoundRecipe, AppRunError> {
        // (1) Read the validated stored envelope (server-owned; uniform not-found so an
        //     unauthorized caller learns nothing about what exists).
        let (_, envelope_bytes) = self
            .apps
            .get(party, handle)
            .map_err(|e| AppRunError::Internal(format!("apps.db read: {e}")))?
            .ok_or(AppRunError::NotAuthorized)?;
        let env = AppEnvelope::from_json_slice(&envelope_bytes)
            .map_err(|e| AppRunError::Internal(format!("stored envelope invalid: {e}")))?;

        // (2) Resolve references.connections against the caller's OWN registry + compute
        //     the run's secret scope (a pure function over the registered creds/endpoints).
        let registered = self
            .connections
            .list()
            .map_err(|e| AppRunError::Internal(format!("connections.db read: {e}")))?;
        let reg_creds: BTreeSet<String> = registered
            .iter()
            .filter_map(|c| c.credential_ref.clone())
            .collect();
        let reg_endpoints: BTreeSet<String> = registered
            .iter()
            .map(|c| c.transport.endpoint().to_string())
            .collect();
        let secret_scope = resolve_secret_scope(
            &env.references.connections,
            &env.steering_config.guards.secret_scope,
            &reg_creds,
            &reg_endpoints,
        )?;

        // (3) Lower the blueprint through the canonical path (+ optional arg injection).
        let mut dag: DagSpec = serde_json::from_value(env.blueprint).map_err(|e| {
            AppRunError::InvalidArgs(format!("app blueprint is not a DagSpec: {e}"))
        })?;
        inject_app_args(&mut dag, args)?;
        let req = to_request(dag).map_err(|e| AppRunError::InvalidArgs(e.to_string()))?;

        // (4) Parse into the authoring vocabulary (SHARED with SubmitWorkflow) + author
        //     SERVER-SIDE (warrants from the party's grants, never the client — SN-8).
        let (steps, edges, mode) =
            author_steps_from_proto(req.steps, req.edges, req.execution_mode)
                .map_err(|s| AppRunError::InvalidArgs(s.message().to_string()))?;
        let mut bound = self
            .author
            .author(party, req.seed, &steps, &edges, mode, &req.context_bundles)
            .await
            .map_err(|e| match e {
                BinderError::NotAuthorized => AppRunError::NotAuthorized,
                BinderError::InvalidArgs(d) => AppRunError::InvalidArgs(d),
                BinderError::Internal(d) => AppRunError::Internal(d),
            })?;

        // (5) The G2 load-bearing grant: give the tool-firing warrants the App's declared
        //     secret scope so the broker precheck (capability.required_secret_scope ⊆
        //     warrant.secret_scope) lets a credentialed connector be dialed in the loop.
        //     A FRESH construction on the resolved warrant (server-authorized from the
        //     validated envelope), applied to the tool-granting motes (the agentic MODEL
        //     anchor + any tool step) — the react in-loop dispatch fires under the anchor
        //     warrant, so this propagates to every observation turn. Empty scope ⇒
        //     unchanged (`SecretScope::None`) ⇒ a credentialed tool fails closed, by design.
        if let Some(scope) = &secret_scope {
            for (_, warrant) in &mut bound.motes {
                if !warrant.tool_grants.is_empty() {
                    warrant.secret_scope = scope.clone();
                }
            }
        }
        Ok(bound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_step(prompt: &str) -> StepSpec {
        serde_json::from_value(serde_json::json!({ "kind": "model", "prompt": prompt }))
            .expect("a StepSpec")
    }

    fn dag(steps: Vec<StepSpec>) -> DagSpec {
        DagSpec {
            seed: 0,
            steps,
            edges: vec![],
            execution_mode: None,
            context_bundles: vec![],
        }
    }

    #[test]
    fn inject_args_empty_is_a_noop() {
        let mut d = dag(vec![model_step("go")]);
        inject_app_args(&mut d, b"").unwrap();
        assert_eq!(d.steps[0].prompt, "go");
        inject_app_args(&mut d, b"{}").unwrap();
        assert_eq!(d.steps[0].prompt, "go");
    }

    #[test]
    fn inject_args_folds_a_sorted_inputs_block_into_the_first_model_step() {
        let mut d = dag(vec![model_step("Answer the question.")]);
        // Deliberately unsorted input; the block must come out key-sorted (deterministic).
        inject_app_args(&mut d, br#"{"topic":"whales","audience":"kids"}"#).unwrap();
        assert_eq!(
            d.steps[0].prompt,
            "Answer the question.\n\nInputs:\n- audience: kids\n- topic: whales"
        );
    }

    #[test]
    fn inject_args_noop_when_no_model_step() {
        let pure: StepSpec = serde_json::from_value(serde_json::json!({ "kind": "pure" })).unwrap();
        let mut d = dag(vec![pure]);
        inject_app_args(&mut d, br#"{"x":"y"}"#).unwrap();
        // Unchanged (no model step to steer).
        assert!(d.steps[0].prompt.is_empty());
    }

    #[test]
    fn inject_args_rejects_non_object_json() {
        let mut d = dag(vec![model_step("go")]);
        assert!(matches!(
            inject_app_args(&mut d, b"[1,2,3]"),
            Err(AppRunError::InvalidArgs(_))
        ));
    }

    fn cref(descriptor: &str, credential_ref: &str) -> ConnectionRef {
        ConnectionRef {
            descriptor: descriptor.to_string(),
            credential_ref: credential_ref.to_string(),
        }
    }

    fn creds(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn secret_scope_grants_the_declared_credential_when_registered() {
        // E1 POSITIVE: App refs the gmail connection + declares its credential in
        // secret_scope, and the credential is registered ⇒ AllowList([KX_GMAIL_CREDENTIAL]).
        let refs = vec![cref("kx-connector-gmail", "KX_GMAIL_CREDENTIAL")];
        let scope = vec!["KX_GMAIL_CREDENTIAL".to_string()];
        let got = resolve_secret_scope(
            &refs,
            &scope,
            &creds(&["KX_GMAIL_CREDENTIAL"]),
            &BTreeSet::new(),
        )
        .unwrap();
        match got {
            Some(SecretScope::AllowList(s)) => {
                assert_eq!(s.len(), 1);
                assert!(s.contains(&SecretRef("KX_GMAIL_CREDENTIAL".to_string())));
            }
            other => panic!("expected AllowList, got {other:?}"),
        }
    }

    #[test]
    fn secret_scope_empty_is_fail_closed_none() {
        // E2 NEGATIVE: the connection is registered but the App declares no secret_scope
        // ⇒ None (SecretScope::None) ⇒ the credentialed tool fails closed at the broker.
        let refs = vec![cref("kx-connector-gmail", "KX_GMAIL_CREDENTIAL")];
        let got = resolve_secret_scope(
            &refs,
            &[],
            &creds(&["KX_GMAIL_CREDENTIAL"]),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(got.is_none(), "empty secret_scope ⇒ fail-closed None");
    }

    #[test]
    fn missing_registered_connection_is_a_missing_integration() {
        // E3 MISSING: App refs gmail by credential but nothing is registered.
        let refs = vec![cref("kx-connector-gmail", "KX_GMAIL_CREDENTIAL")];
        let err = resolve_secret_scope(&refs, &[], &BTreeSet::new(), &BTreeSet::new())
            .expect_err("missing integration");
        match err {
            AppRunError::MissingIntegration(name) => assert_eq!(name, "KX_GMAIL_CREDENTIAL"),
            other => panic!("expected MissingIntegration, got {other:?}"),
        }
    }

    #[test]
    fn credential_less_ref_matches_by_endpoint() {
        // A credential-less connection is satisfied by a matching transport endpoint.
        let refs = vec![cref("https://mcp.example/sse", "")];
        let endpoints: BTreeSet<String> = creds(&["https://mcp.example/sse"]);
        assert!(
            resolve_secret_scope(&refs, &[], &BTreeSet::new(), &endpoints)
                .unwrap()
                .is_none()
        );
        // ... and MissingIntegration when the endpoint is not registered.
        assert!(matches!(
            resolve_secret_scope(&refs, &[], &BTreeSet::new(), &BTreeSet::new()),
            Err(AppRunError::MissingIntegration(_))
        ));
    }

    #[test]
    fn secret_scope_naming_an_unreferenced_credential_is_rejected() {
        // The loud mis-authoring guard: secret_scope may only name a credential a
        // referenced connection provides.
        let refs = vec![cref("kx-connector-gmail", "KX_GMAIL_CREDENTIAL")];
        let scope = vec!["SOME_OTHER_SECRET".to_string()];
        let err = resolve_secret_scope(
            &refs,
            &scope,
            &creds(&["KX_GMAIL_CREDENTIAL", "SOME_OTHER_SECRET"]),
            &BTreeSet::new(),
        )
        .expect_err("loud guard");
        assert!(matches!(err, AppRunError::InvalidArgs(_)));
    }
}
