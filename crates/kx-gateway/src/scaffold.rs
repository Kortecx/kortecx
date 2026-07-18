//! POC-5a / POC-6 — the host App-scaffold orchestrator (the [`AppScaffolder`] impl).
//!
//! Owns the runtime: it spawns a background task that drives a per-file write loop
//! into a CoW-on-CAS branch. POC-6 (agentic creation): a SCHEDULED scaffold first
//! runs the `app-manifest-plan` recipe to plan a use-case-specific project file set
//! (persisted as `.kortecx/manifest.json`), then loops that DYNAMIC manifest through
//! the existing per-file write path; a hosted scaffold keeps its proven framework
//! template. For each file it binds + submits the `app-scaffold-write` recipe (a
//! single Pure greedy model step, the `react-edit` pattern) through the EXISTING
//! binder + submitter (the coordinator stays the sole journal writer — the frozen
//! loop is untouched), awaits the committed body by polling the read-only projection
//! ([`try_committed_body`]), fails closed on an empty body (GR15), and
//! `AdvanceBranch`-es the manifest.
//!
//! POC-6 live streaming: every write step is a normal model mote whose decode
//! publishes token deltas to the in-process broker (keyed by its mote id). The loop
//! captures the run `instance_id` (from `register_run`) + the terminal mote id into
//! the tracker so `GetScaffoldStatus` hands the browser the `(instance, mote)` pair
//! to subscribe to over WS `/tokens` — the file streams into Monaco as it is authored.
//!
//! Progress is observed from REAL signals: the durable branch manifest growing +
//! an advisory in-memory tracker. On a re-`start` (or after a restart) the loop
//! reads the COMMITTED manifest marker (a resume never re-plans — the committed plan
//! is the truth) and writes only the paths still absent. POC-5b: before every write
//! the loop re-checks the per-App lock, so a lock applied mid-scaffold halts cleanly.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kx_content::ContentRef;
use kx_gateway_core::{
    authoring_prompt, body_is_empty, derive_phase, hosted_template, split_done_pending,
    try_committed_body, AppScaffolder, BinderError, BranchManifest, BranchStore, ContentReader,
    GatewayError as CoreError, HostedFileSource, JournalReader, LockStore, RecipeBinder,
    RunSubmitter, ScaffoldPhase, ScaffoldStatus, ScaffoldStep, APP_MANIFEST_PLAN_RECIPE_HANDLE,
    APP_SCAFFOLD_WRITE_RECIPE_HANDLE, MANIFEST_MARKER_PATH, SKELETON,
};
use kx_mote::MoteId;

use crate::manifest::{decode_manifest, manifest_plan_directive, skeleton_manifest, ManifestFile};

/// Per-step await ceiling (comfortably exceeds the recipe warrant's 300s wall-clock,
/// so the warrant's own timeout fires first and we then report the step `Failed`).
const STEP_TIMEOUT: Duration = Duration::from_secs(320);
/// Projection re-fold poll interval while awaiting a write step's commit.
const POLL: Duration = Duration::from_millis(250);
/// POC-6: how many of the most-recent sibling files to carry as coherence context
/// into each write. Bounded so a large dynamic project's accumulated bodies never
/// overflow the model's per-decode batch (the `n_tokens_all <= n_batch` guard).
const SIBLING_CONTEXT_MAX: usize = 2;

/// Advisory in-memory progress for one branch's scaffold (the durable truth is the
/// CoW branch manifest). POC-6: also carries the live token-stream ids of the file
/// currently being authored, so `GetScaffoldStatus` can hand the browser the
/// `(instance, mote)` pair to subscribe to.
#[derive(Clone)]
struct Progress {
    phase: ScaffoldPhase,
    detail: String,
    /// The path being authored right now (streamed), if any.
    writing_path: Option<String>,
    /// The run instance streaming the writing file's tokens (WS ownership gate).
    writing_instance_id: Option<[u8; 16]>,
    /// The write mote whose decode streams the writing file (the broker key).
    writing_mote_id: Option<[u8; 32]>,
}

/// The host scaffold orchestrator. `Clone` (all-`Arc`) so `start` can hand a clone to
/// the spawned background task; the shared tracker keeps live phase/detail.
#[derive(Clone)]
pub(crate) struct HostScaffolder {
    binder: Arc<dyn RecipeBinder>,
    submitter: Arc<dyn RunSubmitter>,
    reader: Arc<dyn JournalReader>,
    content: Arc<dyn ContentReader>,
    branches: Arc<dyn BranchStore>,
    locks: Option<Arc<dyn LockStore>>,
    tracker: Arc<Mutex<HashMap<String, Progress>>>,
}

