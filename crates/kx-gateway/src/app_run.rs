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
//! ## RC-SW1: the skill bind (step 3b — BEFORE lowering; the wish is never authority)
//!
//! An App's `references.skills` (`SkillRef { name, instructions_ref, tools }`) resolve
//! at run, PRE-author — both legs are author-time-or-never (verified ground truth: the
//! coordinator parks an agentic launch only when the step's `tool_contract` is
//! non-empty, and entry context injects pre-compile; a post-author mutation is either
//! inert or breaks `MoteId` identity):
//!
//! - **Instructions** — each `instructions_ref` (present-in-CAS checked FAIL-CLOSED)
//!   becomes a labeled `skill:<name>` [`ContextItemRef`] merged into the entry step's
//!   identity-bearing `CONTEXT_ITEMS` bundle via
//!   [`HostWorkflowAuthor::author_with_context_items`] (ONE canonical inject with any
//!   attached context bundles).
//! - **Tool wishes** — the deduped wish union is intersected
//!   (`wish ∩ caller-authority ∩ fireable ∩ registry ∩ compat`,
//!   [`crate::provision::skill_union_grants`] — FAIL-SOFT: an unfulfillable wish drops
//!   with a warning, never bricks the App) and the survivors FOLD into the ENTRY
//!   AGENTIC step — the first model step that is a DAG ROOT, the SAME step the
//!   instructions bind to (so tools + instructions co-locate; a chained
//!   `pure → model` blueprint whose model step is not a root gets NO fold rather
//!   than tools-without-instructions). Declared entries win; empty ⇒ no fold ⇒ the
//!   step stays a plain transform — a skill on its own grants NOTHING. The folded
//!   contract then rides the existing fail-closed `agentic_step_warrant` path, and
//!   step 5's secret scope covers the now-tool-firing mote automatically.
//!
//! No skills ⇒ the whole block is skipped (a structural no-op — the digest-invariance
//! proof) and `author_with_context_items` receives an empty slice, byte-identical to
//! the plain `author()` path.
//!
//! Gated behind `mcp-gateway` (it needs the connections store); without it `RunApp`
//! returns `unimplemented` and clients fall back to `GetApp` → `SubmitWorkflow`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_app::{AppEnvelope, ConnectionRef, SkillRef};
use kx_blueprint::{to_request, DagSpec, StepSpec};
use kx_content::{ContentRef, ContentStore};
use kx_gateway_core::{
    author_steps_from_proto, AppAuthor, AppCatalog, AppRunError, BinderError, BoundRecipe,
    RegisteredToolsView,
};
use kx_mcp_gateway::SqliteConnectionStore;
use kx_mote::ContextItemRef;
use kx_tool_registry::ToolRegistry;
use kx_warrant::{SecretRef, SecretScope};

use crate::provision::{party_tool_authority, skill_union_grants, DemoLibrary, HostWorkflowAuthor};

/// The narrow author-time content-PRESENCE check (the `instructions_ref`
/// fail-closed gate). Blanket over any [`ContentStore`] so the host hands its
/// `LocalFsContentStore` and tests ride the in-memory store — without forcing
/// the non-object-safe `ContentStore` (associated `Payload`) across an `Arc<dyn>`.
pub(crate) trait ContentPresence: Send + Sync {
    /// `true` iff the store currently holds a blob at `r`.
    fn contains_ref(&self, r: &ContentRef) -> bool;
}

impl<T: ContentStore + Send + Sync> ContentPresence for T {
    fn contains_ref(&self, r: &ContentRef) -> bool {
        self.contains(r)
    }
}

