//! POC-5a / POC-6 — the host App-scaffold orchestrator (the [`AppScaffolder`] impl).
//!
//! Owns the runtime: it spawns a background task that drives a per-file write loop
//! into a CoW-on-CAS branch. POC-6 (agentic creation): BOTH lanes plan their file set
//! with the `app-manifest-plan` recipe and persist it as `.kortecx/manifest.json`, then
//! loop that DYNAMIC manifest through the existing per-file write path. They differ in
//! what a plan MEANS:
//!
//!   • HOSTED (Experience) — a use-case-specific, framework-aware SEPARATED source tree.
//!     The static build config stays template-owned: the supervisor writes it to disk,
//!     and the scaffold content-addresses it into the branch so the IDE tree and the
//!     exported bundle describe the whole project.
//!   • SCHEDULED (functional/agentic) — the PRESERVED base `SKELETON` (README, app.json,
//!     prompts, rules, skills) PLUS use-case files planned for the goal (more skills,
//!     more rules, reference material). The planner may only ADD; the union is what is
//!     persisted as the marker, so the base files stay visible in the progress report.
//!
//! Each lane degrades to its own floor when no model is served — the template's authored
//! files for hosted, the bare base set for scheduled. For each file it binds +
//! submits the `app-scaffold-write` recipe (a
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
    authoring_prompt, body_is_empty, derive_phase, distill_module_api, hosted_authored_role,
    hosted_entry_path, hosted_template, split_done_pending, strip_code_fence, try_committed_body,
    AppScaffolder, BinderError, BranchManifest, BranchStore, ContentReader, ContentWriter,
    GatewayError as CoreError, HostedFileSource, JournalReader, LockStore, RecipeBinder,
    RunSubmitter, ScaffoldPhase, ScaffoldStatus, ScaffoldStep, APP_MANIFEST_PLAN_RECIPE_HANDLE,
    APP_SCAFFOLD_WRITE_RECIPE_HANDLE, MANIFEST_MARKER_PATH, SKELETON,
};
use kx_mote::MoteId;

use crate::manifest::{decode_manifest, encode_manifest, manifest_plan_directive, ManifestFile};

/// Per-step await ceiling (comfortably exceeds the recipe warrant's 300s wall-clock,
/// so the warrant's own timeout fires first and we then report the step `Failed`).
const STEP_TIMEOUT: Duration = Duration::from_secs(320);
/// Projection re-fold poll interval while awaiting a write step's commit.
const POLL: Duration = Duration::from_millis(250);
/// POC-6: how many of the most-recent sibling files to carry as coherence context
/// into each write. Bounded so a large dynamic project's accumulated bodies never
/// overflow the model's per-decode batch (the `n_tokens_all <= n_batch` guard).
const SIBLING_CONTEXT_MAX: usize = 2;
/// How many model-planned files the SCHEDULED lane may add on top of the fixed base
/// [`SKELETON`]. Deliberately far below `MAX_MANIFEST_FILES` (48): each file is a full
/// model step bounded by [`STEP_TIMEOUT`], so 48 extras is a multi-hour scaffold for a
/// lane that wrote exactly 5 files before. The planner is asked for 2-6; this is the
/// fail-closed ceiling if it ignores that.
const MAX_SCHEDULED_EXTRA_FILES: usize = 8;

/// POC-6: the HOSTED lane's graceful fallback when manifest planning is unavailable
/// (no served model / a decode failure) — the framework template's model-AUTHORED files
/// (the visible page + README) as a dynamic-manifest list. The static build config is
/// template-owned (written to disk by the supervisor), so this still yields a runnable
/// project, path-for-path with the framework template.
fn hosted_authored_fallback(framework: &str) -> Vec<ManifestFile> {
    hosted_template(framework)
        .iter()
        .filter_map(|f| match f.source {
            HostedFileSource::Authored { role, .. } => Some(ManifestFile {
                path: f.path.to_string(),
                role: role.to_string(),
            }),
            HostedFileSource::Static(_) => None,
        })
        .collect()
}

