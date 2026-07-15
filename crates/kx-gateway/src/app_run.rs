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
//! ## The skill bind (step 3b — BEFORE lowering; the wish is never authority)
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

use kx_app::{AppEnvelope, ConnectionRef, Reach, SkillRef};
use kx_blueprint::{to_request, DagSpec, StepSpec};
use kx_content::{ContentRef, ContentStore};
use kx_gateway_core::{
    app_dataset_scoped_name, author_steps_from_proto, AppAuthor, AppCapability, AppCatalog,
    AppManifest, AppManifestView, AppRunError, BinderError, BoundRecipe, DatasetError, DatasetView,
    GatewayError, IngestDoc, RegisteredToolsView,
};
use kx_mcp_gateway::SqliteConnectionStore;
use kx_mote::{ContextItemRef, ToolName, ToolVersion, REACT_REQUIRE_APPROVAL_KEY};
use kx_tool_registry::ToolRegistry;
use kx_warrant::{SecretRef, SecretScope};

use crate::provision::{party_tool_authority, skill_union_grants, DemoLibrary, HostWorkflowAuthor};

/// The self-ingest ceilings (`T-RUNAPP-RAG-SELF-CONTAINED`) now live in `kx-app`, beside
/// `DatasetRef`: they are properties of the ENVELOPE CONTRACT, not of this enforcement site.
/// The CLI needs the SAME numbers to warn at EXPORT that a corpus will not self-ingest — the
/// `tracing::warn!` below fires on a server long after the author could have acted — and a
/// constant duplicated across two crates is a constant that drifts, leaving both sides
/// confidently disagreeing about one question.
///
/// Enforcement here is unchanged: over-ceiling ⇒ fail-soft skip, never a refusal — the App
/// simply grounds reference-existing. The ceilings exist because the bundle's own bounds are
/// CLIENT-side and an App can sidestep them: a 1 MiB envelope (the server's only App cap) still
/// names thousands of 64-hex refs.
use kx_app::{MAX_APP_CORPUS_BYTES, MAX_APP_CORPUS_REFS};

/// The narrow author-time content-PRESENCE check (the `instructions_ref`
/// fail-closed gate). Blanket over any [`ContentStore`] so the host hands its
/// `LocalFsContentStore` and tests ride the in-memory store — without forcing
/// the non-object-safe `ContentStore` (associated `Payload`) across an `Arc<dyn>`.
pub(crate) trait ContentPresence: Send + Sync {
    /// `true` iff the store currently holds a blob at `r`.
    fn contains_ref(&self, r: &ContentRef) -> bool;

    /// The blob at `r`, or `None` when the store does not hold it.
    ///
    /// `T-RUNAPP-RAG-SELF-CONTAINED`: an App's declared corpus travels as
    /// `references.datasets[].cas_refs`, so materializing it needs the BYTES, not just
    /// presence. Same author-time posture as [`contains_ref`](Self::contains_ref) — the
    /// refs come from the caller's OWN stored envelope, resolved against the local store.
    fn get_ref(&self, r: &ContentRef) -> Option<Vec<u8>>;
}

impl<T: ContentStore + Send + Sync> ContentPresence for T {
    fn contains_ref(&self, r: &ContentRef) -> bool {
        self.contains(r)
    }

    fn get_ref(&self, r: &ContentRef) -> Option<Vec<u8>> {
        self.get(r).ok().map(|p| p.to_vec())
    }
}

/// The host [`AppAuthor`] impl. Holds the App catalog seam (envelope source; the
/// `apps.db`-backed `AppCatalog`), the caller's connection registry (name/credential
/// resolution), and the live workflow author (server-side warrant resolution). All
/// `Arc` — cheaply shared with the gateway.
pub(crate) struct HostAppAuthor {
    apps: Arc<dyn AppCatalog>,
    connections: Arc<SqliteConnectionStore>,
    /// The CONCRETE live author (not the trait object): needs the inherent
    /// `author_with_context_items` (skill instructions merge into the entry-step
    /// bundle pre-compile — a trait-signature change would churn 17 call sites for
    /// one App-only concern). Same `Arc` the service's `WorkflowAuthor` wraps.
    author: Arc<HostWorkflowAuthor>,
    /// The shared library (grants + blueprint_base) — the skill-wish
    /// caller-authority resolution ([`party_tool_authority`]).
    lib: Arc<DemoLibrary>,
    /// The LIVE tool registry (the SAME `Arc` the coordinator + broker
    /// share) — wish-tool defs for the compat filter.
    tools: Arc<dyn ToolRegistry>,
    /// The broker-fireable view (the SAME truth the admission backstops
    /// intersect against) — a wish tool that is not fireable is dropped, never
    /// authored into a warrant that would then fail the RunApp backstop.
    registered: Arc<dyn RegisteredToolsView>,
    /// The author-time `instructions_ref` presence gate (fail-closed —
    /// instructions are a skill's semantic core; a dispatch-time miss would
    /// dead-letter opaquely instead).
    content: Arc<dyn ContentPresence>,
    /// T-RUNAPP-CONTEXT-RAIL: the live dataset store (the SAME `Arc` the retrieve@1
    /// capability + the `DatasetView` service seam share) — used to fail-closed
    /// PRESENCE-check an App's declared `references.datasets` before folding the
    /// `retrieve@1` grant. `None` on a build without the retrieval seam (`hnsw` off)
    /// ⇒ a declared dataset honest-degrades to an ungrounded run (retrieve@1 is not
    /// registered there anyway).
    datasets: Option<Arc<dyn DatasetView>>,
}

impl HostAppAuthor {
    /// Wire the App-run resolver over the App catalog, the connection registry, the
    /// live workflow author, and the skill-bind seams (library authority +
    /// live registry + fireable view + content presence).
    #[must_use]
    #[allow(clippy::too_many_arguments)] // distinct Arc deps for one host resolver; a
                                         // config struct would only move the arity to the caller (Rule 1: no churn for churn).
    pub(crate) fn new(
        apps: Arc<dyn AppCatalog>,
        connections: Arc<SqliteConnectionStore>,
        author: Arc<HostWorkflowAuthor>,
        lib: Arc<DemoLibrary>,
        tools: Arc<dyn ToolRegistry>,
        registered: Arc<dyn RegisteredToolsView>,
        content: Arc<dyn ContentPresence>,
        datasets: Option<Arc<dyn DatasetView>>,
    ) -> Self {
        Self {
            apps,
            connections,
            author,
            lib,
            tools,
            registered,
            content,
            datasets,
        }
    }