/// The host [`AppAuthor`] impl. Holds the App catalog seam (envelope source; the
/// `apps.db`-backed `AppCatalog`), the caller's connection registry (name/credential
/// resolution), and the live workflow author (server-side warrant resolution). All
/// `Arc` — cheaply shared with the gateway.
pub(crate) struct HostAppAuthor {
    apps: Arc<dyn AppCatalog>,
    connections: Arc<SqliteConnectionStore>,
    /// The CONCRETE live author (not the trait object): RC-SW1 needs the inherent
    /// `author_with_context_items` (skill instructions merge into the entry-step
    /// bundle pre-compile — a trait-signature change would churn 17 call sites for
    /// one App-only concern). Same `Arc` the service's `WorkflowAuthor` wraps.
    author: Arc<HostWorkflowAuthor>,
    /// RC-SW1: the shared library (grants + blueprint_base) — the skill-wish
    /// caller-authority resolution ([`party_tool_authority`]).
    lib: Arc<DemoLibrary>,
    /// RC-SW1: the LIVE tool registry (the SAME `Arc` the coordinator + broker
    /// share) — wish-tool defs for the compat filter.
    tools: Arc<dyn ToolRegistry>,
    /// RC-SW1: the broker-fireable view (the SAME truth the admission backstops
    /// intersect against) — a wish tool that is not fireable is dropped, never
    /// authored into a warrant that would then fail the RunApp backstop.
    registered: Arc<dyn RegisteredToolsView>,
    /// RC-SW1: the author-time `instructions_ref` presence gate (fail-closed —
    /// instructions are a skill's semantic core; a dispatch-time miss would
    /// dead-letter opaquely instead).
    content: Arc<dyn ContentPresence>,
}

