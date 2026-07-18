//! POC-5a — the host App-scaffold orchestrator (the [`AppScaffolder`] impl).
//!
//! Owns the runtime: it spawns a background task that drives the FIXED-skeleton write
//! loop into a CoW-on-CAS branch. For each skeleton file it binds + submits the
//! `app-scaffold-write` recipe (a single Pure greedy model step, the `react-edit`
//! pattern) through the EXISTING binder + submitter (the coordinator stays the sole
//! journal writer — the frozen loop is untouched), awaits the committed body by
//! polling the read-only projection ([`try_committed_body`]), fails closed on an
//! empty body (GR15), and `AdvanceBranch`-es the manifest.
//!
//! Progress is observed from REAL signals: the durable branch manifest growing +
//! an advisory in-memory tracker. On a re-`start` (or after a restart) the loop
//! writes only the skeleton paths still absent — the manifest is the resume truth.
//! POC-5b: before every write the loop re-checks the per-App lock, so a lock applied
//! mid-scaffold halts cleanly (no partial file).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kx_content::ContentRef;
use kx_gateway_core::{
    authoring_prompt, body_is_empty, derive_phase, hosted_template, split_done_pending,
    try_committed_body, AppScaffolder, BinderError, BranchStore, ContentReader,
    GatewayError as CoreError, HostedFileSource, JournalReader, LockStore, RecipeBinder,
    RunSubmitter, ScaffoldFile, ScaffoldPhase, ScaffoldStatus, ScaffoldStep,
    APP_SCAFFOLD_WRITE_RECIPE_HANDLE, SKELETON,
};
use kx_mote::MoteId;

/// Per-step await ceiling (comfortably exceeds the recipe warrant's 300s wall-clock,
/// so the warrant's own timeout fires first and we then report the step `Failed`).
const STEP_TIMEOUT: Duration = Duration::from_secs(320);
/// Projection re-fold poll interval while awaiting a write step's commit.
const POLL: Duration = Duration::from_millis(250);

/// Advisory in-memory progress for one branch's scaffold (the durable truth is the
/// CoW branch manifest).
#[derive(Clone)]
struct Progress {
    phase: ScaffoldPhase,
    detail: String,
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
                },
            );
        }
    }

    /// Poll the read-only projection until the write step's terminal mote commits a
    /// non-empty body; fail closed on a failed terminal / empty body / timeout.
    async fn await_body(&self, terminal: MoteId) -> Result<[u8; 32], CoreError> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            match try_committed_body(self.reader.as_ref(), self.content.as_ref(), terminal)? {
                ScaffoldStep::Ready { result_ref, body } => {
                    if body_is_empty(&body) {
                        return Err(CoreError::FailedPrecondition(
                            "scaffold write produced an empty file body (the branch was NOT advanced)",
                        ));
                    }
                    return Ok(result_ref);
                }
                ScaffoldStep::Failed => {
                    return Err(CoreError::FailedPrecondition(
                        "scaffold write step did not commit a body (the model step failed)",
                    ));
                }
                ScaffoldStep::Pending => {}
            }
            if Instant::now() >= deadline {
                return Err(CoreError::FailedPrecondition(
                    "scaffold write step timed out before committing a body",
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

        let mut prior: Vec<String> = Vec::new();
        for file in SKELETON {
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

            self.set(branch, ScaffoldPhase::Writing, file.path);
            let body_ref = self.write_one(principal, file, goal, &prior).await?;
            self.branches
                .advance(principal, branch, file.path, body_ref)?;
            prior.push(ContentRef::from_bytes(body_ref).to_hex());
        }
        Ok(())
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
            // Reuse the per-file model write path via a ScaffoldFile view.
            let file = ScaffoldFile {
                path: tf.path,
                role,
            };
            let body_ref = self.write_one(principal, &file, goal, &prior).await?;
            self.branches
                .advance(principal, branch, tf.path, body_ref)?;
            prior.push(ContentRef::from_bytes(body_ref).to_hex());
        }
        Ok(())
    }

    /// Author one skeleton file: bind + submit the scaffold-write recipe, await the
    /// terminal body. The warrant is SERVER-minted (SN-8); the prompt is DATA only.
    async fn write_one(
        &self,
        principal: &str,
        file: &ScaffoldFile,
        goal: &str,
        prior: &[String],
    ) -> Result<[u8; 32], CoreError> {
        let prompt = authoring_prompt(file, goal, !prior.is_empty());
        let args = serde_json::to_vec(&serde_json::json!({ "prompt": prompt }))
            .map_err(|e| CoreError::Internal(format!("scaffold args: {e}")))?;
        let bound = self
            .binder
            .bind(
                principal,
                APP_SCAFFOLD_WRITE_RECIPE_HANDLE,
                &args,
                &[],
                prior,
            )
            .await
            .map_err(|e| match e {
                BinderError::NotAuthorized => CoreError::FailedPrecondition(
                    "scaffold recipe not available (no served model on this serve)",
                ),
                BinderError::InvalidArgs(d) | BinderError::Internal(d) => CoreError::Internal(d),
            })?;
        let terminal = bound.terminal_mote_id;
        self.submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(|e| CoreError::Internal(format!("scaffold register_run: {e}")))?;
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, false)
                .await
                .map_err(|e| CoreError::Internal(format!("scaffold submit_mote: {e}")))?;
        }
        self.await_body(terminal).await
    }
}

impl AppScaffolder for HostScaffolder {
    fn start(&self, principal: &str, branch_handle: &str, goal: &str) -> Result<bool, CoreError> {
        // Resumed iff the branch already holds ≥1 skeleton file.
        let resumed = match self.branches.get(principal, branch_handle)? {
            Some(m) => {
                let skel: BTreeSet<&str> = SKELETON.iter().map(|f| f.path).collect();
                m.items.iter().any(|it| skel.contains(it.path.as_str()))
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
        let manifest_paths: BTreeSet<String> = self
            .branches
            .get(principal, branch_handle)?
            .map(|m| m.items.into_iter().map(|i| i.path).collect())
            .unwrap_or_default();
        let (files_done, files_pending) = split_done_pending(&manifest_paths);
        let (phase, detail) = match self
            .tracker
            .lock()
            .ok()
            .and_then(|t| t.get(branch_handle).cloned())
        {
            Some(p) => (p.phase, p.detail),
            None => (derive_phase(&files_done, &files_pending), String::new()),
        };
        Ok(ScaffoldStatus {
            phase,
            files_done,
            files_pending,
            detail,
        })
    }
}