/// Guarantee the framework's ENTRY component is in the planned set, returning
/// `(files, injected)`.
///
/// The hosted planner is TOLD it must emit the entry (`manifest::framework_contract`), but
/// that is prompt text and `decode_manifest` never checked it. A plan that leaves it out
/// runs to completion, and `materialize` then writes the template's placeholder body for
/// that path — so the App serves the framework starter page under the user's own name, with
/// nothing on any surface reporting a problem.
///
/// Injected FIRST, because the entry is the file every sibling is written to fit: the write
/// loop carries the most recent siblings forward as coherence context, and the entry is the
/// one that names them. The template's own role text is reused so an injected file is
/// authored to exactly the contract a planned one would have been.
fn ensure_entry_planned(framework: &str, files: Vec<ManifestFile>) -> (Vec<ManifestFile>, bool) {
    let entry = hosted_entry_path(framework);
    if files.iter().any(|f| f.path == entry) {
        return (files, false);
    }
    let Some(role) = hosted_authored_role(framework, entry) else {
        // Unreachable for the three shipped templates (pinned by `entry_path_is_the_file_
        // the_static_entry_actually_imports`); degrade OPEN rather than refuse a scaffold.
        return (files, false);
    };
    let mut with_entry = Vec::with_capacity(files.len() + 1);
    with_entry.push(ManifestFile {
        path: entry.to_string(),
        role: role.to_string(),
    });
    with_entry.extend(files);
    (with_entry, true)
}

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
    /// The content-store WRITE seam (`PutContent`'s trait). The scaffold needs it for
    /// the two things it must content-address WITHOUT a model: the template's static
    /// config files (so the branch — and therefore the IDE tree — holds the whole
    /// project, not just the model-authored part) and a server-authored plan marker
    /// (so a fallback plan is as durable as a planned one). A separate seam rather
    /// than widening `content`, because `try_committed_body` takes `&dyn ContentReader`.
    writer: Arc<dyn ContentWriter>,
    branches: Arc<dyn BranchStore>,
    locks: Option<Arc<dyn LockStore>>,
    tracker: Arc<Mutex<HashMap<String, Progress>>>,
    /// Per-branch LANE fallback: which paths `status()` should report as planned
    /// BEFORE a `.kortecx/manifest.json` marker is committed.
    ///
    /// The marker is the durable truth, but it only lands once planning returns — and
    /// planning is a model step that can run for minutes. In that window `status()` used
    /// to fall through to its last-resort default, the SCHEDULED skeleton, for BOTH
    /// lanes: a hosted app displayed README/app.json/prompts/rules/skills — a tree that
    /// lane never writes — and then visibly swapped to the real one. That read as "the
    /// template was replaced by the project". Recording the lane at start makes the
    /// pre-marker report belong to the right lane, so the only change a user sees is a
    /// fallback plan being refined into a fuller one.
    ///
    /// Deliberately NOT stored in `Progress`: `set()` replaces that struct wholesale on
    /// every phase change, so a field there would be erased by the first transition.
    lane_fallback: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl HostScaffolder {
    pub(crate) fn new(
        binder: Arc<dyn RecipeBinder>,
        submitter: Arc<dyn RunSubmitter>,
        reader: Arc<dyn JournalReader>,
        content: Arc<dyn ContentReader>,
        writer: Arc<dyn ContentWriter>,
        branches: Arc<dyn BranchStore>,
        locks: Option<Arc<dyn LockStore>>,
    ) -> Self {
        Self {
            binder,
            submitter,
            reader,
            content,
            writer,
            branches,
            locks,
            tracker: Arc::new(Mutex::new(HashMap::new())),
            lane_fallback: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record which paths `status()` should report as planned for `branch` until a
    /// manifest marker is committed. See [`HostScaffolder::lane_fallback`].
    fn set_lane_fallback(&self, branch: &str, paths: Vec<String>) {
        if let Ok(mut m) = self.lane_fallback.lock() {
            m.insert(branch.to_string(), paths);
        }
    }

    /// Content-address `files` as a `.kortecx/manifest.json` marker and bind it into the
    /// branch. Advisory: a failure is logged, never fatal — the marker only affects what
    /// `status()` REPORTS, and the branch manifest remains the durable truth of what was
    /// actually written.
    fn persist_manifest_marker(&self, principal: &str, branch: &str, files: &[ManifestFile]) {
        let bytes = match encode_manifest(files) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(branch = %branch, error = %e, "scaffold manifest marker not encodable");
                return;
            }
        };
        match self.writer.put(&bytes) {
            Ok((r, _existed)) => {
                if let Err(e) = self
                    .branches
                    .advance(principal, branch, MANIFEST_MARKER_PATH, r)
                {
                    tracing::warn!(branch = %branch, error = %e, "failed to persist scaffold manifest marker");
                }
            }
            Err(e) => {
                tracing::warn!(branch = %branch, error = %e, "failed to store scaffold manifest marker");
            }
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

        // The planned set is the PRESERVED base skeleton plus whatever use-case files the
        // planner adds on top (see `resolve_manifest_scheduled`).
        let files = self
            .resolve_manifest_scheduled(principal, branch, goal)
            .await;
        let all_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let mut prior: Vec<(String, String)> = Vec::new();
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
                prior.push((
                    file.path.clone(),
                    ContentRef::from_bytes(it.content_ref).to_hex(),
                ));
                continue;
            }

            self.set(branch, ScaffoldPhase::Writing, &file.path);
            // `framework: None` keeps the AGENTIC authoring directive for every file,
            // including the planner's extras — they are markdown for an automation, not
            // source for a web project.
            let body_ref = self
                .write_one(
                    principal, branch, &file.path, &file.role, goal, None, &all_paths, &prior,
                )
                .await?;
            self.branches
                .advance(principal, branch, &file.path, body_ref)?;
            prior.push((
                file.path.clone(),
                ContentRef::from_bytes(body_ref).to_hex(),
            ));
        }
        Ok(())
    }

    /// Resolve the planned file set for a SCHEDULED (agentic) scaffold: the fixed base
    /// [`SKELETON`] **plus** model-planned use-case files.
    ///
    /// PRESERVE-AND-EXTEND, not replace. The five base files are inviolable — every
    /// downstream surface assumes `app.json` and `prompts/system.md` exist, and the
    /// `skeleton_paths_are_stable` unit test pins the constant — so the planner may only
    /// ADD. Extras that re-declare a base path are dropped here rather than trusted:
    /// `decode_manifest`'s uniqueness check is manifest-internal and cannot see `SKELETON`.
    ///
    /// The UNION is what gets persisted as the marker, and that is load-bearing:
    /// `split_done_pending` reports only the planned list, so a marker holding just the
    /// extras would make the five base files vanish from `files_done` in the console.
    ///
    /// Degrades to exactly the old behaviour — the bare skeleton — when no model is served
    /// or the plan does not decode. A scheduled app is never worse off than before.
    async fn resolve_manifest_scheduled(
        &self,
        principal: &str,
        branch: &str,
        goal: &str,
    ) -> Vec<ManifestFile> {
        let base: Vec<ManifestFile> = SKELETON
            .iter()
            .map(|f| ManifestFile {
                path: f.path.to_string(),
                role: f.role.to_string(),
            })
            .collect();
        // Resume: a committed marker is the truth (a re-plan would draw a different set
        // from a non-deterministic model and strand half-written files).
        if let Ok(Some(m)) = self.branches.get(principal, branch) {
            if let Some(files) = self.read_planned_manifest(&m) {
                return files;
            }
        }
        let extras = match self.plan_manifest(principal, branch, None, goal).await {
            Ok((files, _committed_ref)) => files,
            Err(e) => {
                tracing::info!(branch = %branch, error = %e, "agentic manifest planning unavailable; writing the base skeleton only");
                // Say so on the wire instead of leaving `detail` empty — a five-file app
                // is otherwise indistinguishable from a planner that chose to add nothing.
                self.set(
                    branch,
                    ScaffoldPhase::Planning,
                    "planning unavailable — writing the base set only",
                );
                Vec::new()
            }
        };
        let base_paths: BTreeSet<&str> = SKELETON.iter().map(|f| f.path).collect();
        let mut files = base;
        files.extend(
            extras
                .into_iter()
                .filter(|f| !base_paths.contains(f.path.as_str()))
                // G023: the scheduled lane is markdown-only (`AGENTIC_PLAN_SYSTEM` asks for
                // ".md only" but that was prompt text, unenforced). Drop a non-`.md` extra
                // rather than author it: only `.md` rides the run's project context rail
                // (`app_run::is_project_rail_path`), so a stray `.py`/`.json` would be a file
                // the user sees, the model never gets, and no surface can explain. Matched to
                // the rail EXACTLY (lowercase `md`) so "authored" ⟺ "reaches the model".
                .filter(|f| {
                    let is_md = matches!(f.path.rsplit('.').next(), Some("md"));
                    if !is_md {
                        tracing::info!(
                            branch = %branch, path = %f.path,
                            "dropping a non-.md scheduled extra (only markdown reaches the run context rail)"
                        );
                    }
                    is_md
                })
                // Bound the lane. MAX_MANIFEST_FILES (48) × the per-step model timeout is
                // a multi-hour worst case for a lane that wrote exactly 5 files before,
                // and the live scaffold witness has a wall-clock deadline.
                .take(MAX_SCHEDULED_EXTRA_FILES),
        );
        // The union — NOT just the extras — is the marker, so `status()` keeps reporting
        // the base files as planned.
        self.persist_manifest_marker(principal, branch, &files);
        files
    }

    /// POC-6: resolve the planned file set for a HOSTED (Experience) scaffold. A
    /// committed `.kortecx/manifest.json` marker pins the plan (a resume is DETERMINISTIC — a
    /// re-plan would draw a different set from a non-deterministic model); a fresh run asks the
    /// model to plan a use-case-specific, framework-aware SEPARATED source tree and persists the
    /// exact planned bytes as the marker; no served model / a decode failure degrades to the
    /// framework template's authored files (the visible page + README — still a runnable
    /// project, since the static config is template-owned). Never returns empty.
    async fn resolve_manifest_hosted(
        &self,
        principal: &str,
        branch: &str,
        framework: &str,
        goal: &str,
    ) -> Vec<ManifestFile> {
        // Resume: a committed marker is the truth (no re-plan).
        if let Ok(Some(m)) = self.branches.get(principal, branch) {
            if let Some(files) = self.read_planned_manifest(&m) {
                return files;
            }
        }
        // Fresh plan → persist the planner's committed bytes as the durable marker.
        match self
            .plan_manifest(principal, branch, Some(framework), goal)
            .await
        {
            Ok((files, manifest_ref)) => {
                // The framework contract TELLS the planner it must emit the entry component
                // (`framework_contract` in `manifest.rs`) — as prompt text, which nothing
                // checked. A plan that omits it still scaffolds "successfully", and then the
                // supervisor writes the template's own placeholder body for that path and
                // serves it: the starter page, under the user's App name, with no error
                // anywhere. Make the contract true instead of merely requested.
                let (files, injected) = ensure_entry_planned(framework, files);
                // Persist the set we will ACTUALLY write. The planner's committed bytes are
                // the right marker only when they needed no correction — if we injected the
                // entry and then stored the raw bytes, a resume would read back the plan
                // WITHOUT it and reintroduce the bug on the second run.
                if injected {
                    tracing::info!(
                        branch = %branch,
                        entry = %hosted_entry_path(framework),
                        "the plan omitted the framework entry component; injected it"
                    );
                    self.persist_manifest_marker(principal, branch, &files);
                } else if let Err(e) =
                    self.branches
                        .advance(principal, branch, MANIFEST_MARKER_PATH, manifest_ref)
                {
                    tracing::warn!(branch = %branch, error = %e, "failed to persist scaffold manifest marker");
                }
                files
            }
            Err(e) => {
                tracing::info!(branch = %branch, error = %e, "manifest planning unavailable; falling back to the framework template's authored files");
                let files = hosted_authored_fallback(framework);
                // Persist the FALLBACK as the marker too. Without this the branch holds no
                // marker, so `status()` falls through to its last-resort default — the
                // SCHEDULED lane's skeleton — and the console shows a hosted app a tree of
                // README/app.json/prompts/rules/skills that this lane will never write. The
                // marker is what makes the reported plan match the lane in every case, not
                // just the happy one.
                self.persist_manifest_marker(principal, branch, &files);
                files
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
        framework: Option<&str>,
        goal: &str,
    ) -> Result<(Vec<ManifestFile>, [u8; 32]), CoreError> {
        let prompt = manifest_plan_directive(goal, framework);
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
        // Bind the template's STATIC build config into the branch before anything else.
        //
        // These files (package.json, the bundler + tsconfig, the HTML entry, the app entry)
        // are template-owned: the supervisor writes them to disk on every materialize and
        // deliberately lets them win over a colliding model file, which is what guarantees a
        // runnable project. But they were only ever written to DISK — never content-addressed
        // — so the branch, and therefore the IDE file tree and the exported `.kxapp` bundle,
        // held only the model-authored half of the project. A user browsing their own hosted
        // app could not see the config that runs it.
        //
        // Advancing them here changes nothing about who wins at materialize (the supervisor
        // still rewrites them from the template, and now simply skips them as duplicates) —
        // it makes the branch a complete description of the project instead of a partial one.
        // No model, no journal append, no Mote: content store + branches.db only.
        self.advance_template_statics(principal, branch, framework)?;
        // Plan a use-case-specific, framework-aware SEPARATED source tree (persisted as the
        // `.kortecx/manifest.json` marker), then author each planned file streaming into Monaco.
        // The static build config is TEMPLATE-owned (the supervisor writes it to disk and
        // `materialize` keeps it authoritative), so the model owns only the source tree.
        let files = self
            .resolve_manifest_hosted(principal, branch, framework, goal)
            .await;
        let all_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        // Author the framework ENTRY last. The entry (`src/App.tsx`, `app/page.tsx`, …) is the
        // file that IMPORTS and MOUNTS every child, so authoring it after its children is what
        // lets each child's distilled export/prop API reach the entry's authoring prompt — a
        // parent that renders a child before the child's prop shape is known is the dominant
        // cross-file break we observed. Only the VISITATION order changes; `files` (and thus the
        // persisted `.kortecx/manifest.json` marker, and a resume) is untouched. Children keep
        // their planned order, so an earlier child's API still reaches a later one.
        let entry = hosted_entry_path(framework);
        let order: Vec<&ManifestFile> = files
            .iter()
            .filter(|f| f.path != entry)
            .chain(files.iter().filter(|f| f.path == entry))
            .collect();
        let mut prior: Vec<(String, String)> = Vec::new();
        for file in order {
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
            if let Some(it) = manifest.items.iter().find(|i| i.path == file.path) {
                prior.push((
                    file.path.clone(),
                    ContentRef::from_bytes(it.content_ref).to_hex(),
                ));
                continue;
            }
            self.set(branch, ScaffoldPhase::Writing, &file.path);
            let body_ref = self
                .write_one(
                    principal,
                    branch,
                    &file.path,
                    &file.role,
                    goal,
                    Some(framework),
                    &all_paths,
                    &prior,
                )
                .await?;
            self.branches
                .advance(principal, branch, &file.path, body_ref)?;
            prior.push((
                file.path.clone(),
                ContentRef::from_bytes(body_ref).to_hex(),
            ));
        }
        Ok(())
    }

    /// Content-address the framework template's STATIC files and bind them into the branch.
    ///
    /// Idempotent on resume (a path already in the manifest is skipped) and lock-aware on
    /// every iteration, mirroring the authoring loop: a lock applied mid-scaffold halts this
    /// too, so POC-5b's "a locked branch takes no further writes" holds for the whole run and
    /// not just the model-authored part.
    fn advance_template_statics(
        &self,
        principal: &str,
        branch: &str,
        framework: &str,
    ) -> Result<(), CoreError> {
        for tf in hosted_template(framework) {
            let HostedFileSource::Static(body) = tf.source else {
                continue; // authored files are the write loop's job
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
            if manifest.items.iter().any(|i| i.path == tf.path) {
                continue;
            }
            let (r, _existed) = self.writer.put(body.as_bytes())?;
            self.branches.advance(principal, branch, tf.path, r)?;
        }
        Ok(())
    }

    /// Author one project file: bind + submit the scaffold-write recipe, surface the
    /// live token-stream ids, await the terminal body. The warrant is SERVER-minted
    /// (SN-8); the prompt is DATA only. `path`/`role` come from either the fixed
    /// skeleton, the hosted template, or a dynamically-planned manifest file.
    #[allow(clippy::too_many_arguments)]
    async fn write_one(
        &self,
        principal: &str,
        branch: &str,
        path: &str,
        role: &str,
        goal: &str,
        framework: Option<&str>,
        all_paths: &[&str],
        prior: &[(String, String)],
    ) -> Result<[u8; 32], CoreError> {
        // Bound the sibling BODY context to the most RECENT files. A dynamic project can
        // hold many files, and the write mote assembles every context ref's BODY into
        // the prompt — an unbounded accumulation overflows the model's per-decode batch
        // (`n_tokens_all <= n_batch`) on the later files of a large project. The most
        // recent siblings give the most coherence signal; older files are dropped.
        let ctx: Vec<String> = prior
            .iter()
            .rev()
            .take(SIBLING_CONTEXT_MAX)
            .rev()
            .map(|(_, r)| r.clone())
            .collect();
        // POC-6 coherence: distill EVERY prior sibling's export/prop API. The bounded body
        // window above carries only the two most-recent BODIES (a batch-size guard); the path
        // list is prompt text that cannot convey an export list or a prop interface. A model
        // that sees a sibling's PATH but not its API imports a symbol it never exported or
        // passes flat props to a component that declared one object — the App mounts and then
        // throws. Each summary is tiny, so all of them fit regardless of authoring order; the
        // read is a local content-store hit and the scheduled lane's markdown yields `None`.
        let sibling_apis: Vec<(String, String)> = prior
            .iter()
            .filter_map(|(p, r)| {
                let cref = ContentRef::from_hex(r)?;
                let body = self.content.get(&cref)?;
                distill_module_api(p, &body).map(|api| (p.clone(), api))
            })
            .collect();
        let prompt = authoring_prompt(
            path,
            role,
            goal,
            framework,
            all_paths,
            !ctx.is_empty(),
            &sibling_apis,
        );
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
        let (result_ref, body) = self.await_terminal(terminal).await?;
        // The authoring prompt asks for "no markdown code fences" and that request has
        // always been the only enforcement. A model that ignores it puts ```tsx on line 1
        // of a source file, which the bundler then reports as a syntax error in a file the
        // user never wrote. Unwrap it here — ONE place, because both lanes author through
        // this function — rather than in each scaffolder.
        //
        // The committed body is already in CAS under `result_ref`, so a body that needed no
        // change keeps that exact ref (`strip_code_fence` returns a subslice); only a body
        // we actually altered is re-`put`. Re-run the empty-body guard on the stripped
        // bytes: a body that was NOTHING but a fence strips to empty, and that must refuse
        // the write rather than advance the branch to a blank file.
        let stripped = strip_code_fence(&body);
        if stripped.len() == body.len() {
            return Ok(result_ref);
        }
        if body_is_empty(stripped) {
            return Err(CoreError::FailedPrecondition(
                "scaffold model step produced only a code fence (the branch was NOT advanced)",
            ));
        }
        tracing::debug!(branch = %branch, path = %path, "stripped a wrapping code fence from an authored body");
        let (fenced_ref, _existed) = self.writer.put(stripped)?;
        Ok(fenced_ref)
    }
}

impl AppScaffolder for HostScaffolder {
    fn start(&self, principal: &str, branch_handle: &str, goal: &str) -> Result<bool, CoreError> {
        // Resumed iff the branch already holds the committed plan marker or ≥1 base file.
        // The marker now lands on this lane too (the planned set is the base skeleton plus
        // the planner's extras), and it is written BEFORE the write loop — so checking it
        // is what makes a resume that got as far as planning read as resumed.
        let resumed = match self.branches.get(principal, branch_handle)? {
            Some(m) => {
                let skel: BTreeSet<&str> = SKELETON.iter().map(|f| f.path).collect();
                m.items
                    .iter()
                    .any(|it| it.path == MANIFEST_MARKER_PATH || skel.contains(it.path.as_str()))
            }
            None => false,
        };
        // Until the marker lands, report the base skeleton — this lane's floor, and what it
        // will write even if planning is unavailable.
        self.set_lane_fallback(
            branch_handle,
            SKELETON.iter().map(|f| f.path.to_string()).collect(),
        );
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
        // Resumed iff the branch already holds the committed plan marker — the planner persists
        // it before the write loop, so any authored source file implies the marker exists.
        let resumed = match self.branches.get(principal, branch_handle)? {
            Some(m) => m.items.iter().any(|it| it.path == MANIFEST_MARKER_PATH),
            None => false,
        };
        // Report THIS lane's paths until the plan marker lands (see `lane_fallback`):
        // the template's authored files plus its statics, which the write loop below
        // content-addresses into the branch.
        self.set_lane_fallback(
            branch_handle,
            hosted_template(framework)
                .iter()
                .map(|f| f.path.to_string())
                .collect(),
        );
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
        // The planned set is the committed DYNAMIC manifest marker when one exists (it is
        // the durable truth, and a resume must report exactly what it will finish). Before
        // the marker lands, report the LANE's own fallback — hosted apps get the framework
        // template, not the scheduled skeleton. The bare SKELETON survives only as the
        // last resort for a branch this process never started (e.g. after a serve restart).
        let planned: Vec<String> = manifest
            .as_ref()
            .and_then(|m| self.read_planned_manifest(m))
            .map(|files| files.into_iter().map(|f| f.path).collect())
            .or_else(|| {
                self.lane_fallback
                    .lock()
                    .ok()
                    .and_then(|m| m.get(branch_handle).cloned())
            })
            .unwrap_or_else(|| SKELETON.iter().map(|f| f.path.to_string()).collect());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str) -> ManifestFile {
        ManifestFile {
            path: path.to_string(),
            role: "a role".into(),
        }
    }

    #[test]
    fn ensure_entry_planned_injects_a_missing_entry_first() {
        // The planner IS told to emit src/App.tsx, but that is prompt text — a plan that
        // drops it used to scaffold "successfully" and then serve the starter page.
        let (files, injected) =
            ensure_entry_planned("vite_react", vec![f("src/components/Card.tsx")]);
        assert!(injected);
        assert_eq!(files[0].path, "src/App.tsx");
        assert_eq!(files.len(), 2);
        // The injected file carries the TEMPLATE's own role, so it is authored to exactly
        // the contract a planned entry would have been.
        assert_eq!(
            files[0].role,
            hosted_authored_role("vite_react", "src/App.tsx").unwrap()
        );
        // The planner's own files survive, in order.
        assert_eq!(files[1].path, "src/components/Card.tsx");
    }

    #[test]
    fn ensure_entry_planned_leaves_a_complete_plan_untouched() {
        // No injection ⇒ the caller keeps persisting the planner's committed BYTES as the
        // marker, so a resume replays exactly what the model said.
        let plan = vec![f("src/App.tsx"), f("src/components/Card.tsx")];
        let (files, injected) = ensure_entry_planned("vite_react", plan.clone());
        assert!(!injected);
        assert_eq!(files, plan);
    }

    #[test]
    fn ensure_entry_planned_injects_the_right_entry_per_framework() {
        for (fw, entry) in [
            ("next_js", "app/page.tsx"),
            ("svelte", "src/App.svelte"),
            ("auto", "src/App.tsx"),
        ] {
            let (files, injected) = ensure_entry_planned(fw, vec![f("src/other.ts")]);
            assert!(injected, "{fw}");
            assert_eq!(files[0].path, entry, "{fw}");
        }
        // A plan that already names the framework's OWN entry is complete, even though it
        // is not the Vite one.
        let (_, injected) = ensure_entry_planned("next_js", vec![f("app/page.tsx")]);
        assert!(!injected);
        // ...and naming a DIFFERENT framework's entry does not satisfy it.
        let (files, injected) = ensure_entry_planned("next_js", vec![f("src/App.tsx")]);
        assert!(injected);
        assert_eq!(files[0].path, "app/page.tsx");
    }
}