impl HostAppAuthor {
    /// Wire the App-run resolver over the App catalog, the connection registry, the
    /// live workflow author, and the RC-SW1 skill-bind seams (library authority +
    /// live registry + fireable view + content presence).
    #[must_use]
    pub(crate) fn new(
        apps: Arc<dyn AppCatalog>,
        connections: Arc<SqliteConnectionStore>,
        author: Arc<HostWorkflowAuthor>,
        lib: Arc<DemoLibrary>,
        tools: Arc<dyn ToolRegistry>,
        registered: Arc<dyn RegisteredToolsView>,
        content: Arc<dyn ContentPresence>,
    ) -> Self {
        Self {
            apps,
            connections,
            author,
            lib,
            tools,
            registered,
            content,
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

/// RC-SW1: the deduped multi-skill tool WISH union, in envelope (canonical-bytes)
/// order. A version conflict across skills for one tool id is FAIL-SOFT
/// first-occurrence-wins + a warning (a wish is a wish — one stale skill must
/// never brick the App). A pure function (Rule 5.2 — unit-testable).
fn skill_wish_union(skills: &[SkillRef]) -> BTreeMap<String, String> {
    let mut wish: BTreeMap<String, String> = BTreeMap::new();
    for s in skills {
        for (id, version) in &s.tools {
            match wish.get(id) {
                None => {
                    wish.insert(id.clone(), version.clone());
                }
                Some(kept) if kept != version => {
                    tracing::warn!(
                        tool = %id, kept = %kept, dropped = %version, skill = %s.name,
                        "skill wish version conflict: first occurrence wins"
                    );
                }
                Some(_) => {}
            }
        }
    }
    wish
}

/// RC-SW1: the ENTRY agentic step — the first MODEL step that is a DAG ROOT (no
/// incoming edge). This is EXACTLY where `author_with_context_items` →
/// `inject_entry_config` places the skill instructions (it targets DAG roots),
/// so the tool fold and the instruction inject MUST co-locate on it — otherwise
/// a chained `pure → model` blueprint would grant the model step tools while the
/// instructions land on the pure root the model never reads (a silent split).
/// `None` ⇒ no root model step: the fold is skipped (granting a non-root model
/// step tools its instructions can't reach is the split we refuse to create;
/// instructions still bind to the root per the PR-7 context semantics).
fn entry_agentic_step_index(dag: &DagSpec) -> Option<usize> {
    let has_incoming: BTreeSet<u32> = dag.edges.iter().map(|e| e.child).collect();
    dag.steps
        .iter()
        .enumerate()
        .find(|(i, s)| {
            is_model_step(s) && !has_incoming.contains(&u32::try_from(*i).unwrap_or(u32::MAX))
        })
        .map(|(i, _)| i)
}

/// RC-SW1: fold the GRANTED (already-intersected) skill tools into the ENTRY
/// AGENTIC step's `tool_contract` — the SAME root model step the instructions
/// bind to ([`entry_agentic_step_index`]). The fold decides LOOP EXISTENCE at
/// author time (a non-empty contract compiles the step as a generator; the
/// coordinator parks it as an agentic launch); an author-declared `(id →
/// version)` pin always wins (`or_insert`). Empty `granted` ⇒ NO fold — the step
/// stays a plain transform, which IS the conformance "binds-empty-grants-to-
/// zero" behavior. A pure function.
fn fold_skill_tools(dag: &mut DagSpec, granted: &BTreeMap<String, String>) {
    if granted.is_empty() {
        return;
    }
    let Some(idx) = entry_agentic_step_index(dag) else {
        tracing::warn!(
            "skill tool wishes dropped: the blueprint has no ROOT model step to fold them onto \
             (a non-root model step's instructions would be unreachable — refusing the split; \
             instructions still bind to the entry root)"
        );
        return;
    };
    let step = &mut dag.steps[idx];
    for (id, version) in granted {
        step.tool_contract
            .entry(id.clone())
            .or_insert_with(|| version.clone());
    }
}

/// RC-SW1: resolve each [`SkillRef`]'s instructions into a labeled `skill:<name>`
/// [`ContextItemRef`] — FAIL-CLOSED on a blob missing from the content store at
/// author time (instructions are the skill's semantic core; deferring the miss to
/// dispatch would dead-letter the run opaquely — "never run the model on PARTIAL
/// context"). The label keeps the item legible in the assembled prompt (vs the
/// anonymous `ref:<hex12>` the raw-refs slot would mint).
fn skill_context_items(
    skills: &[SkillRef],
    content: &dyn ContentPresence,
) -> Result<Vec<ContextItemRef>, AppRunError> {
    let mut items = Vec::with_capacity(skills.len());
    for s in skills {
        let bytes = crate::provision::decode_hex32(&s.instructions_ref).ok_or_else(|| {
            // Defense-in-depth: already impossible past AppEnvelope::validate.
            AppRunError::InvalidArgs(format!(
                "skill {:?} instructions_ref is not a 64-hex content ref",
                s.name
            ))
        })?;
        if !content.contains_ref(&ContentRef::from_bytes(bytes)) {
            return Err(AppRunError::InvalidArgs(format!(
                "skill {:?} instructions ({}…) not found in the content store; add the skill \
                 body first (kx skills add / PutContent), then re-run",
                s.name,
                &s.instructions_ref[..12]
            )));
        }
        items.push(ContextItemRef {
            name: format!("skill:{}", s.name),
            content_ref: bytes,
        });
    }
    Ok(items)
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

        // (3b) RC-SW1: resolve the App's attached skills BEFORE lowering (module
        //      doc). Structurally gated: NO skills ⇒ zero new code runs (the
        //      digest no-op). Instructions → labeled context items (fail-closed
        //      CAS presence); wishes → the fail-soft intersection, folded into
        //      the entry model step's tool_contract (declared pins win).
        let mut skill_items: Vec<ContextItemRef> = Vec::new();
        if !env.references.skills.is_empty() {
            skill_items = skill_context_items(&env.references.skills, self.content.as_ref())?;
            let wish = skill_wish_union(&env.references.skills);
            if !wish.is_empty() {
                // Use-gate + conditional narrowing (SN-8; see party_tool_authority).
                let allowlist = party_tool_authority(&self.lib, party).map_err(|e| match e {
                    BinderError::NotAuthorized => AppRunError::NotAuthorized,
                    BinderError::InvalidArgs(d) => AppRunError::InvalidArgs(d),
                    BinderError::Internal(d) => AppRunError::Internal(d),
                })?;
                let fireable = self.registered.registered_grants();
                // The declared contract seed is read from the SAME entry agentic
                // step the fold targets (the root model step), so an author pin on
                // that step wins + the fs/net compat union is seeded correctly.
                let declared = entry_agentic_step_index(&dag)
                    .map(|i| dag.steps[i].tool_contract.clone())
                    .unwrap_or_default();
                let granted = skill_union_grants(
                    &declared,
                    &wish,
                    allowlist.as_ref(),
                    self.tools.as_ref(),
                    &fireable,
                );
                fold_skill_tools(&mut dag, &granted);
            }
        }

        let req = to_request(dag).map_err(|e| AppRunError::InvalidArgs(e.to_string()))?;

        // (4) Parse into the authoring vocabulary (SHARED with SubmitWorkflow) + author
        //     SERVER-SIDE (warrants from the party's grants, never the client — SN-8).
        //     The skill instructions ride as extra context items into the SAME entry
        //     bundle inject (empty slice ⇒ byte-identical to the plain author path).
        let (steps, edges, mode) =
            author_steps_from_proto(req.steps, req.edges, req.execution_mode)
                .map_err(|s| AppRunError::InvalidArgs(s.message().to_string()))?;
        let mut bound = self
            .author
            .author_with_context_items(
                party,
                req.seed,
                &steps,
                &edges,
                mode,
                &req.context_bundles,
                &skill_items,
            )
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

    fn pure_step() -> StepSpec {
        serde_json::from_value(serde_json::json!({ "kind": "pure", "params": { "x": "y" } }))
            .expect("a StepSpec")
    }

    fn edge(parent: u32, child: u32) -> kx_blueprint::EdgeSpec {
        serde_json::from_value(serde_json::json!({ "parent": parent, "child": child }))
            .expect("an EdgeSpec")
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

    // ----- RC-SW1: skill-bind pure helpers -----

    fn skill(name: &str, instructions_ref: &str, tools: &[(&str, &str)]) -> SkillRef {
        SkillRef {
            name: name.into(),
            instructions_ref: instructions_ref.into(),
            tools: tools
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    #[test]
    fn skill_wish_union_first_wins_on_version_conflict_across_skills() {
        let skills = vec![
            skill(
                "a",
                &"a".repeat(64),
                &[("gmail/search", "1"), ("retrieve", "1")],
            ),
            skill(
                "b",
                &"b".repeat(64),
                &[("gmail/search", "2"), ("fs-read", "1")],
            ),
        ];
        let wish = skill_wish_union(&skills);
        assert_eq!(wish["gmail/search"], "1", "first occurrence wins");
        assert_eq!(wish.len(), 3);
    }

    #[test]
    fn fold_skill_tools_targets_the_entry_root_model_step_and_declared_version_wins() {
        let mut declared_step = model_step("go");
        declared_step
            .tool_contract
            .insert("gmail/search".into(), "9".into());
        let mut d = dag(vec![declared_step, model_step("second")]);
        let granted: BTreeMap<String, String> = [
            ("gmail/search".to_string(), "1".to_string()),
            ("retrieve".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect();
        fold_skill_tools(&mut d, &granted);
        // The author-declared pin survives; the wish addition folds in.
        assert_eq!(d.steps[0].tool_contract["gmail/search"], "9");
        assert_eq!(d.steps[0].tool_contract["retrieve"], "1");
        // Only the entry (root) model step receives the fold.
        assert!(d.steps[1].tool_contract.is_empty());
    }

    #[test]
    fn fold_skill_tools_refuses_the_split_when_the_model_step_is_not_a_root() {
        // A chained pure → model blueprint: instructions inject_entry_config only
        // on the pure ROOT, so folding tools into the (non-root) model step would
        // grant tools the instructions can never reach. The fold must SKIP —
        // never create tools-without-instructions on the model step.
        let mut d = dag(vec![pure_step(), model_step("go")]);
        d.edges = vec![edge(0, 1)];
        let granted: BTreeMap<String, String> = [("retrieve".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        fold_skill_tools(&mut d, &granted);
        assert!(
            d.steps[1].tool_contract.is_empty(),
            "no root model step ⇒ no fold ⇒ no split"
        );
        assert_eq!(entry_agentic_step_index(&d), None);
        // A single model step IS a root ⇒ it is the entry agentic step.
        let single = dag(vec![model_step("go")]);
        assert_eq!(entry_agentic_step_index(&single), Some(0));
    }

    #[test]
    fn fold_skill_tools_skips_when_granted_is_empty() {
        // The conformance "binds-empty-grants-to-zero" behavior: no fold ⇒ the
        // step keeps an EMPTY contract ⇒ it compiles as a plain transform (no
        // loop, no grants) — a skill on its own grants nothing.
        let mut d = dag(vec![model_step("go")]);
        fold_skill_tools(&mut d, &BTreeMap::new());
        assert!(d.steps[0].tool_contract.is_empty());
    }

    #[test]
    fn skill_context_items_labels_and_fails_closed_on_a_missing_blob() {
        use kx_content::{ContentStore as _, InMemoryContentStore};
        let store = InMemoryContentStore::new();
        let r = store.put(b"# Skill instructions").unwrap();
        let hex = hex_str(&r.0);

        // Present blob ⇒ a labeled item carrying the exact ref bytes.
        let items = skill_context_items(&[skill("triage", &hex, &[])], &store).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "skill:triage");
        assert_eq!(items[0].content_ref, r.0);

        // Absent blob ⇒ FAIL-CLOSED with the actionable text (never a silent
        // partial-context run; a dispatch-time miss would dead-letter opaquely).
        let missing = "c".repeat(64);
        let err = skill_context_items(&[skill("triage", &missing, &[])], &store).unwrap_err();
        match err {
            AppRunError::InvalidArgs(msg) => {
                assert!(msg.contains("not found in the content store"), "{msg}");
                assert!(msg.contains("cccccccccccc"), "names the ref prefix: {msg}");
            }
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    // ----- RC-SW1: author_app end-to-end (the digest no-op pin + the skill bind) -----

    use kx_content::InMemoryContentStore;
    use kx_gateway_core::WorkflowAuthor as _;
    use kx_mote::{ConfigKey, ModelId, CONTEXT_ITEMS_KEY};

    fn hex_str(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    struct FixedFireable(BTreeSet<(String, String)>);
    impl RegisteredToolsView for FixedFireable {
        fn registered_grants(&self) -> BTreeSet<(String, String)> {
            self.0.clone()
        }
    }

    fn echo_registry() -> Arc<dyn ToolRegistry> {
        use kx_tool_registry::{
            IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance,
        };
        use kx_warrant::{FsScope, NetScope, ResourceCeiling};
        let mut reg = InMemoryToolRegistry::new();
        let def = ToolDef {
            tool_id: kx_mote::ToolName("echo-tool".into()),
            tool_version: kx_mote::ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: kx_warrant::ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: String::new(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: None,
        };
        let _ = reg.register(def, ToolProvenance::HumanAuthored { author: "t".into() });
        Arc::new(reg)
    }

    /// A full [`HostAppAuthor`] over a tempdir: served model "m", the echo-tool
    /// registry, an explicit fireable set, and an in-memory content store.
    fn rig(
        dir: &std::path::Path,
        fireable: &[(&str, &str)],
    ) -> (
        HostAppAuthor,
        Arc<InMemoryContentStore>,
        Arc<HostWorkflowAuthor>,
    ) {
        let lib = Arc::new(
            DemoLibrary::open_full(
                dir,
                kx_warrant::ExecutorClass::Bwrap,
                &["alice@acme".to_string()],
                Some(ModelId("m".into())),
            )
            .unwrap(),
        );
        let author = Arc::new(HostWorkflowAuthor::from_shared_with_tools(
            lib.clone(),
            echo_registry(),
        ));
        let apps: Arc<dyn AppCatalog> = Arc::new(crate::apps::AppsDb::open(dir).unwrap());
        let connections =
            Arc::new(SqliteConnectionStore::open(dir.join("connections.db")).unwrap());
        let content = Arc::new(InMemoryContentStore::new());
        let fire: BTreeSet<(String, String)> = fireable
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        let host = HostAppAuthor::new(
            apps.clone(),
            connections,
            author.clone(),
            lib,
            echo_registry(),
            Arc::new(FixedFireable(fire)),
            content.clone(),
        );
        (host, content, author)
    }

    fn save_app(host: &HostAppAuthor, env: &AppEnvelope) -> String {
        let handle = "team/apps/t".to_string();
        host.apps
            .save("alice@acme", &handle, &env.to_canonical_json().unwrap())
            .unwrap();
        handle
    }

    /// THE digest no-op pin: a skill-FREE App authors mote-for-mote, warrant-for-
    /// warrant byte-identically to the plain `WorkflowAuthor::author` path over
    /// the same lowered blueprint (zero new code runs on the default path).
    #[tokio::test]
    async fn author_app_without_skills_is_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, author) = rig(dir.path(), &[("echo-tool", "1")]);
        let env = AppEnvelope::new(
            "plain",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        let handle = save_app(&host, &env);
        let via_app = host.author_app("alice@acme", &handle, b"").await.unwrap();

        // The reference: the same blueprint lowered + authored directly.
        let d = dag(vec![model_step("go")]);
        let req = to_request(d).unwrap();
        let (steps, edges, mode) =
            author_steps_from_proto(req.steps, req.edges, req.execution_mode).unwrap();
        let direct = author
            .author("alice@acme", req.seed, &steps, &edges, mode, &[])
            .await
            .unwrap();

        assert_eq!(via_app.motes.len(), direct.motes.len());
        for ((m1, w1), (m2, w2)) in via_app.motes.iter().zip(direct.motes.iter()) {
            assert_eq!(m1.id, m2.id, "byte-identical MoteIds (the no-op proof)");
            assert_eq!(w1, w2, "byte-identical warrants (the no-op proof)");
        }
        assert_eq!(via_app.recipe_fingerprint, direct.recipe_fingerprint);
    }

    /// The skill bind end-to-end: instructions land as a labeled entry context
    /// item AND the granted wish folds into the step contract + warrant (the
    /// loop-existence leg), on the SAME mote.
    #[tokio::test]
    async fn author_app_with_a_skill_folds_grants_and_injects_instructions() {
        let dir = tempfile::tempdir().unwrap();
        let (host, content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        let blob = kx_content::ContentStore::put(content.as_ref(), b"# Triage rules").unwrap();
        let hex = hex_str(&blob.0);

        let mut env = AppEnvelope::new(
            "skilled",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.references.skills.push(SkillRef {
            name: "triage".into(),
            instructions_ref: hex,
            tools: [("echo-tool".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        });
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host.author_app("alice@acme", &handle, b"").await.unwrap();

        let (mote, warrant) = &bound.motes[0];
        // The wish became a REAL grant (wish ∩ fireable ∩ registry) on the step
        // contract (generator ⇒ the coordinator parks the agentic launch) AND the
        // warrant (menu + fireability at runtime).
        assert!(mote
            .def
            .tool_contract
            .contains_key(&kx_mote::ToolName("echo-tool".into())));
        assert!(warrant
            .tool_grants
            .iter()
            .any(|g| g.tool_id.0 == "echo-tool" && g.tool_version.0 == "1"));
        // The instructions ride the entry mote's identity-bearing context.
        let encoded = mote
            .def
            .config_subset
            .get(&ConfigKey(CONTEXT_ITEMS_KEY.into()))
            .expect("entry mote carries CONTEXT_ITEMS");
        let items = kx_mote::decode_context_items(&encoded.0);
        assert!(
            items
                .iter()
                .any(|i| i.name == "skill:triage" && i.content_ref == blob.0),
            "labeled skill instructions present: {items:?}"
        );
    }

    /// FAIL-SOFT: a wish the serve cannot fire drops (with a warning) — the run
    /// still authors, TOOL-LESS (a plain transform, zero grants), and the
    /// instructions still bind. This is the live face of the conformance
    /// "binds-empty-grants-to-zero" check — a skill grants nothing on its own.
    #[tokio::test]
    async fn author_app_with_an_unfireable_skill_wish_proceeds_toolless() {
        let dir = tempfile::tempdir().unwrap();
        let (host, content, _) = rig(dir.path(), &[]); // NOTHING fireable
        let blob = kx_content::ContentStore::put(content.as_ref(), b"# Rules").unwrap();
        let hex = hex_str(&blob.0);

        let mut env = AppEnvelope::new(
            "wishful",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.references.skills.push(SkillRef {
            name: "wishful".into(),
            instructions_ref: hex,
            tools: [("echo-tool".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        });
        let handle = save_app(&host, &env);
        let bound = host.author_app("alice@acme", &handle, b"").await.unwrap();

        let (mote, warrant) = &bound.motes[0];
        assert!(
            mote.def.tool_contract.is_empty(),
            "no fold ⇒ the step stays a plain transform"
        );
        assert!(warrant.tool_grants.is_empty(), "zero grants minted");
        let encoded = mote
            .def
            .config_subset
            .get(&ConfigKey(CONTEXT_ITEMS_KEY.into()))
            .expect("instructions still bind");
        assert!(!kx_mote::decode_context_items(&encoded.0).is_empty());
    }
}