impl HostScaffolder {
    pub(crate) fn new(
        binder: Arc<dyn RecipeBinder>,
        submitter: Arc<dyn RunSubmitter>,
        reader: Arc<dyn JournalReader>,
        content: Arc<dyn ContentReader>,
        branches: Arc<dyn BranchStore>,
        locks: Option<Arc<dyn LockStore>>,
    ) -> Self {
        Self {
            binder,
            submitter,
            reader,
            content,
            branches,
            locks,
            tracker: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn set(&self, branch: &str, phase: ScaffoldPhase, detail: &str) {
        if let Ok(mut t) = self.tracker.lock() {
            t.insert(
                branch.to_string(),
                Progress {
                    phase,
                    detail: detail.to_string(),
                    writing_path: None,
                    writing_instance_id: None,
                    writing_mote_id: None,
                },
            );
        }
    }

    /// POC-6: record the file currently being authored + the live token-stream ids
    /// (the write/plan mote streams via the broker keyed by `mote_id`; the browser
    /// gates on `instance_id`). Called once the run + terminal mote are known, so
    /// `GetScaffoldStatus` can hand the browser the `(instance, mote)` pair.
    fn set_writing(
        &self,
        branch: &str,
        phase: ScaffoldPhase,
        path: &str,
        instance_id: [u8; 16],
        mote_id: [u8; 32],
    ) {
        if let Ok(mut t) = self.tracker.lock() {
            t.insert(
                branch.to_string(),
                Progress {
                    phase,
                    detail: path.to_string(),
                    writing_path: Some(path.to_string()),
                    writing_instance_id: Some(instance_id),
                    writing_mote_id: Some(mote_id),
                },
            );
        }
    }

    /// Poll the read-only projection until the terminal mote commits a non-empty
    /// body; return `(result_ref, body)`. Fail closed on a failed terminal / empty
    /// body / timeout. Used for both a scaffold write step (the caller advances the
    /// branch to `result_ref`) and the manifest planner (the caller decodes `body`).
    async fn await_terminal(&self, terminal: MoteId) -> Result<([u8; 32], Vec<u8>), CoreError> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            match try_committed_body(self.reader.as_ref(), self.content.as_ref(), terminal)? {
                ScaffoldStep::Ready { result_ref, body } => {
                    if body_is_empty(&body) {
                        return Err(CoreError::FailedPrecondition(
                            "scaffold model step produced an empty body (the branch was NOT advanced)",
                        ));
                    }
                    return Ok((result_ref, body));
                }
                ScaffoldStep::Failed => {
                    return Err(CoreError::FailedPrecondition(
                        "scaffold model step did not commit a body (the model step failed)",
                    ));
                }
                ScaffoldStep::Pending => {}
            }
            if Instant::now() >= deadline {
                return Err(CoreError::FailedPrecondition(
                    "scaffold model step timed out before committing a body",
                ));
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Drive the full scaffold to terminal, recording the outcome in the tracker.
    async fn run(self, principal: String, branch: String, goal: String) {
        match self.run_inner(&principal, &branch, &goal).await {
            Ok(()) => self.set(&branch, ScaffoldPhase::Done, ""),
            Err(e) => {
                let detail = format!("{e}");
                tracing::warn!(branch = %branch, error = %detail, "App scaffold failed");
                self.set(&branch, ScaffoldPhase::Failed, &detail);
            }
        }
    }

    async fn run_inner(&self, principal: &str, branch: &str, goal: &str) -> Result<(), CoreError> {
        // Ensure the project branch exists (idempotent on resume).
        self.branches.create(
            principal,
            branch,
            None,
            "Kortecx App project branch (agentically scaffolded, in-CAS)",
        )?;

        // POC-6: resolve the DYNAMIC file set — a committed marker pins a resume
        // (deterministic — never re-plan), a fresh run asks the model to plan a
        // use-case-specific project, and a planning failure / no model degrades to
        // the proven fixed skeleton.
        let files = self.resolve_manifest(principal, branch, goal).await;

        let mut prior: Vec<String> = Vec::new();
        for file in &files {
            // POC-5b: a lock applied before/during the scaffold halts it cleanly.
            if let Some(l) = self.locks.as_ref() {
                if l.is_locked(principal, branch)? {
                    return Err(CoreError::FailedPrecondition(
                        "branch is locked; the scaffold was halted",
                    ));
                }
            }
            // Resume: a path already in the manifest is kept as context and skipped.
            let manifest = self
                .branches
                .get(principal, branch)?
                .ok_or(CoreError::Internal(String::from(
                    "scaffold branch vanished mid-run",
                )))?;
            if let Some(it) = manifest.items.iter().find(|i| i.path == file.path) {
                prior.push(ContentRef::from_bytes(it.content_ref).to_hex());
                continue;
            }

            self.set(branch, ScaffoldPhase::Writing, &file.path);
            let body_ref = self
                .write_one(principal, branch, &file.path, &file.role, goal, &prior)
                .await?;
            self.branches
                .advance(principal, branch, &file.path, body_ref)?;
            prior.push(ContentRef::from_bytes(body_ref).to_hex());
        }
        Ok(())
    }

    /// POC-6: resolve the planned file set for a scheduled scaffold. A committed
    /// `.kortecx/manifest.json` marker pins the plan (a resume is DETERMINISTIC — a
    /// re-plan would draw a different set from a non-deterministic model); a fresh
    /// run asks the model to plan a use-case-specific project and persists the exact
    /// planned bytes as the marker; no served model / a decode failure degrades to
    /// the proven fixed skeleton. Never returns empty.
    async fn resolve_manifest(
        &self,
        principal: &str,
        branch: &str,
        goal: &str,
    ) -> Vec<ManifestFile> {
        // Resume: a committed marker is the truth (no re-plan).
        if let Ok(Some(m)) = self.branches.get(principal, branch) {
            if let Some(files) = self.read_planned_manifest(&m) {
                return files;
            }
        }
        // Fresh plan → persist the planner's committed bytes as the durable marker.
        match self.plan_manifest(principal, branch, goal).await {
            Ok((files, manifest_ref)) => {
                if let Err(e) =
                    self.branches
                        .advance(principal, branch, MANIFEST_MARKER_PATH, manifest_ref)
                {
                    tracing::warn!(branch = %branch, error = %e, "failed to persist scaffold manifest marker");
                }
                files
            }
            Err(e) => {
                tracing::info!(branch = %branch, error = %e, "manifest planning unavailable; falling back to the fixed skeleton");
                skeleton_manifest()
            }
        }
    }

    /// POC-6: run the `app-manifest-plan` recipe for `goal` and decode its committed
    /// answer into the dynamic project file set. Returns `(files, committed_ref)` so
    /// the caller can persist the exact planned bytes as the marker. The planner is a
    /// normal model mote — its decode streams live (surfaced via `set_writing` under
    /// the marker path, so the browser can watch the plan being written). Fails
    /// closed (the caller falls back to the fixed skeleton on any error).
    async fn plan_manifest(
        &self,
        principal: &str,
        branch: &str,
        goal: &str,
    ) -> Result<(Vec<ManifestFile>, [u8; 32]), CoreError> {
        let prompt = manifest_plan_directive(goal);
        let args = serde_json::to_vec(&serde_json::json!({ "prompt": prompt }))
            .map_err(|e| CoreError::Internal(format!("manifest-plan args: {e}")))?;
        let bound = self
            .binder
            .bind(principal, APP_MANIFEST_PLAN_RECIPE_HANDLE, &args, &[], &[])
            .await
            .map_err(|e| match e {
                BinderError::NotAuthorized => CoreError::FailedPrecondition(
                    "manifest-plan recipe not available (no served model on this serve)",
                ),
                BinderError::InvalidArgs(d) | BinderError::Internal(d) => CoreError::Internal(d),
            })?;
        let terminal = bound.terminal_mote_id;
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(|e| CoreError::Internal(format!("manifest-plan register_run: {e}")))?;
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, false)
                .await
                .map_err(|e| CoreError::Internal(format!("manifest-plan submit_mote: {e}")))?;
        }
        // Stream the planner's decode into the same live surface (the marker path,
        // so the browser highlights it as JSON — "watch it plan").
        self.set_writing(
            branch,
            ScaffoldPhase::Planning,
            MANIFEST_MARKER_PATH,
            instance_id,
            *terminal.as_bytes(),
        );
        let (result_ref, body) = self.await_terminal(terminal).await?;
        let files = decode_manifest(&body)
            .map_err(|e| CoreError::Internal(format!("manifest decode failed: {e}")))?;
        Ok((files, result_ref))
    }

    /// POC-6: read the committed dynamic manifest's file set from a branch manifest,
    /// if a `.kortecx/manifest.json` marker is present + decodes. `None` ⇒ no
    /// committed plan yet (the caller plans fresh / falls back to the skeleton).
    fn read_planned_manifest(&self, manifest: &BranchManifest) -> Option<Vec<ManifestFile>> {
        let it = manifest
            .items
            .iter()
            .find(|i| i.path == MANIFEST_MARKER_PATH)?;
        let bytes = self.content.get(&ContentRef::from_bytes(it.content_ref))?;
        decode_manifest(&bytes).ok()
    }

    /// Drive a HOSTED-app scaffold to terminal: author the framework template's
    /// model-authored files (the visible page + README) into the branch. The static
    /// config files are template-owned (the supervisor writes them to disk).
    async fn run_hosted(self, principal: String, branch: String, framework: String, goal: String) {
        match self
            .run_hosted_inner(&principal, &branch, &framework, &goal)
            .await
        {
            Ok(()) => self.set(&branch, ScaffoldPhase::Done, ""),
            Err(e) => {
                let detail = format!("{e}");
                tracing::warn!(branch = %branch, error = %detail, "hosted-app scaffold failed");
                self.set(&branch, ScaffoldPhase::Failed, &detail);
            }
        }
    }

    async fn run_hosted_inner(
        &self,
        principal: &str,
        branch: &str,
        framework: &str,
        goal: &str,
    ) -> Result<(), CoreError> {
        self.branches.create(
            principal,
            branch,
            None,
            "Kortecx hosted-app project branch (agentically authored, in-CAS)",
        )?;
        let mut prior: Vec<String> = Vec::new();
        for tf in hosted_template(framework) {
            // Only the model-authored files are scaffolded here (page + README); the
            // static config is written to disk by the supervisor from the template.
            let HostedFileSource::Authored { role, .. } = tf.source else {
                continue;
            };
            if let Some(l) = self.locks.as_ref() {
                if l.is_locked(principal, branch)? {
                    return Err(CoreError::FailedPrecondition(
                        "branch is locked; the scaffold was halted",
                    ));
                }
            }
            let manifest = self
                .branches
                .get(principal, branch)?
                .ok_or(CoreError::Internal(String::from(
                    "scaffold branch vanished mid-run",
                )))?;
            if let Some(it) = manifest.items.iter().find(|i| i.path == tf.path) {
                prior.push(ContentRef::from_bytes(it.content_ref).to_hex());
                continue;
            }
            self.set(branch, ScaffoldPhase::Writing, tf.path);
            // Reuse the per-file model write path directly (path + role).
            let body_ref = self
                .write_one(principal, branch, tf.path, role, goal, &prior)
                .await?;
            self.branches
                .advance(principal, branch, tf.path, body_ref)?;
            prior.push(ContentRef::from_bytes(body_ref).to_hex());
        }
        Ok(())
    }

    /// Author one project file: bind + submit the scaffold-write recipe, surface the
    /// live token-stream ids, await the terminal body. The warrant is SERVER-minted
    /// (SN-8); the prompt is DATA only. `path`/`role` come from either the fixed
    /// skeleton, the hosted template, or a dynamically-planned manifest file.
    async fn write_one(
        &self,
        principal: &str,
        branch: &str,
        path: &str,
        role: &str,
        goal: &str,
        prior: &[String],
    ) -> Result<[u8; 32], CoreError> {
        // Bound the sibling context to the most RECENT files. A dynamic project can
        // hold many files, and the write mote assembles every context ref's BODY into
        // the prompt — an unbounded accumulation overflows the model's per-decode batch
        // (`n_tokens_all <= n_batch`) on the later files of a large project. The most
        // recent siblings give the most coherence signal; older files are dropped.
        let ctx: Vec<String> = prior
            .iter()
            .rev()
            .take(SIBLING_CONTEXT_MAX)
            .rev()
            .cloned()
            .collect();
        let prompt = authoring_prompt(path, role, goal, !ctx.is_empty());
        let args = serde_json::to_vec(&serde_json::json!({ "prompt": prompt }))
            .map_err(|e| CoreError::Internal(format!("scaffold args: {e}")))?;
        let bound = self
            .binder
            .bind(
                principal,
                APP_SCAFFOLD_WRITE_RECIPE_HANDLE,
                &args,
                &[],
                &ctx,
            )
            .await
            .map_err(|e| match e {
                BinderError::NotAuthorized => CoreError::FailedPrecondition(
                    "scaffold recipe not available (no served model on this serve)",
                ),
                BinderError::InvalidArgs(d) | BinderError::Internal(d) => CoreError::Internal(d),
            })?;
        let terminal = bound.terminal_mote_id;
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(|e| CoreError::Internal(format!("scaffold register_run: {e}")))?;
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, false)
                .await
                .map_err(|e| CoreError::Internal(format!("scaffold submit_mote: {e}")))?;
        }
        // POC-6: surface the run instance + write mote so `GetScaffoldStatus` hands
        // the browser the `(instance, mote)` pair to stream this file's tokens.
        self.set_writing(
            branch,
            ScaffoldPhase::Writing,
            path,
            instance_id,
            *terminal.as_bytes(),
        );
        let (result_ref, _body) = self.await_terminal(terminal).await?;
        Ok(result_ref)
    }
}