    /// T-RUNAPP-CONTEXT-RAIL: declarative RAG-on-App (the "reference-existing" model).
    /// When the App declares datasets to ground over (`references.datasets[].dataset_ref`
    /// ∪ `steering_config.context.dataset_refs`), grant the entry agentic step the
    /// read-only `retrieve@1` tool + steer it to search the named dataset(s) live in the
    /// loop — exactly how `kx/recipes/react-rag` grounds. Server-authorized by the
    /// operator's INGESTED dataset (not a caller-Use escalation: `retrieve@1` is a
    /// `ReadOnlyNondet`, `NetScope::None`, `FsScope::empty` builtin that only reads
    /// operator-provided corpora — so it is granted directly, like the recipe, rather than
    /// through the caller-Use wish intersection).
    ///
    /// - A declared dataset carrying `cas_refs` is SELF-CONTAINED: the corpus travels in
    ///   the App and materializes here (`T-RUNAPP-RAG-SELF-CONTAINED`, see
    ///   [`ensure_app_dataset`](Self::ensure_app_dataset)) ⇒ an imported App grounds with
    ///   NO source datasets present.
    /// - Otherwise the dataset must EXIST in the live store ⇒ else fail-closed `InvalidArgs`
    ///   (a mis-authoring guard; the operator pre-ingests via `kx datasets ingest`). This is
    ///   also where a self-contained App lands when its blobs did not travel (an export
    ///   without `--with-data`) — the 2a reference-existing path, unchanged.
    /// - No retrieval seam on this build (`hnsw` off ⇒ `self.datasets == None` ⇒ `retrieve@1`
    ///   is not even registered) ⇒ honest-degrade to an UNGROUNDED run (mirrors chat-rag's
    ///   no-dataset-view path), never a hard error.
    /// - No root model step to ground ⇒ the fold + steer skip (mirror `fold_skill_tools`).
    async fn fold_dataset_rag(
        &self,
        bindings: &[DatasetBinding],
        dag: &mut DagSpec,
    ) -> Result<(), AppRunError> {
        let Some(view) = self.datasets.as_ref() else {
            tracing::warn!(
                count = bindings.len(),
                "app declares datasets to ground over but this build has no retrieval seam \
                 (rebuild with --features hnsw); running UNGROUNDED"
            );
            return Ok(());
        };
        // The live embed scope keys every self-contained name (the stale-index escape).
        // `None` ⇒ no server embedder ⇒ a server-embed ingest could not succeed anyway,
        // so every binding takes the reference-existing path.
        let scope_tag = view.embed_scope_tag();
        let mut available: BTreeSet<String> = view
            .list_datasets()
            .into_iter()
            .map(|d| d.dataset_id)
            .collect();

        let mut resolved: Vec<String> = Vec::with_capacity(bindings.len());
        for b in bindings {
            resolved.push(
                self.resolve_dataset(view, scope_tag.as_deref(), &mut available, b)
                    .await?,
            );
        }
        for name in &resolved {
            if !available.contains(name) {
                return Err(AppRunError::InvalidArgs(format!(
                    "app grounds on dataset {name:?} but no such dataset is ingested; run \
                     `kx datasets ingest {name} …` first, then re-run"
                )));
            }
        }
        // Grant retrieve@1 on the entry root model step (agentic_step_warrant mints the
        // grant from the folded contract ∩ registry). `or_insert` ⇒ an author pin wins.
        let granted: BTreeMap<String, String> = [("retrieve".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        fold_skill_tools(dag, &granted);
        // Steer the entry step to USE retrieve on the RESOLVED dataset(s) — steer-only DATA,
        // never a grant (SN-8; the same class as `inject_app_args` / `fold_react_rag_dataset`).
        steer_dataset_prompt(dag, &resolved);
        Ok(())
    }

    /// The PHYSICAL dataset name one declared binding grounds on, materializing a
    /// self-contained corpus on first use. Inserts into `available` when it ingests, so
    /// the caller's presence check sees a just-created dataset.
    ///
    /// Precedence, in order:
    /// 1. no `cas_refs` (or no embedder) ⇒ the DECLARED name — today's reference-existing path;
    /// 2. the scoped name already exists ⇒ use it, **no ingest** — the cheap steady state
    ///    (the host embeds BEFORE its content-addressed dedup, so a blind re-ingest would
    ///    re-pay the whole embed cost on every run, not just the first);
    /// 3. else ingest the corpus under the scoped name ⇒ use it;
    /// 4. ingest skipped (blobs absent / over-ceiling / not text) ⇒ fall back to the
    ///    DECLARED name, which is exactly (1).
    async fn resolve_dataset(
        &self,
        view: &Arc<dyn DatasetView>,
        scope_tag: Option<&str>,
        available: &mut BTreeSet<String>,
        binding: &DatasetBinding,
    ) -> Result<String, AppRunError> {
        let (Some(tag), false) = (scope_tag, binding.cas_refs.is_empty()) else {
            return Ok(binding.declared.clone());
        };
        let scoped = app_dataset_scoped_name(tag, &binding.declared, &binding.cas_refs);
        if available.contains(&scoped) {
            return Ok(scoped);
        }
        if self.ensure_app_dataset(view, &scoped, binding).await? {
            available.insert(scoped.clone());
            return Ok(scoped);
        }
        Ok(binding.declared.clone())
    }

    /// `T-RUNAPP-RAG-SELF-CONTAINED`: materialize an App's declared corpus
    /// (`references.datasets[].cas_refs`) into `scoped`, so a SHARED App grounds on the
    /// bytes it carries instead of on the author's local datasets. Returns `true` iff the
    /// dataset is now ingested and queryable.
    ///
    /// FAIL-SOFT by design — every recoverable miss returns `false` (the caller falls back
    /// to the declared name, i.e. today's behavior), never a hard error:
    /// - **any blob absent** — the LEGITIMATE common state: an export without `--with-data`
    ///   still serializes `cas_refs`, it just does not ship the blobs;
    /// - **over-ceiling** — see [`MAX_APP_CORPUS_REFS`] / [`MAX_APP_CORPUS_BYTES`];
    /// - **not UTF-8** — a server-embed needs text, and `DatasetRef` carries no
    ///   `media_type` to declare otherwise (App corpora are text-only);
    /// - **no embedder / bad name / dim mismatch / stale index** — a `DatasetError` the
    ///   scoped name is meant to prevent, but never worth bricking the run over.
    ///
    /// Only a genuine backend failure (`DatasetError::Internal` — a poisoned lock, a failed
    /// write) is hard: the store is broken, and grounding on a silently-empty index would
    /// be worse than refusing.
    async fn ensure_app_dataset(
        &self,
        view: &Arc<dyn DatasetView>,
        scoped: &str,
        binding: &DatasetBinding,
    ) -> Result<bool, AppRunError> {
        let declared = &binding.declared;
        // SORT + DEDUPE to exactly the set `app_dataset_scoped_name` hashed. Both halves of
        // the contract must key on the SAME set: a repeated ref is ONE doc in the name, so
        // it must be one doc in the index too — otherwise a duplicate would re-pay the embed
        // cost (the host embeds BEFORE its content-addressed dedup) and be counted twice
        // against the ceilings. Sorting also pins the ingest order. Cheap + bounded: the
        // 1 MiB envelope cap bounds the raw list long before this.
        let mut refs: Vec<&str> = binding.cas_refs.iter().map(String::as_str).collect();
        refs.sort_unstable();
        refs.dedup();

        // Ceilings BEFORE any store read (Rule 8c — never unbounded work on untrusted
        // input), over the DEDUPED set: it is the real work, and the raw count would reject
        // a legal corpus that merely repeats a ref.
        if refs.len() > MAX_APP_CORPUS_REFS {
            tracing::warn!(
                dataset = declared,
                refs = refs.len(),
                ceiling = MAX_APP_CORPUS_REFS,
                "app corpus exceeds the cas_ref ceiling; NOT self-ingesting"
            );
            return Ok(false);
        }
        let mut blobs: Vec<Vec<u8>> = Vec::with_capacity(refs.len());
        let mut total: u64 = 0;
        for hexref in refs {
            // The envelope validator pins every cas_ref to 64-hex, so a decode miss here
            // means a store that disagrees with a validated envelope — skip, never panic.
            let Some(cref) = ContentRef::from_hex(hexref) else {
                tracing::warn!(dataset = declared, "app corpus names a malformed cas_ref");
                return Ok(false);
            };
            let Some(bytes) = self.content.get_ref(&cref) else {
                tracing::debug!(
                    dataset = declared,
                    "app corpus blob absent from the content store (exported without \
                     --with-data?); grounding on an EXISTING dataset of that name instead"
                );
                return Ok(false);
            };
            total += bytes.len() as u64;
            if total > MAX_APP_CORPUS_BYTES {
                tracing::warn!(
                    dataset = declared,
                    ceiling = MAX_APP_CORPUS_BYTES,
                    "app corpus exceeds the byte ceiling; NOT self-ingesting"
                );
                return Ok(false);
            }
            if std::str::from_utf8(&bytes).is_err() {
                tracing::warn!(
                    dataset = declared,
                    "app corpus blob is not UTF-8 (a server-embed needs text); NOT \
                     self-ingesting"
                );
                return Ok(false);
            }
            blobs.push(bytes);
        }

        // Embedding is a synchronous per-chunk model call and `ingest` is a sync seam;
        // `author_app` is async, so run it OFF the reactor rather than stalling a worker.
        let view = Arc::clone(view);
        let name = scoped.to_string();
        let doc_count = blobs.len();
        let outcome = tokio::task::spawn_blocking(move || {
            let docs: Vec<IngestDoc<'_>> = blobs
                .iter()
                .map(|b| IngestDoc {
                    content: b,
                    embedding: None,
                })
                .collect();
            view.ingest(&name, &docs)
        })
        .await
        .map_err(|e| AppRunError::Internal(format!("app corpus ingest panicked: {e}")))?;

        match outcome {
            Ok(o) => {
                tracing::info!(
                    dataset = declared,
                    scoped,
                    docs = doc_count,
                    inserted = o.inserted,
                    "self-contained app corpus ingested; grounding on the app's OWN bytes"
                );
                Ok(true)
            }
            Err(DatasetError::Internal(e)) => Err(AppRunError::Internal(format!(
                "app corpus ingest into {scoped:?} failed: {e}"
            ))),
            Err(e) => {
                tracing::warn!(
                    dataset = declared,
                    scoped,
                    error = ?e,
                    "could not self-ingest the app corpus; grounding on an EXISTING dataset \
                     of that name instead"
                );
                Ok(false)
            }
        }
    }
}

/// T-RUNAPP-CONTEXT-RAIL: steer the ENTRY root model step to USE `retrieve` on the
/// named dataset(s) — steer-only DATA, never a grant (SN-8; the same class as
/// [`inject_app_args`]). A NO-OP when there is no root model step (mirror
/// [`fold_skill_tools`]). Deterministic (declaration order) ⇒ recovery-stable. Pure.
fn steer_dataset_prompt(dag: &mut DagSpec, dataset_names: &[String]) {
    let Some(idx) = entry_agentic_step_index(dag) else {
        return;
    };
    let list = dataset_names.join(", ");
    let directive = format!(
        "Grounding: use the `retrieve` tool to search the dataset(s) [{list}] for relevant \
         passages BEFORE answering, and ground your answer in what you retrieve."
    );
    let step = &mut dag.steps[idx];
    step.prompt = format!("{}\n\n{directive}", step.prompt).trim().to_string();
}

/// One dataset an App grounds over: the name it DECLARED, plus the corpus it carries
/// for that name (empty ⇒ reference-existing; non-empty ⇒ self-contained).
#[derive(Clone, Debug, PartialEq, Eq)]
struct DatasetBinding {
    /// The author's declared `dataset_ref` — the reference-existing lookup key, and the
    /// readable half of the self-contained scoped name.
    declared: String,
    /// The 64-hex content refs the declared dataset spans (`T-RUNAPP-RAG-SELF-CONTAINED`).
    cas_refs: Vec<String>,
}

/// T-RUNAPP-CONTEXT-RAIL: the datasets an App grounds over — `references.datasets`
/// dataset refs UNIONed with `steering_config.context.dataset_refs`, deduped in
/// declaration order (empty names skipped). A steering ref names a dataset only, so it
/// carries no corpus; a `references.datasets` entry may carry one
/// (`T-RUNAPP-RAG-SELF-CONTAINED`). Pure.
///
/// First declaration wins on a duplicate name — so a corpus-bearing entry is not
/// displaced by a later bare mention of the same name, and the order (hence the steer
/// text) stays a pure function of the envelope ⇒ recovery-stable.
fn collect_dataset_bindings(env: &AppEnvelope) -> Vec<DatasetBinding> {
    let mut out: Vec<DatasetBinding> = Vec::new();
    let mut push = |declared: &String, cas_refs: Vec<String>| {
        if declared.is_empty() || out.iter().any(|b| &b.declared == declared) {
            return;
        }
        out.push(DatasetBinding {
            declared: declared.clone(),
            cas_refs,
        });
    };
    for d in &env.references.datasets {
        push(&d.dataset_ref, d.cas_refs.clone());
    }
    for n in &env.steering_config.context.dataset_refs {
        push(n, Vec::new());
    }
    out
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

/// The deduped multi-skill tool WISH union, in envelope (canonical-bytes)
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

/// T-RUNAPP-CONTEXT-RAIL: the combined tool WISH the entry step folds — the skill
/// wish union (envelope order) UNIONed with `steering_config.tools.requested_grants`.
/// Skills merge first (deterministic); a cross-source version conflict is FAIL-SOFT
/// first-occurrence-wins + a warning (a wish is NEVER authority — the server still
/// intersects it against caller-Use ∩ fireable ∩ registry ∩ compat, SN-8). A pure
/// function (Rule 5.2 — unit-testable).
fn combined_tool_wish(
    skills: &[SkillRef],
    steering_grants: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut wish = skill_wish_union(skills);
    for (id, version) in steering_grants {
        match wish.get(id) {
            None => {
                wish.insert(id.clone(), version.clone());
            }
            Some(kept) if kept != version => {
                tracing::warn!(
                    tool = %id, kept = %kept, dropped = %version,
                    "steering.tools wish version conflicts with a skill wish: skill wins"
                );
            }
            Some(_) => {}
        }
    }
    wish
}

/// Map a caller-authority [`BinderError`] into the App-run error surface (identical
/// variants). Centralized so the reach + wish resolution paths agree.
fn map_binder_err(e: BinderError) -> AppRunError {
    match e {
        BinderError::NotAuthorized => AppRunError::NotAuthorized,
        BinderError::InvalidArgs(d) => AppRunError::InvalidArgs(d),
        BinderError::Internal(d) => AppRunError::Internal(d),
    }
}

/// The caller's resolvable tool CEILING = `party_tool_authority ∩ fireable ∩
/// registry` — the tools this caller is registered AND allowed to fire on this
/// serve. The single source of truth shared by (a) the `Reach::InheritPrincipal`
/// wish in [`HostAppAuthor::author_app`] and (b) the capability manifest — so the
/// manifest can never report a tool "in policy" that the run would drop.
///
/// When [`party_tool_authority`] is `None` (the caller expressed no explicit tool
/// grants — the permissive local-owner default), the allowlist leg is a no-op and
/// the ceiling is `fireable ∩ registry` (BOUNDED — never unbounded; the broker only
/// registers what the operator wired). Deterministic (sorted `BTreeSet`s + pure
/// registry lookups) ⇒ the folded contract it seeds replays byte-identically.
///
/// # Errors
/// Returns [`BinderError`] from [`party_tool_authority`] (e.g. `NotAuthorized` when
/// the caller may not author blueprints at all).
pub(crate) fn principal_tool_ceiling(
    lib: &DemoLibrary,
    party: &str,
    registered: &dyn RegisteredToolsView,
    tools: &dyn ToolRegistry,
) -> Result<BTreeSet<(String, String)>, BinderError> {
    let allowlist = party_tool_authority(lib, party)?;
    let mut ceiling = BTreeSet::new();
    for (id, ver) in registered.registered_grants() {
        // Registry membership — author_app's `skill_union_grants` also requires it,
        // so the ceiling matches what the run would actually materialize.
        if tools
            .lookup(&ToolName(id.clone()), &ToolVersion(ver.clone()))
            .is_none()
        {
            continue;
        }
        if let Some(allow) = &allowlist {
            if !allow.contains(&(id.clone(), ver.clone())) {
                continue;
            }
        }
        ceiling.insert((id, ver));
    }
    Ok(ceiling)
}

/// Apply `reach` to the declared tool wish. `Explicit` keeps the declared wish
/// verbatim (the byte-identical default). `InheritPrincipal` REPLACES it with the
/// caller's tool `ceiling` — a REPLACE, never a UNION with the declared set (a union
/// would let an App reach a tool outside the ceiling; the forbidden SN-8 widen).
/// Because the wish is either the declared set or the ceiling, and the downstream
/// [`skill_union_grants`] fold only ever removes, the materialized contract is
/// always `⊆ wish` and `⊆ ceiling` (monotonic narrowing). A pure function
/// (Rule 5.2 — unit-testable without the gateway rig).
fn effective_tool_wish(
    reach: Reach,
    declared: BTreeMap<String, String>,
    ceiling: Option<&BTreeSet<(String, String)>>,
) -> BTreeMap<String, String> {
    match reach {
        Reach::Explicit => declared,
        Reach::InheritPrincipal => ceiling
            .map(|c| c.iter().map(|(id, v)| (id.clone(), v.clone())).collect())
            .unwrap_or_default(),
    }
}

/// The ENTRY agentic step — the first MODEL step that is a DAG ROOT (no
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

/// Fold the GRANTED (already-intersected) skill tools into the ENTRY
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

/// Resolve each [`SkillRef`]'s instructions into a labeled `skill:<name>`
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

/// T-RUNAPP-CONTEXT-RAIL: decode a 64-hex content ref + assert the blob is PRESENT
/// in the content store at author time (fail-closed — never run the model on PARTIAL
/// context; a dispatch-time miss would dead-letter the run opaquely). The exact gate
/// [`skill_context_items`] applies to a skill's `instructions_ref`, factored so every
/// rail item shares it. Returns the `[u8; 32]` ref for a [`ContextItemRef`].
fn decode_present_ref(
    field: &str,
    content_ref: &str,
    content: &dyn ContentPresence,
) -> Result<[u8; 32], AppRunError> {
    let bytes = crate::provision::decode_hex32(content_ref).ok_or_else(|| {
        // Defense-in-depth: already impossible past AppEnvelope::validate (check_ref).
        AppRunError::InvalidArgs(format!(
            "{field} is not a 64-hex content ref: {content_ref:?}"
        ))
    })?;
    if !content.contains_ref(&ContentRef::from_bytes(bytes)) {
        return Err(AppRunError::InvalidArgs(format!(
            "{field} ({}…) not found in the content store; upload it first \
             (kx content put / PutContent), then re-run",
            &content_ref[..12.min(content_ref.len())]
        )));
    }
    Ok(bytes)
}

/// T-RUNAPP-CONTEXT-RAIL: resolve the App's declarative KNOWLEDGE rail —
/// `references.context / prompts / rules / memory` + `steering_config.context.context_refs`
/// — into labeled [`ContextItemRef`]s, merged (alongside any skill instructions) through
/// the SAME identity-bearing entry-step `author_with_context_items` inject the skill bind uses.
/// Each item is FAIL-CLOSED on a blob missing from the content store (mirrors
/// [`skill_context_items`]). Labels keep each item legible in the assembled prompt
/// (`context.<prefix>:<name>` per `kx-context-assembler`); a raw steering ref (no name)
/// is labeled `ref:<hex12>` (the D155 raw-refs convention). `references.memory` here are
/// STATIC content notes (distinct from the durable `recall@1` store). An entirely empty
/// rail ⇒ empty `Vec` ⇒ `author_with_context_items` sees no items ⇒ byte-identical to the
/// plain author path (the digest `7d22d4bd` no-op).
fn context_rail_items(
    env: &AppEnvelope,
    content: &dyn ContentPresence,
) -> Result<Vec<ContextItemRef>, AppRunError> {
    let r = &env.references;
    let mut items = Vec::new();
    for c in &r.context {
        let bytes = decode_present_ref("context", &c.content_ref, content)?;
        items.push(ContextItemRef {
            name: format!("context:{}", c.name),
            content_ref: bytes,
        });
    }
    // prompts / rules / memory are all `ArtifactRef { name, content_ref }`, distinguished
    // only by their label prefix (the assembler heading the model reads).
    for (prefix, arts) in [
        ("prompt", &r.prompts),
        ("rule", &r.rules),
        ("memory", &r.memory),
    ] {
        for a in arts {
            let bytes = decode_present_ref(prefix, &a.content_ref, content)?;
            items.push(ContextItemRef {
                name: format!("{prefix}:{}", a.name),
                content_ref: bytes,
            });
        }
    }
    // steering_config.context.context_refs: raw 64-hex, no name ⇒ `ref:<hex12>`.
    for cr in &env.steering_config.context.context_refs {
        let bytes = decode_present_ref("context_ref", cr, content)?;
        items.push(ContextItemRef {
            name: format!("ref:{}", &cr[..12.min(cr.len())]),
            content_ref: bytes,
        });
    }
    Ok(items)
}

#[tonic::async_trait]
impl AppAuthor for HostAppAuthor {
    // A single linear resolve→lower→author→stamp pipeline (context rail + skills + RAG +
    // HITL); the steps read top-to-bottom and share local state, so splitting would only
    // scatter it. The T-APP-TRIGGER-TARGET HITL fold pushed it one line over the default.
    #[allow(clippy::too_many_lines)]
    async fn author_app(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
        require_approval: bool,
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

        // (1b) T-RUNAPP-CONTEXT-RAIL: resolve the App's declarative knowledge rail
        //      (context/prompts/rules/memory + steering context refs) into labeled
        //      context items BEFORE the blueprint is consumed. Skills (3b) extend this
        //      same Vec; the whole set rides ONE `author_with_context_items` inject.
        //      Empty rail ⇒ empty Vec ⇒ the digest no-op.
        let mut context_items = context_rail_items(&env, self.content.as_ref())?;
        // The datasets to ground over (collected now, while `env` is fully intact — the
        // blueprint move below partially moves `env`). Empty ⇒ no RAG fold (the no-op).
        let dataset_bindings = collect_dataset_bindings(&env);

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

        // (3a) PR-3: the App's model axis (Axis 1). A non-empty `steering_config.model.
        //      model_route` is a WISH intersected with the served catalog: if this serve
        //      offers it, pin it onto every model step that did not already name a model
        //      (an explicit per-step id wins); if it does NOT, REFUSE the run at submit —
        //      never silently run on a different model (SN-8: the user names the model, no
        //      auto-select, never degrade-to-primary). Empty route ⇒ no injection ⇒
        //      byte-identical to the pre-PR-3 path (the digest no-op).
        let route = &env.steering_config.model.model_route;
        if !route.is_empty() {
            if !self.lib.serve_model_ids().contains(route) {
                return Err(AppRunError::UnservedModelRoute(route.clone()));
            }
            for s in &mut dag.steps {
                if is_model_step(s) && s.model_id.is_empty() {
                    s.model_id.clone_from(route);
                }
            }
        }

        // (3b) skills + T-RUNAPP-CONTEXT-RAIL steering.tools: skill instructions →
        //      labeled context items (fail-closed CAS presence); the skill tool wishes
        //      UNIONed with steering_config.tools.requested_grants → ONE server-side
        //      intersection (wish ∩ caller-Use ∩ fireable ∩ registry ∩ compat) folded onto
        //      the entry model step's tool_contract (declared pins win). Structurally gated:
        //      no skills AND no steering wishes ⇒ zero new code runs (the digest no-op).
        if !env.references.skills.is_empty() {
            context_items.extend(skill_context_items(
                &env.references.skills,
                self.content.as_ref(),
            )?);
        }
        // `Reach::InheritPrincipal` REPLACES the declared wish with the caller's whole
        // tool ceiling (never a UNION — a union would widen past the ceiling, SN-8).
        // The fold below re-applies the SAME `allowlist ∩ fireable ∩ registry`, so the
        // materialized set is `ceiling ∩ compat ⊆ ceiling` (monotonic narrowing).
        // Default (`Explicit`) leaves the declared wish untouched — byte-identical, and
        // the ceiling is not even computed.
        let reach = env.steering_config.tools.reach;
        let ceiling = if reach == Reach::InheritPrincipal {
            Some(
                principal_tool_ceiling(
                    &self.lib,
                    party,
                    self.registered.as_ref(),
                    self.tools.as_ref(),
                )
                .map_err(map_binder_err)?,
            )
        } else {
            None
        };
        let wish = effective_tool_wish(
            reach,
            combined_tool_wish(
                &env.references.skills,
                &env.steering_config.tools.requested_grants,
            ),
            ceiling.as_ref(),
        );
        if !wish.is_empty() {
            // Use-gate + conditional narrowing (SN-8; see party_tool_authority).
            let allowlist = party_tool_authority(&self.lib, party).map_err(map_binder_err)?;
            let fireable = self.registered.registered_grants();
            // The declared contract seed is read from the SAME entry agentic step the
            // fold targets (the root model step), so an author pin on that step wins +
            // the fs/net compat union is seeded correctly.
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

        // (3c) T-RUNAPP-CONTEXT-RAIL: declarative RAG-on-App — the datasets the App
        //      grounds over (collected above) grant the entry step retrieve@1 + steer it to
        //      search them. A binding carrying `cas_refs` materializes its own corpus first
        //      (T-RUNAPP-RAG-SELF-CONTAINED). Empty ⇒ skipped (the digest no-op).
        if !dataset_bindings.is_empty() {
            self.fold_dataset_rag(&dataset_bindings, &mut dag).await?;
        }

        // (3d) T-APP-TRIGGER-TARGET / D114: stamp the per-run HITL posture onto the entry
        //      agentic step's config BEFORE the DAG is lowered — the coordinator's
        //      `react_seed_params` reads `require_approval` off the launch Mote's
        //      config_subset (a canonical-JSON bool; `"true"` lowers to `b"true"`). Injected
        //      pre-lowering so it is part of the launch MoteId (never post-author, which
        //      would change the id + orphan its edges). `false` ⇒ nothing injected ⇒
        //      byte-identical (the serve-wide KX_SERVE_REQUIRE_APPROVAL default applies).
        if require_approval {
            if let Some(i) = entry_agentic_step_index(&dag) {
                dag.steps[i]
                    .params
                    .insert(REACT_REQUIRE_APPROVAL_KEY.to_string(), "true".to_string());
            }
        }

        let req = to_request(dag).map_err(|e| AppRunError::InvalidArgs(e.to_string()))?;

        // T-RUNAPP-CONTEXT-RAIL: steering_config.context.bundle_handles attach as named
        // context bundles alongside any the blueprint already carries (resolved fail-closed
        // by author_with_context_items → resolve_context_items). Empty ⇒ req.context_bundles
        // verbatim ⇒ byte-identical.
        let mut context_bundles = req.context_bundles;
        context_bundles.extend(env.steering_config.context.bundle_handles.iter().cloned());

        // (4) Parse into the authoring vocabulary (SHARED with SubmitWorkflow) + author
        //     SERVER-SIDE (warrants from the party's grants, never the client — SN-8).
        //     The context rail (context/prompts/rules/memory/refs) + skill instructions ride
        //     as extra context items into the SAME entry bundle inject (empty set ⇒
        //     byte-identical to the plain author path).
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
                &context_bundles,
                &context_items,
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

/// Map a caller-authority [`BinderError`] into a [`GatewayError`] for the manifest seam.
fn binder_to_gateway(e: BinderError) -> GatewayError {
    match e {
        BinderError::NotAuthorized => GatewayError::NotAuthorized,
        BinderError::InvalidArgs(d) | BinderError::Internal(d) => GatewayError::Internal(d),
    }
}

impl AppManifestView for HostAppAuthor {
    /// Derive the READ-ONLY capability manifest for a caller-owned App: the requested
    /// tools/connections/model diffed against the caller's LIVE policy, using the SAME
    /// folds `author_app` applies at run ([`principal_tool_ceiling`], the connection
    /// registry, the served catalog) — so a capability the manifest reports `in_policy`
    /// is exactly one the run would grant. Advisory: it reads, never writes.
    fn manifest(&self, principal: &str, handle: &str) -> Result<Option<AppManifest>, GatewayError> {
        let Some((_, bytes)) = self
            .apps
            .get(principal, handle)
            .map_err(|e| GatewayError::Internal(format!("apps.db read: {e}")))?
        else {
            return Ok(None); // absent OR not-owned — uniform, no oracle.
        };
        let env = AppEnvelope::from_json_slice(&bytes)
            .map_err(|e| GatewayError::Internal(format!("stored envelope invalid: {e}")))?;

        let reach_inherit = env.steering_config.tools.reach == Reach::InheritPrincipal;
        // The declared wish (steering ∪ skills) and the caller's tool ceiling — the exact
        // two sets author_app resolves. A caller with NO tool authority (NotAuthorized ⇒
        // no Use grant) simply has an empty ceiling (nothing in policy), not an error.
        let wish = combined_tool_wish(
            &env.references.skills,
            &env.steering_config.tools.requested_grants,
        );
        let ceiling = match principal_tool_ceiling(
            &self.lib,
            principal,
            self.registered.as_ref(),
            self.tools.as_ref(),
        ) {
            Ok(c) => c,
            Err(BinderError::NotAuthorized) => BTreeSet::new(),
            Err(e) => return Err(binder_to_gateway(e)),
        };

        // Tool lines: the declared wish, plus the ceiling when the App inherits it.
        // `in_policy` = ∈ ceiling (fireable+registered+allowed); `inherited` = surfaced
        // only because reach=InheritPrincipal (in the ceiling, not explicitly declared).
        let mut keys: BTreeSet<(String, String)> =
            wish.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if reach_inherit {
            keys.extend(ceiling.iter().cloned());
        }
        let tools = keys
            .into_iter()
            .map(|(id, version)| {
                let requested = wish.get(&id).is_some_and(|v| *v == version);
                let in_policy = ceiling.contains(&(id.clone(), version.clone()));
                AppCapability {
                    inherited: reach_inherit && in_policy && !requested,
                    requested,
                    in_policy,
                    id,
                    version,
                }
            })
            .collect();

        // Connection lines: each referenced connection vs. the caller's registry, using
        // the SAME match resolve_secret_scope applies (by credential name, else endpoint).
        let registered = self
            .connections
            .list()
            .map_err(|e| GatewayError::Internal(format!("connections.db read: {e}")))?;
        let reg_creds: BTreeSet<String> = registered
            .iter()
            .filter_map(|c| c.credential_ref.clone())
            .collect();
        let reg_endpoints: BTreeSet<String> = registered
            .iter()
            .map(|c| c.transport.endpoint().to_string())
            .collect();
        let connections = env
            .references
            .connections
            .iter()
            .map(|c| {
                let in_policy = if c.credential_ref.is_empty() {
                    reg_endpoints.contains(&c.descriptor)
                } else {
                    reg_creds.contains(&c.credential_ref)
                };
                AppCapability {
                    id: c.descriptor.clone(),
                    version: String::new(),
                    requested: true,
                    in_policy,
                    inherited: false,
                }
            })
            .collect();

        // Model line: the declared route vs. the served catalog (empty ⇒ served default).
        let model_route = env.steering_config.model.model_route.clone();
        let model_route_served =
            model_route.is_empty() || self.lib.serve_model_ids().contains(&model_route);

        Ok(Some(AppManifest {
            reach_inherit,
            tools,
            connections,
            model_route,
            model_route_served,
        }))
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

    // ----- skill-bind pure helpers -----

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

    // ----- author_app end-to-end (the digest no-op pin + the skill bind) -----

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

    /// The test registry: `echo-tool@1` (skill/steering fold tests) + `retrieve@1`
    /// (the RAG-on-App dataset fold test — the entry-step retrieve grant is minted from
    /// the contract ∩ registry, so retrieve must be REGISTERED for the fold to author).
    /// Both are ReadOnlyNondet-compatible builtins (empty syscall / net / fs); the extra
    /// retrieve def is inert for the echo tests (grants are minted only for contract tools).
    fn echo_registry() -> Arc<dyn ToolRegistry> {
        use kx_tool_registry::{
            IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance,
        };
        use kx_warrant::{FsScope, NetScope, ResourceCeiling};
        let builtin = |id: &str| ToolDef {
            tool_id: kx_mote::ToolName(id.into()),
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
        let mut reg = InMemoryToolRegistry::new();
        for id in ["echo-tool", "retrieve"] {
            let _ = reg.register(
                builtin(id),
                ToolProvenance::HumanAuthored { author: "t".into() },
            );
        }
        Arc::new(reg)
    }

    /// A test [`DatasetView`]: `list_datasets` drives the App-run presence check, and
    /// `ingest` RECORDS its calls so a test can assert what a self-contained App
    /// materialized — and, just as load-bearing, that a second run re-ingests NOTHING.
    /// `query` is unused by `author_app`.
    struct FakeDatasets {
        present: std::sync::Mutex<BTreeSet<String>>,
        /// `None` models a host with NO server embedder ⇒ the self-contained path never
        /// engages and every binding takes the reference-existing route.
        scope_tag: Option<String>,
        ingests: std::sync::Mutex<Vec<(String, Vec<Vec<u8>>)>>,
        ingest_err: Option<fn() -> DatasetError>,
    }

    impl FakeDatasets {
        /// Reference-existing only (no embed scope) — no carried corpus can engage.
        fn new(present: &[&str]) -> Self {
            Self {
                present: std::sync::Mutex::new(present.iter().map(|s| (*s).to_string()).collect()),
                scope_tag: None,
                ingests: std::sync::Mutex::new(Vec::new()),
                ingest_err: None,
            }
        }

        /// A host WITH a server embedder ⇒ the self-contained corpus path is live.
        fn embedding(present: &[&str]) -> Self {
            Self {
                scope_tag: Some("scope-m1".to_string()),
                ..Self::new(present)
            }
        }

        /// An embedding host whose ingest always fails with `err`.
        fn failing(present: &[&str], err: fn() -> DatasetError) -> Self {
            Self {
                ingest_err: Some(err),
                ..Self::embedding(present)
            }
        }

        fn ingests(&self) -> Vec<(String, Vec<Vec<u8>>)> {
            self.ingests.lock().unwrap().clone()
        }
    }

    impl DatasetView for FakeDatasets {
        fn embed_scope_tag(&self) -> Option<String> {
            self.scope_tag.clone()
        }
        fn list_datasets(&self) -> Vec<kx_gateway_core::DatasetSummaryEntry> {
            self.present
                .lock()
                .unwrap()
                .iter()
                .map(|id| kx_gateway_core::DatasetSummaryEntry {
                    dataset_id: id.clone(),
                    name: id.clone(),
                    doc_count: 1,
                    dim: 0,
                    created_ms: 0,
                    chunked: false,
                    embed_model_fingerprint: String::new(),
                    index_version: 0,
                    chunk_count: 1,
                })
                .collect()
        }
        fn ingest(
            &self,
            dataset: &str,
            docs: &[kx_gateway_core::IngestDoc<'_>],
        ) -> Result<kx_gateway_core::IngestOutcome, DatasetError> {
            if let Some(err) = self.ingest_err {
                return Err(err());
            }
            self.ingests.lock().unwrap().push((
                dataset.to_string(),
                docs.iter().map(|d| d.content.to_vec()).collect(),
            ));
            self.present.lock().unwrap().insert(dataset.to_string());
            Ok(kx_gateway_core::IngestOutcome {
                dataset_id: dataset.to_string(),
                doc_count: docs.len() as u64,
                inserted: docs.len() as u64,
                dim: 4,
            })
        }
        fn query(
            &self,
            _dataset: &str,
            _emb: Option<&[f32]>,
            _text: &str,
            _k: usize,
            _mode: kx_gateway_core::RetrievalMode,
            _rerank: Option<bool>,
        ) -> Result<Vec<kx_gateway_core::DatasetHitEntry>, DatasetError> {
            Ok(Vec::new()) // author_app never queries.
        }
    }

    /// A full [`HostAppAuthor`] over a tempdir: served model "m", the echo-tool
    /// registry, an explicit fireable set, and an in-memory content store. No dataset
    /// view (RAG-on-App off) — use [`rig_ex`] to attach one.
    fn rig(
        dir: &std::path::Path,
        fireable: &[(&str, &str)],
    ) -> (
        HostAppAuthor,
        Arc<InMemoryContentStore>,
        Arc<HostWorkflowAuthor>,
    ) {
        rig_ex(dir, fireable, None)
    }

    /// A rig whose serve offers `secondaries` alongside the primary `m` — for the
    /// model-axis tests (a step routing to a secondary served model).
    fn rig_with_secondaries(
        dir: &std::path::Path,
        fireable: &[(&str, &str)],
        secondaries: &[&str],
    ) -> (
        HostAppAuthor,
        Arc<InMemoryContentStore>,
        Arc<HostWorkflowAuthor>,
    ) {
        let secs: Vec<ModelId> = secondaries
            .iter()
            .map(|s| ModelId((*s).to_string()))
            .collect();
        let lib = Arc::new(
            DemoLibrary::open_serve(
                dir,
                kx_warrant::ExecutorClass::Bwrap,
                &["alice@acme".to_string()],
                Some(&ModelId("m".into())),
                None,
                None,
                None,
                false,
                None,
                false,
                &secs,
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
            None,
        );
        (host, content, author)
    }

    /// [`rig`] plus an optional dataset view (the RAG-on-App presence-check seam).
    fn rig_ex(
        dir: &std::path::Path,
        fireable: &[(&str, &str)],
        datasets: Option<Arc<dyn DatasetView>>,
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
            datasets,
        );
        (host, content, author)
    }

    fn save_app(host: &HostAppAuthor, env: &AppEnvelope) -> String {
        let handle = "team/apps/t".to_string();
        host.apps
            .save(
                "alice@acme",
                &handle,
                &env.to_canonical_json().unwrap(),
                None,
            )
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
        let via_app = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

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

    #[test]
    fn effective_tool_wish_never_unions_past_the_ceiling() {
        // Exhaustive over a 4-tool universe (all 16×16 declared/ceiling subset pairs):
        // Explicit keeps the declared wish; InheritPrincipal yields EXACTLY the ceiling
        // and NEVER a declared tool outside it (a union would). This is the SN-8
        // monotonic-narrowing / no-widen invariant on the wish selection; the downstream
        // `skill_union_grants` fold then only narrows further (⊆ wish). A complete proof
        // for the space (an exhaustive enumeration, not random sampling).
        let universe = ["a", "b", "c", "d"];
        let n = universe.len();
        let subset = |mask: u32| -> Vec<String> {
            (0..n)
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| universe[i].to_string())
                .collect()
        };
        for dmask in 0u32..(1 << n) {
            for cmask in 0u32..(1 << n) {
                let declared: BTreeMap<String, String> = subset(dmask)
                    .into_iter()
                    .map(|k| (k, "1".to_string()))
                    .collect();
                let ceiling: BTreeSet<(String, String)> = subset(cmask)
                    .into_iter()
                    .map(|k| (k, "1".to_string()))
                    .collect();

                let explicit =
                    effective_tool_wish(Reach::Explicit, declared.clone(), Some(&ceiling));
                assert_eq!(
                    explicit, declared,
                    "Explicit keeps the declared wish verbatim"
                );

                let inherit =
                    effective_tool_wish(Reach::InheritPrincipal, declared.clone(), Some(&ceiling));
                let inherit_set: BTreeSet<(String, String)> = inherit
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert!(inherit_set.is_subset(&ceiling), "materialized ⊆ ceiling");
                assert_eq!(
                    inherit_set, ceiling,
                    "InheritPrincipal replaces the wish with the ceiling"
                );
                for id in declared.keys() {
                    if !ceiling.iter().any(|(cid, _)| cid == id) {
                        assert!(
                            !inherit.contains_key(id),
                            "declared {id:?} outside the ceiling must not appear (no union)"
                        );
                    }
                }
            }
        }
        // InheritPrincipal with no ceiling ⇒ empty wish (fail-closed, never a widen).
        assert!(effective_tool_wish(Reach::InheritPrincipal, BTreeMap::new(), None).is_empty());
    }

    #[test]
    fn principal_tool_ceiling_is_fireable_intersect_registry() {
        let dir = tempfile::tempdir().unwrap();
        // echo-tool@1 is fireable AND in echo_registry; not-registered@9 is fireable but
        // absent from the registry ⇒ excluded (matching skill_union_grants). alice has no
        // explicit tool allowlist ⇒ no further narrowing.
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1"), ("not-registered", "9")]);
        let ceiling = principal_tool_ceiling(
            &host.lib,
            "alice@acme",
            host.registered.as_ref(),
            host.tools.as_ref(),
        )
        .unwrap();
        assert_eq!(
            ceiling,
            [("echo-tool".to_string(), "1".to_string())]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            "ceiling = fireable ∩ registry (the unregistered tool is dropped)"
        );
    }

    #[tokio::test]
    async fn author_app_reach_inherit_principal_folds_the_whole_ceiling() {
        // An App declaring NO tools but reach=InheritPrincipal inherits the caller's
        // whole fireable ∩ registry ceiling onto its entry agentic step.
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1"), ("retrieve", "1")]);
        let mut env = AppEnvelope::new(
            "inheritor",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config.tools.reach = Reach::InheritPrincipal;
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

        let (mote, warrant) = bound
            .motes
            .iter()
            .find(|(_, w)| !w.tool_grants.is_empty())
            .expect("an agentic mote carrying the inherited ceiling");
        for id in ["echo-tool", "retrieve"] {
            assert!(
                mote.def
                    .tool_contract
                    .contains_key(&kx_mote::ToolName(id.into())),
                "the whole ceiling ({id}) folds under InheritPrincipal"
            );
            assert!(warrant
                .tool_grants
                .iter()
                .any(|g| g.tool_id.0 == id && g.tool_version.0 == "1"));
        }
    }

    #[tokio::test]
    async fn author_app_explicit_default_grants_no_undeclared_tools() {
        // The default (Explicit) reach with NO declared wish grants nothing — the entry
        // step stays a plain transform (byte-identical to the pre-reach behavior), even
        // though the caller HAS a fireable ceiling.
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1"), ("retrieve", "1")]);
        let env = AppEnvelope::new(
            "plain",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        assert_eq!(env.steering_config.tools.reach, Reach::Explicit);
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        assert!(
            bound.motes.iter().all(|(_, w)| w.tool_grants.is_empty()),
            "Explicit + no declared wish grants nothing (the entry step stays plain)"
        );
    }

    #[tokio::test]
    async fn author_app_unserved_model_route_refuses_at_submit() {
        // An App naming a model this serve does not offer REFUSES loudly at submit —
        // it never silently authors on the primary (SN-8: no auto-select / degrade).
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        let mut env = AppEnvelope::new(
            "ghosted",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config.model.model_route = "kx-serve:ghost".into();
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        match host.author_app("alice@acme", &handle, b"", false).await {
            Err(AppRunError::UnservedModelRoute(r)) => assert_eq!(r, "kx-serve:ghost"),
            Err(other) => panic!("expected UnservedModelRoute, got {other:?}"),
            Ok(_) => panic!("expected UnservedModelRoute REFUSE, but the run authored"),
        }
    }

    #[tokio::test]
    async fn author_app_routes_to_a_served_secondary_model() {
        // model_route names a SECONDARY served model (which step_def previously rejected).
        // The relaxation routes the step there: the authored MoteDef carries the secondary
        // id AND the step warrant's model_route matches it (the dispatcher equality gate).
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig_with_secondaries(dir.path(), &[], &["kx-serve:beta"]);
        let mut env = AppEnvelope::new(
            "routed",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config.model.model_route = "kx-serve:beta".into();
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        let beta = ModelId("kx-serve:beta".into());
        assert!(
            bound
                .motes
                .iter()
                .any(|(m, w)| m.def.model_id == beta && w.model_route.model_id == beta),
            "the secondary route is pinned onto BOTH the MoteDef and its warrant"
        );
    }

    #[test]
    fn manifest_diffs_declared_needs_against_policy() {
        let dir = tempfile::tempdir().unwrap();
        // echo-tool@1 is fireable + registered; gmail/search@1 is neither.
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        let mut env = AppEnvelope::new(
            "assistant",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config.tools.requested_grants = [
            ("echo-tool".to_string(), "1".to_string()),
            ("gmail/search".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect();
        env.references.connections.push(ConnectionRef {
            descriptor: "mcp+stdio://gmail".into(),
            credential_ref: "KX_GMAIL_CREDENTIAL".into(),
        });
        env.steering_config.model.model_route = "m".into(); // the served primary
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let m = host
            .manifest("alice@acme", &handle)
            .unwrap()
            .expect("owned app");

        assert!(!m.reach_inherit);
        let echo = m.tools.iter().find(|c| c.id == "echo-tool").unwrap();
        assert!(echo.requested && echo.in_policy && !echo.inherited);
        let gmail = m.tools.iter().find(|c| c.id == "gmail/search").unwrap();
        assert!(
            gmail.requested && !gmail.in_policy,
            "requested but not fireable ⇒ a missing capability"
        );
        let conn = m
            .connections
            .iter()
            .find(|c| c.id == "mcp+stdio://gmail")
            .unwrap();
        assert!(
            conn.requested && !conn.in_policy,
            "an unregistered connection is requested but not in policy"
        );
        assert_eq!(m.model_route, "m");
        assert!(m.model_route_served);
    }

    #[tokio::test]
    async fn manifest_inherit_flags_inherited_and_predicts_the_run() {
        // Under InheritPrincipal the manifest reports the whole ceiling as in-policy +
        // inherited, and its in-policy tool set equals exactly what the run grants.
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1"), ("retrieve", "1")]);
        let mut env = AppEnvelope::new(
            "inheritor",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config.tools.reach = Reach::InheritPrincipal;
        env.validate().unwrap();
        let handle = save_app(&host, &env);

        let m = host
            .manifest("alice@acme", &handle)
            .unwrap()
            .expect("owned app");
        assert!(m.reach_inherit);
        assert!(
            m.tools
                .iter()
                .all(|c| c.inherited && c.in_policy && !c.requested),
            "no tool was explicitly declared ⇒ every ceiling tool is inherited"
        );
        let manifest_in_policy: BTreeSet<(String, String)> = m
            .tools
            .iter()
            .filter(|c| c.in_policy)
            .map(|c| (c.id.clone(), c.version.clone()))
            .collect();

        // Parity by construction: the run grants EXACTLY the manifest's in-policy set.
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        let run_grants: BTreeSet<(String, String)> = bound
            .motes
            .iter()
            .flat_map(|(_, w)| {
                w.tool_grants
                    .iter()
                    .map(|g| (g.tool_id.0.clone(), g.tool_version.0.clone()))
            })
            .collect();
        assert_eq!(
            manifest_in_policy, run_grants,
            "the manifest predicts exactly what the run grants"
        );
    }

    #[test]
    fn manifest_flags_an_unserved_model_route() {
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        let mut env = AppEnvelope::new(
            "routed",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config.model.model_route = "kx-serve:ghost".into();
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let m = host
            .manifest("alice@acme", &handle)
            .unwrap()
            .expect("owned app");
        assert_eq!(m.model_route, "kx-serve:ghost");
        assert!(
            !m.model_route_served,
            "an unserved route is flagged in the manifest before the run refuses it"
        );
    }

    #[test]
    fn manifest_for_an_absent_app_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        assert!(host
            .manifest("alice@acme", "team/apps/ghost")
            .unwrap()
            .is_none());
    }

    /// D114 / T-APP-TRIGGER-TARGET: `require_approval = true` stamps the HITL posture
    /// (the canonical-JSON bool the coordinator's `react_seed_params` reads) onto the
    /// entry agentic step's `config_subset`; `false` injects nothing (byte-identical to
    /// today). The posture is part of the launch MoteId — so the same App with vs without
    /// approval produces distinct ids (injected BEFORE lowering, never post-author).
    #[tokio::test]
    async fn author_app_require_approval_stamps_hitl_and_false_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        // An agentic entry step (MODEL + a non-empty tool_contract) — the step the HITL
        // posture (and the react loop) attach to.
        let env = AppEnvelope::new(
            "gated",
            serde_json::json!({
                "steps": [{ "kind": "model", "prompt": "go", "tool_contract": { "echo-tool": "1" } }]
            }),
        );
        let handle = save_app(&host, &env);

        let with_hitl = host
            .author_app("alice@acme", &handle, b"", true)
            .await
            .unwrap();
        let without = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

        let key = kx_mote::ConfigKey(REACT_REQUIRE_APPROVAL_KEY.to_string());
        let (m_hitl, _) = &with_hitl.motes[0];
        let (m_plain, _) = &without.motes[0];
        // true ⇒ the canonical-JSON bool `true` on the entry mote's identity-bearing config.
        assert_eq!(
            m_hitl.def.config_subset.get(&key).map(|v| v.0.clone()),
            Some(b"true".to_vec()),
            "require_approval=true stamps the HITL posture"
        );
        // false ⇒ nothing injected (byte-identical to a plain agentic App).
        assert!(
            !m_plain.def.config_subset.contains_key(&key),
            "require_approval=false injects nothing"
        );
        // The posture is part of the launch identity (folded before lowering).
        assert_ne!(m_hitl.id, m_plain.id, "the HITL bit is part of the mote id");
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
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

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

    // ----- T-RUNAPP-CONTEXT-RAIL: the declarative knowledge rail -----

    /// The rail helper labels every kind (`context:`/`prompt:`/`rule:`/`memory:`/`ref:`),
    /// carries the exact ref bytes, and FAILS CLOSED on a blob missing from the store.
    #[test]
    fn context_rail_items_labels_every_kind_and_fails_closed() {
        use kx_content::{ContentStore as _, InMemoryContentStore};
        let store = InMemoryContentStore::new();
        let ctx = store.put(b"context body").unwrap();
        let prompt = store.put(b"prompt body").unwrap();
        let rule = store.put(b"rule body").unwrap();
        let mem = store.put(b"memory body").unwrap();
        let sref = store.put(b"steering ref body").unwrap();

        let mut env = AppEnvelope::new(
            "rails",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.references.context.push(kx_app::ContextRef {
            name: "c1".into(),
            content_ref: hex_str(&ctx.0),
            media_type: "text/plain".into(),
        });
        env.references.prompts.push(kx_app::ArtifactRef {
            name: "p1".into(),
            content_ref: hex_str(&prompt.0),
        });
        env.references.rules.push(kx_app::ArtifactRef {
            name: "r1".into(),
            content_ref: hex_str(&rule.0),
        });
        env.references.memory.push(kx_app::ArtifactRef {
            name: "m1".into(),
            content_ref: hex_str(&mem.0),
        });
        env.steering_config
            .context
            .context_refs
            .push(hex_str(&sref.0));

        let items = context_rail_items(&env, &store).unwrap();
        let named = |n: &str| items.iter().find(|i| i.name == n);
        assert_eq!(named("context:c1").unwrap().content_ref, ctx.0);
        assert_eq!(named("prompt:p1").unwrap().content_ref, prompt.0);
        assert_eq!(named("rule:r1").unwrap().content_ref, rule.0);
        assert_eq!(named("memory:m1").unwrap().content_ref, mem.0);
        assert!(
            items
                .iter()
                .any(|i| i.name.starts_with("ref:") && i.content_ref == sref.0),
            "raw steering ref labeled ref:<hex12>: {items:?}"
        );

        // A blob absent from the store ⇒ fail-closed (never a partial-context run).
        let mut bad = AppEnvelope::new(
            "bad",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        bad.references.rules.push(kx_app::ArtifactRef {
            name: "x".into(),
            content_ref: "d".repeat(64),
        });
        let err = context_rail_items(&bad, &store).unwrap_err();
        match err {
            AppRunError::InvalidArgs(msg) => {
                assert!(msg.contains("not found in the content store"), "{msg}");
                assert!(msg.contains("dddddddddddd"), "names the ref prefix: {msg}");
            }
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    /// End-to-end: an App declaring `references.rules` + `references.memory` (no skills)
    /// injects those as labeled items on the entry mote's identity-bearing context —
    /// the App self-grounds without a hand-authored blueprint.
    #[tokio::test]
    async fn author_app_with_a_context_rail_injects_labeled_items() {
        let dir = tempfile::tempdir().unwrap();
        let (host, content, _) = rig(dir.path(), &[]);
        let rule =
            kx_content::ContentStore::put(content.as_ref(), b"# Cite your sources.").unwrap();
        let note = kx_content::ContentStore::put(content.as_ref(), b"Prior finding: whales sing.")
            .unwrap();

        let mut env = AppEnvelope::new(
            "grounded",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.references.rules.push(kx_app::ArtifactRef {
            name: "cite".into(),
            content_ref: hex_str(&rule.0),
        });
        env.references.memory.push(kx_app::ArtifactRef {
            name: "prior".into(),
            content_ref: hex_str(&note.0),
        });
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

        let (mote, _warrant) = &bound.motes[0];
        let encoded = mote
            .def
            .config_subset
            .get(&ConfigKey(CONTEXT_ITEMS_KEY.into()))
            .expect("entry mote carries CONTEXT_ITEMS");
        let items = kx_mote::decode_context_items(&encoded.0);
        assert!(
            items
                .iter()
                .any(|i| i.name == "rule:cite" && i.content_ref == rule.0),
            "labeled rule present: {items:?}"
        );
        assert!(
            items
                .iter()
                .any(|i| i.name == "memory:prior" && i.content_ref == note.0),
            "labeled memory note present: {items:?}"
        );
    }

    /// A rail-bearing App legitimately produces a DIFFERENT entry MoteId than the plain
    /// blueprint (identity-bearing context) — but is idempotent (recovery-stable). The
    /// digest no-op is proven separately by `author_app_without_skills_is_byte_identical`.
    #[tokio::test]
    async fn author_app_with_a_rail_is_identity_bearing_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let (host, content, author) = rig(dir.path(), &[]);
        let rule = kx_content::ContentStore::put(content.as_ref(), b"# Rule.").unwrap();
        let mut env = AppEnvelope::new(
            "grounded",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.references.rules.push(kx_app::ArtifactRef {
            name: "r".into(),
            content_ref: hex_str(&rule.0),
        });
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let a = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        let b = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

        // The plain (no-rail) reference over the same blueprint.
        let d = dag(vec![model_step("go")]);
        let req = to_request(d).unwrap();
        let (steps, edges, mode) =
            author_steps_from_proto(req.steps, req.edges, req.execution_mode).unwrap();
        let plain = author
            .author("alice@acme", req.seed, &steps, &edges, mode, &[])
            .await
            .unwrap();

        assert_ne!(
            a.motes[0].0.id, plain.motes[0].0.id,
            "a rail changes the entry MoteId (identity-bearing context)"
        );
        assert_eq!(
            a.motes[0].0.id, b.motes[0].0.id,
            "the same rail re-derives the same MoteId (recovery-stable)"
        );
    }

    #[test]
    fn combined_tool_wish_unions_skills_and_steering_skill_wins_on_conflict() {
        let skills = vec![skill(
            "a",
            &"a".repeat(64),
            &[("echo-tool", "1"), ("retrieve", "1")],
        )];
        let steering: BTreeMap<String, String> = [
            ("echo-tool".to_string(), "9".to_string()), // conflicts with the skill wish (1)
            ("fs-read".to_string(), "1".to_string()),   // steering-only
        ]
        .into_iter()
        .collect();
        let wish = combined_tool_wish(&skills, &steering);
        assert_eq!(
            wish["echo-tool"], "1",
            "skill wins the cross-source conflict"
        );
        assert_eq!(wish["retrieve"], "1");
        assert_eq!(wish["fs-read"], "1", "steering-only wish included");
        assert_eq!(wish.len(), 3);
        // No skills ⇒ steering wishes stand alone.
        assert_eq!(combined_tool_wish(&[], &steering).len(), 2);
        // Neither ⇒ empty ⇒ the fold is skipped (the digest no-op).
        assert!(combined_tool_wish(&[], &BTreeMap::new()).is_empty());
    }

    /// steering_config.tools.requested_grants folds a REAL grant onto the entry step
    /// even with NO skills (the tools-steering axis, server-intersected — SN-8).
    #[tokio::test]
    async fn author_app_with_steering_tools_folds_grants_without_skills() {
        let dir = tempfile::tempdir().unwrap();
        let (host, _content, _) = rig(dir.path(), &[("echo-tool", "1")]);
        let mut env = AppEnvelope::new(
            "steered",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "go" }] }),
        );
        env.steering_config
            .tools
            .requested_grants
            .insert("echo-tool".into(), "1".into());
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        let (mote, warrant) = &bound.motes[0];
        assert!(
            mote.def
                .tool_contract
                .contains_key(&kx_mote::ToolName("echo-tool".into())),
            "steering wish folded onto the entry step contract"
        );
        assert!(
            warrant
                .tool_grants
                .iter()
                .any(|g| g.tool_id.0 == "echo-tool" && g.tool_version.0 == "1"),
            "steering wish became a real (intersected) grant"
        );
    }

    // ----- T-RUNAPP-CONTEXT-RAIL: declarative RAG-on-App (datasets → retrieve@1) -----

    #[test]
    fn steer_dataset_prompt_appends_a_grounding_directive_naming_datasets() {
        let mut d = dag(vec![model_step("Answer the question.")]);
        steer_dataset_prompt(&mut d, &["science".to_string(), "history".to_string()]);
        assert!(
            d.steps[0].prompt.contains("retrieve"),
            "{}",
            d.steps[0].prompt
        );
        assert!(
            d.steps[0].prompt.contains("science, history"),
            "names the datasets: {}",
            d.steps[0].prompt
        );
        assert!(d.steps[0].prompt.starts_with("Answer the question."));
        // No root model step ⇒ no-op (mirror fold_skill_tools).
        let mut none = dag(vec![pure_step()]);
        steer_dataset_prompt(&mut none, &["science".to_string()]);
        assert!(none.steps[0].prompt.is_empty());
    }

    /// An App declaring `references.datasets` grants the entry step retrieve@1 (contract +
    /// warrant, server-authorized) AND steers the entry prompt — declarative RAG-on-App.
    #[tokio::test]
    async fn author_app_with_a_dataset_grants_retrieve_and_steers() {
        let dir = tempfile::tempdir().unwrap();
        let ds: Arc<dyn DatasetView> = Arc::new(FakeDatasets::new(&["science"]));
        let (host, _c, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(ds));
        let mut env = AppEnvelope::new(
            "grounded",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "Answer." }] }),
        );
        env.references.datasets.push(kx_app::DatasetRef {
            dataset_ref: "science".into(),
            cas_refs: vec![],
        });
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        let (mote, warrant) = &bound.motes[0];
        assert!(
            mote.def
                .tool_contract
                .contains_key(&kx_mote::ToolName("retrieve".into())),
            "retrieve@1 folded onto the entry step"
        );
        assert!(
            warrant
                .tool_grants
                .iter()
                .any(|g| g.tool_id.0 == "retrieve" && g.tool_version.0 == "1"),
            "retrieve@1 is a real grant (server-authorized by the operator's dataset)"
        );
    }

    /// The one-doc corpus a self-contained App carries in these tests.
    const CORPUS_DOC: &[u8] = b"Water boils at 100C.";

    /// [`CORPUS_DOC`]'s content ref — the content store is content-addressed, so this is
    /// the ref ANY store derives for those bytes.
    fn content_hex() -> String {
        ContentRef::of(CORPUS_DOC).to_hex()
    }

    /// A `grounded` App declaring `science` over `corpus`, saved and ready to author.
    fn grounded_app(host: &HostAppAuthor, corpus: Vec<String>) -> String {
        let mut env = AppEnvelope::new(
            "grounded",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "Answer." }] }),
        );
        env.references.datasets.push(kx_app::DatasetRef {
            dataset_ref: "science".into(),
            cas_refs: corpus,
        });
        env.validate().unwrap();
        save_app(host, &env)
    }

    /// One `science` binding over `corpus`.
    fn science(corpus: &[String]) -> Vec<DatasetBinding> {
        vec![DatasetBinding {
            declared: "science".into(),
            cas_refs: corpus.to_vec(),
        }]
    }

    /// Fold the RAG rail over a one-model-step DAG and hand back the steered prompt.
    /// The authored `MoteDef` carries only a `prompt_template_hash` (SN-8 — identity, not
    /// text), so the resolved dataset NAME is only observable at the `DagSpec` level.
    async fn fold_and_steer(
        host: &HostAppAuthor,
        bindings: &[DatasetBinding],
    ) -> Result<String, AppRunError> {
        let mut d = dag(vec![model_step("Answer.")]);
        host.fold_dataset_rag(bindings, &mut d).await?;
        Ok(d.steps[0].prompt.clone())
    }

    /// THE TOKEN (`T-RUNAPP-RAG-SELF-CONTAINED`): an App carrying its corpus in
    /// `cas_refs` grounds on a host where NO dataset of that name exists — it
    /// materializes its OWN bytes under the scoped name and steers the model at that
    /// name. This is the whole point: a shared App is self-grounding.
    #[tokio::test]
    async fn author_app_self_ingests_a_carried_corpus_with_no_source_dataset() {
        let dir = tempfile::tempdir().unwrap();
        // NOTE: the store starts EMPTY — there is no `science` to fall back on.
        let ds = Arc::new(FakeDatasets::embedding(&[]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));
        let corpus = vec![
            store.put(b"Water boils at 100C.").unwrap().to_hex(),
            store.put(b"Iron melts at 1538C.").unwrap().to_hex(),
        ];
        let handle = grounded_app(&host, corpus.clone());

        // End-to-end: the run authors AND retrieve@1 is really granted, on a host with no
        // `science` dataset — which is a fail-closed InvalidArgs without a carried corpus.
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        let (mote, warrant) = &bound.motes[0];
        assert!(mote
            .def
            .tool_contract
            .contains_key(&kx_mote::ToolName("retrieve".into())));
        assert!(warrant
            .tool_grants
            .iter()
            .any(|g| g.tool_id.0 == "retrieve" && g.tool_version.0 == "1"));

        // The corpus was ingested ONCE, under the scoped name, carrying BOTH blobs.
        let scoped = app_dataset_scoped_name("scope-m1", "science", &corpus);
        let ingests = ds.ingests();
        assert_eq!(ingests.len(), 1, "exactly one ingest: {ingests:?}");
        assert_eq!(ingests[0].0, scoped);
        assert_eq!(ingests[0].1.len(), 2, "both carried blobs");
        assert!(ingests[0].1.contains(&CORPUS_DOC.to_vec()));
        assert!(
            scoped.starts_with("science.app-"),
            "readable-first: {scoped}"
        );

        // The steer names the PHYSICAL dataset the model must hand to `retrieve` — a name
        // it cannot reach is a silently UNGROUNDED answer (retrieve@1 fails soft).
        let prompt = fold_and_steer(&host, &science(&corpus)).await.unwrap();
        assert!(prompt.contains(&scoped), "steers {scoped}: {prompt}");
    }

    /// The name is keyed on the DEDUPED corpus set, so the ingest must be too. A repeated
    /// ref is one doc in the name; if it were N docs in the ingest we would re-pay the embed
    /// cost N times (the host embeds BEFORE its content-addressed dedup) and count it N
    /// times against the ceilings — while landing the identical index either way.
    #[tokio::test]
    async fn a_repeated_cas_ref_is_ingested_once_matching_the_scoped_name() {
        let dir = tempfile::tempdir().unwrap();
        let ds = Arc::new(FakeDatasets::embedding(&[]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));
        let r = store.put(CORPUS_DOC).unwrap().to_hex();

        // The same blob named three times, plus a distinct one.
        let other = store.put(b"Iron melts at 1538C.").unwrap().to_hex();
        let dupes = vec![r.clone(), other.clone(), r.clone(), r];
        let prompt = fold_and_steer(&host, &science(&dupes)).await.unwrap();

        let ingests = ds.ingests();
        assert_eq!(ingests.len(), 1);
        assert_eq!(ingests[0].1.len(), 2, "two DISTINCT docs, not four");
        // ...and the ingested name is the one derived from the deduped set.
        assert_eq!(
            ingests[0].0,
            app_dataset_scoped_name("scope-m1", "science", &dupes)
        );
        assert!(prompt.contains(&ingests[0].0), "{prompt}");
        let _ = other;
    }

    /// The steady state: once the scoped dataset exists a run resolves to it and ingests
    /// NOTHING. The host embeds BEFORE its content-addressed dedup, so a blind re-ingest
    /// would re-pay the whole embed cost on every run, not just the first.
    #[tokio::test]
    async fn an_already_materialized_app_corpus_is_never_re_ingested() {
        let dir = tempfile::tempdir().unwrap();
        let scoped = app_dataset_scoped_name("scope-m1", "science", &[content_hex()]);
        let ds = Arc::new(FakeDatasets::embedding(&[scoped.as_str()]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));
        let put = store.put(CORPUS_DOC).unwrap().to_hex();
        assert_eq!(
            put,
            content_hex(),
            "the corpus ref the scoped name was built from"
        );

        let prompt = fold_and_steer(&host, &science(&[put])).await.unwrap();
        assert!(ds.ingests().is_empty(), "resolved to the existing index");
        assert!(prompt.contains(&scoped), "steers {scoped}: {prompt}");
    }

    /// The LEGITIMATE no-`--with-data` state: the envelope still serializes `cas_refs`,
    /// but the blobs never travelled. The App falls back to the DECLARED name — exactly
    /// the plain reference-existing behavior — rather than inventing an empty index.
    #[tokio::test]
    async fn an_app_corpus_whose_blobs_did_not_travel_falls_back_to_the_declared_name() {
        let dir = tempfile::tempdir().unwrap();
        let ds = Arc::new(FakeDatasets::embedding(&["science"]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, _store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));
        // A well-formed ref that is NOT in the content store.
        let prompt = fold_and_steer(&host, &science(&["ab".repeat(32)]))
            .await
            .unwrap();

        assert!(ds.ingests().is_empty(), "nothing to ingest");
        assert!(prompt.contains("[science]"), "declared name: {prompt}");
        assert!(!prompt.contains(".app-"), "no scoped name: {prompt}");
    }

    /// The negative twin: blobs absent AND no local dataset of that name ⇒ today's loud,
    /// actionable mis-authoring refusal is preserved (a carried corpus does not soften it).
    #[tokio::test]
    async fn an_app_corpus_that_did_not_travel_still_fails_closed_with_no_local_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let ds: Arc<dyn DatasetView> = Arc::new(FakeDatasets::embedding(&[]));
        let (host, _store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(ds));
        let handle = grounded_app(&host, vec!["ab".repeat(32)]);
        match host.author_app("alice@acme", &handle, b"", false).await {
            Err(AppRunError::InvalidArgs(msg)) => {
                assert!(msg.contains("science"), "{msg}");
                assert!(msg.contains("kx datasets ingest"), "actionable: {msg}");
            }
            Ok(_) => panic!("expected fail-closed on a corpus that did not travel"),
            Err(other) => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    /// FAIL-SOFT: a recoverable ingest failure (here: no embedder wired) falls back to
    /// reference-existing. Never brick a run over a corpus we could not materialize —
    /// the declared dataset may well be there.
    #[tokio::test]
    async fn a_recoverable_corpus_ingest_failure_falls_back_instead_of_bricking_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let ds: Arc<dyn DatasetView> = Arc::new(FakeDatasets::failing(&["science"], || {
            DatasetError::EmbedderUnavailable
        }));
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(ds));
        let corpus = vec![store.put(CORPUS_DOC).unwrap().to_hex()];

        let prompt = fold_and_steer(&host, &science(&corpus)).await.unwrap();
        assert!(prompt.contains("[science]"), "{prompt}");
    }

    /// HARD: a genuine backend failure is NOT papered over. Grounding on a store that
    /// cannot answer would be worse than refusing.
    #[tokio::test]
    async fn a_corpus_ingest_backend_failure_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let ds: Arc<dyn DatasetView> = Arc::new(FakeDatasets::failing(&["science"], || {
            DatasetError::Internal("poisoned".into())
        }));
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(ds));
        let corpus = vec![store.put(CORPUS_DOC).unwrap().to_hex()];

        match fold_and_steer(&host, &science(&corpus)).await {
            Err(AppRunError::Internal(msg)) => assert!(msg.contains("poisoned"), "{msg}"),
            other => panic!("expected a hard Internal, got {other:?}"),
        }
    }

    /// A server-embed needs TEXT, and `DatasetRef` carries no `media_type` to say
    /// otherwise — a binary corpus skips rather than erroring out of the run.
    #[tokio::test]
    async fn a_non_utf8_app_corpus_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let ds = Arc::new(FakeDatasets::embedding(&["science"]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));
        let corpus = vec![store.put(&[0xff, 0xfe, 0x00]).unwrap().to_hex()];

        let prompt = fold_and_steer(&host, &science(&corpus)).await.unwrap();
        assert!(ds.ingests().is_empty(), "binary corpus not ingested");
        assert!(prompt.contains("[science]"), "{prompt}");
    }

    /// The DoS ceiling: the CLI's bundle bounds are client-side, so a hand-rolled envelope
    /// can name thousands of refs. Over-ceiling skips BEFORE any store read.
    #[tokio::test]
    async fn an_over_ceiling_app_corpus_is_skipped_before_reading_any_blob() {
        let dir = tempfile::tempdir().unwrap();
        let ds = Arc::new(FakeDatasets::embedding(&["science"]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, _store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));
        let corpus: Vec<String> = (0..=MAX_APP_CORPUS_REFS)
            .map(|i| format!("{i:064x}"))
            .collect();

        let prompt = fold_and_steer(&host, &science(&corpus)).await.unwrap();
        assert!(ds.ingests().is_empty(), "over-ceiling corpus not ingested");
        assert!(prompt.contains("[science]"), "{prompt}");
    }

    /// The BYTE ceiling — the ref ceiling's sibling, and the one this module's own doc calls
    /// "the bound that matters": every byte is chunked and synchronously EMBEDDED on first run,
    /// so an over-ceiling corpus costs model-TIME, not disk. `MAX_APP_CORPUS_REFS` cannot catch
    /// it — a handful of huge blobs sails far under 4096 refs — so this check is the only thing
    /// between a hand-rolled envelope and hours of embedding inside one run.
    ///
    /// It was UNTESTED until §2.395 while its ref sibling (above) and its UTF-8 sibling (below)
    /// both had tests. That is the dangerous shape: all three fail-soft to the declared name, so
    /// from the outside the three outcomes are indistinguishable — if this ceiling silently
    /// became a no-op, not one other test would fail.
    #[tokio::test]
    async fn an_over_byte_ceiling_app_corpus_is_skipped() {
        // The fixture size is PINNED, never derived from `MAX_APP_CORPUS_BYTES`. A test that
        // sizes its input from the constant under test is not a detector: raising the constant
        // also inflates the input, so the test dies ALLOCATING instead of failing its assertion
        // — red either way, proving nothing. (The first cut of this test did exactly that:
        // mutating the ceiling to `u64::MAX` panicked in `raw_vec`, not at the assert. Caught by
        // mutation-testing the test itself — §2.395.)
        //
        // Pinning makes it a TWO-WAY detector:
        //   • the ceiling VALUE moves      ⇒ the assert_eq below fires (a deliberate change must
        //                                    be a deliberate edit here, with the cost re-thought);
        //   • the ceiling CHECK disappears ⇒ the fixture self-ingests ⇒ `ingests()` is non-empty.
        const PINNED_CEILING: u64 = 64 * 1024 * 1024;
        assert_eq!(
            MAX_APP_CORPUS_BYTES, PINNED_CEILING,
            "the corpus byte ceiling moved — it bounds synchronous EMBEDDING time on first run, \
             so re-examine that cost, then update PINNED_CEILING deliberately"
        );

        let dir = tempfile::tempdir().unwrap();
        let ds = Arc::new(FakeDatasets::embedding(&["science"]));
        let view: Arc<dyn DatasetView> = ds.clone();
        let (host, store, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(view));

        // Each half is just over CEILING/2, so NEITHER blob alone trips the ceiling and the sum
        // does — pinning the check to the RUNNING TOTAL rather than to any single blob. The two
        // must DIFFER: the store is content-addressed, so identical bytes dedup to one ref and
        // the total would never reach the ceiling.
        // `try_from`, not `as`: an `as` cast silently truncates on a 32-bit target, turning
        // these into UNDER-ceiling blobs and inverting the test into a green that proves the
        // opposite. Fail loudly there instead.
        let half = usize::try_from(PINNED_CEILING / 2)
            .expect("half the corpus ceiling must fit in usize on this target")
            + 1;
        let corpus = vec![
            store.put(&vec![b'a'; half]).unwrap().to_hex(),
            store.put(&vec![b'b'; half]).unwrap().to_hex(),
        ];

        let prompt = fold_and_steer(&host, &science(&corpus)).await.unwrap();
        assert!(
            ds.ingests().is_empty(),
            "an over-byte-ceiling corpus must NOT self-ingest"
        );
        // Fail-soft, exactly like the ref/UTF-8 ceilings: the App still grounds on the declared
        // dataset NAME rather than refusing the run.
        assert!(prompt.contains("[science]"), "{prompt}");
    }

    /// The bindings are a pure function of the envelope: declaration-order dedup, the
    /// steering union, empties skipped — and FIRST declaration wins, so a bare re-mention
    /// of a name cannot displace the corpus-bearing entry that named it first.
    #[test]
    fn collect_dataset_bindings_dedups_in_declaration_order_and_keeps_the_corpus() {
        let mut env = AppEnvelope::new("a", serde_json::json!({ "steps": [] }));
        let corpus = vec![content_hex()];
        for (name, refs) in [
            ("science", corpus.clone()),
            ("", corpus.clone()),    // empty ⇒ skipped
            ("science", Vec::new()), // dup ⇒ the corpus-bearing FIRST entry wins
        ] {
            env.references.datasets.push(kx_app::DatasetRef {
                dataset_ref: name.into(),
                cas_refs: refs,
            });
        }
        env.steering_config.context.dataset_refs =
            vec!["history".into(), "science".into(), String::new()];

        assert_eq!(
            collect_dataset_bindings(&env),
            vec![
                DatasetBinding {
                    declared: "science".into(),
                    cas_refs: corpus
                },
                DatasetBinding {
                    declared: "history".into(),
                    cas_refs: Vec::new()
                },
            ]
        );
    }

    /// FAIL-CLOSED: grounding on a dataset that is not ingested is a mis-authoring error
    /// (not a silently ungrounded run).
    #[tokio::test]
    async fn author_app_with_an_absent_dataset_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let ds: Arc<dyn DatasetView> = Arc::new(FakeDatasets::new(&["science"]));
        let (host, _c, _) = rig_ex(dir.path(), &[("retrieve", "1")], Some(ds));
        let mut env = AppEnvelope::new(
            "grounded",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "Answer." }] }),
        );
        env.references.datasets.push(kx_app::DatasetRef {
            dataset_ref: "does-not-exist".into(),
            cas_refs: vec![],
        });
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        match host.author_app("alice@acme", &handle, b"", false).await {
            Err(AppRunError::InvalidArgs(msg)) => {
                assert!(msg.contains("does-not-exist"), "{msg}");
                assert!(msg.contains("kx datasets ingest"), "actionable: {msg}");
            }
            Ok(_) => panic!("expected fail-closed on an absent dataset"),
            Err(other) => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    /// HONEST DEGRADE: with no retrieval seam on the build (datasets == None), a declared
    /// dataset produces an UNGROUNDED run (no retrieve fold), never a hard error.
    #[tokio::test]
    async fn author_app_with_a_dataset_but_no_view_degrades_ungrounded() {
        let dir = tempfile::tempdir().unwrap();
        let (host, _c, _) = rig_ex(dir.path(), &[], None); // no dataset view (non-hnsw)
        let mut env = AppEnvelope::new(
            "grounded",
            serde_json::json!({ "steps": [{ "kind": "model", "prompt": "Answer." }] }),
        );
        env.references.datasets.push(kx_app::DatasetRef {
            dataset_ref: "science".into(),
            cas_refs: vec![],
        });
        env.validate().unwrap();
        let handle = save_app(&host, &env);
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();
        // No retrieve grant (ungrounded), but the run authors fine.
        assert!(
            !bound.motes[0]
                .0
                .def
                .tool_contract
                .contains_key(&kx_mote::ToolName("retrieve".into())),
            "no retrieval seam ⇒ no retrieve fold (ungrounded degrade)"
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
        let bound = host
            .author_app("alice@acme", &handle, b"", false)
            .await
            .unwrap();

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