impl AppScaffolder for HostScaffolder {
    fn start(&self, principal: &str, branch_handle: &str, goal: &str) -> Result<bool, CoreError> {
        // Resumed iff the branch already holds a committed plan marker (a dynamic
        // scaffold) OR ≥1 fixed-skeleton file (a legacy / fallback scaffold).
        let resumed = match self.branches.get(principal, branch_handle)? {
            Some(m) => {
                let skel: BTreeSet<&str> = SKELETON.iter().map(|f| f.path).collect();
                m.items
                    .iter()
                    .any(|it| it.path == MANIFEST_MARKER_PATH || skel.contains(it.path.as_str()))
            }
            None => false,
        };
        self.set(branch_handle, ScaffoldPhase::Planning, "");
        // Spawn the background loop and return immediately (the RPC is non-blocking).
        let driver = self.clone();
        let (p, b, g) = (
            principal.to_string(),
            branch_handle.to_string(),
            goal.to_string(),
        );
        tokio::spawn(async move {
            driver.run(p, b, g).await;
        });
        Ok(resumed)
    }

    fn start_hosted(
        &self,
        principal: &str,
        branch_handle: &str,
        envelope_json: &[u8],
        goal: &str,
    ) -> Result<bool, CoreError> {
        // Parse the framework from the opaque envelope (the host owns the kx-app types).
        let framework = kx_app::AppEnvelope::from_json_slice(envelope_json)
            .ok()
            .and_then(|e| e.hosted.map(|h| h.framework.as_str().to_string()))
            .unwrap_or_else(|| "auto".to_string());
        let framework = framework.as_str();
        // Resumed iff the branch already holds ≥1 authored file for this framework.
        let authored: BTreeSet<&str> = hosted_template(framework)
            .iter()
            .filter(|f| matches!(f.source, HostedFileSource::Authored { .. }))
            .map(|f| f.path)
            .collect();
        let resumed = match self.branches.get(principal, branch_handle)? {
            Some(m) => m.items.iter().any(|it| authored.contains(it.path.as_str())),
            None => false,
        };
        self.set(branch_handle, ScaffoldPhase::Planning, "");
        let driver = self.clone();
        let (p, b, f, g) = (
            principal.to_string(),
            branch_handle.to_string(),
            framework.to_string(),
            goal.to_string(),
        );
        tokio::spawn(async move {
            driver.run_hosted(p, b, f, g).await;
        });
        Ok(resumed)
    }

    fn status(&self, principal: &str, branch_handle: &str) -> Result<ScaffoldStatus, CoreError> {
        let manifest = self.branches.get(principal, branch_handle)?;
        let manifest_paths: BTreeSet<String> = manifest
            .as_ref()
            .map(|m| m.items.iter().map(|i| i.path.clone()).collect())
            .unwrap_or_default();
        // POC-6: the planned set is the committed DYNAMIC manifest (excludes its
        // marker), else the fixed skeleton (pre-plan / graceful fallback).
        let planned: Vec<String> = manifest
            .as_ref()
            .and_then(|m| self.read_planned_manifest(m))
            .map_or_else(
                || SKELETON.iter().map(|f| f.path.to_string()).collect(),
                |files| files.into_iter().map(|f| f.path).collect(),
            );
        let (files_done, files_pending) = split_done_pending(&planned, &manifest_paths);
        let (phase, detail, writing_path, writing_instance_id, writing_mote_id) = match self
            .tracker
            .lock()
            .ok()
            .and_then(|t| t.get(branch_handle).cloned())
        {
            Some(p) => (
                p.phase,
                p.detail,
                p.writing_path,
                p.writing_instance_id,
                p.writing_mote_id,
            ),
            None => (
                derive_phase(&files_done, &files_pending),
                String::new(),
                None,
                None,
                None,
            ),
        };
        Ok(ScaffoldStatus {
            phase,
            files_done,
            files_pending,
            detail,
            writing_path,
            writing_instance_id,
            writing_mote_id,
        })
    }
}
