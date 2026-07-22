//! D213 Experience lane — the hosted-app run/build/serve supervisor (host impl).
//!
//! Materializes a hosted (Experience) App's branch file tree to a working directory,
//! `npm install`s it (once, cached), and runs it on a loopback port as a supervised child.
//! The server is exposed DIRECTLY at `http://127.0.0.1:<port>/` — the console's Run button
//! opens it in a new browser tab; there is NO reverse proxy (a public URL / a proxy /
//! multi-tenant isolation are Cloud).
//!
//! TWO LANES, chosen by the App envelope's `HostedConfig.serve_mode`:
//!   • `dev` (the default) — `npm run dev`. Hot module reload; an in-CAS edit is live on
//!     the next materialize. This is what makes a hosted app an editable workspace.
//!   • `production` — `npm run build`, then the framework's preview/start server over the
//!     built output. What actually ships: minified, tree-shaken, no HMR.
//!
//! `rebuild` on `start` is ORTHOGONAL to that choice: it re-materializes, drops
//! `node_modules`, reinstalls and restarts the SAME lane. It is a clean restart, not a
//! production build — which is why the console labels it "Restart clean".
//!
//! **Why this is off-digest (D213).** The supervisor is a plain `tokio::process` child in
//! the host binary — it is NEVER a Mote, never routed through `kx-executor`, never
//! journaled, and touches only a runtime data dir. Nothing it does reaches the
//! coordinator / executor / journal fold, so the canonical projection digest (`7d22d4bd`)
//! cannot move. Same off-journal posture as `apps.db`, the cron ticker, and MCP stdio.
//!
//! Behind the `hosted-apps` cargo feature (adds `tokio/process`); absent, the four hosted
//! RPCs return `unimplemented`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_content::ContentRef;
use kx_gateway_core::{
    build_command_args, dev_command_args, hosted_entry_path, hosted_template, preview_command_args,
    AppCatalog, BranchStore, ContentReader, GatewayError, HostedAppSupervisor, HostedFileSource,
    HostedServeMode, HostedState, HostedStatus, MANIFEST_MARKER_PATH,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Cap on the retained log ring per hosted app (advisory tail).
const MAX_LOG_LINES: usize = 200;
/// How long to wait for a freshly-spawned dev server to accept a connection.
const READINESS_TIMEOUT: Duration = Duration::from_secs(120);
/// The poll cadence while waiting for readiness.
const READINESS_POLL: Duration = Duration::from_millis(250);
/// The `install_cmd` sentinel that SKIPS `npm install` (used by hermetic tests).
const SKIP_INSTALL: &str = "skip";

type AppKey = (String, String);

/// The mutable per-app state (behind an `Arc<Mutex<..>>` so the background lifecycle
/// task, the log readers, and the status/stop RPCs share one view).
struct RunningApp {
    state: HostedState,
    /// The supervised server child (`kill_on_drop`). Taken + killed on stop.
    child: Option<tokio::process::Child>,
    port: u16,
    framework: String,
    /// Which lane this app is being served on (echoed in the status so a client never
    /// has to infer it from the state sequence).
    serve_mode: HostedServeMode,
    detail: String,
    /// The install/dev-server log tail (its own lock so log writes never contend with
    /// child/state access).
    logs: Arc<Mutex<VecDeque<String>>>,
    /// Bumped on every (re)start so a superseded background task aborts its writes.
    generation: u64,
}

impl RunningApp {
    fn new(framework: String) -> Self {
        Self {
            state: HostedState::Stopped,
            child: None,
            port: 0,
            framework,
            serve_mode: HostedServeMode::default(),
            detail: String::new(),
            logs: Arc::new(Mutex::new(VecDeque::new())),
            generation: 0,
        }
    }

    fn snapshot(&self, handle: &str) -> HostedStatus {
        let url = if self.state == HostedState::Running && self.port != 0 {
            format!("http://127.0.0.1:{}/", self.port)
        } else {
            String::new()
        };
        let recent_logs = self
            .logs
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default();
        HostedStatus {
            handle: handle.to_string(),
            state: self.state,
            url,
            recent_logs,
            framework: self.framework.clone(),
            port: u32::from(self.port),
            detail: self.detail.clone(),
            serve_mode: self.serve_mode,
        }
    }
}

/// The resolved per-app launch config (read from the envelope + branch at `start`).
struct LaunchPlan {
    branch_handle: String,
    framework: String,
    install_cmd: String,
    dev_cmd: String,
    /// Which lane to serve on (from the envelope's `HostedConfig.serve_mode`).
    serve_mode: HostedServeMode,
    /// Advisory `npm run build` override; ignored in dev mode.
    build_cmd: String,
    workdir: PathBuf,
}

/// The hosted-app supervisor. Holds the catalog/branch/content seams + the working-dir
/// root + the live-app map.
pub(crate) struct HostedSupervisor {
    data_root: PathBuf,
    catalog: Arc<dyn AppCatalog>,
    branches: Arc<dyn BranchStore>,
    content: Arc<dyn ContentReader>,
    running: Mutex<HashMap<AppKey, Arc<Mutex<RunningApp>>>>,
}

impl HostedSupervisor {
    /// Create a supervisor rooted at `<data_root>/hosted/` (created lazily per app).
    pub(crate) fn new(
        data_root: &Path,
        catalog: Arc<dyn AppCatalog>,
        branches: Arc<dyn BranchStore>,
        content: Arc<dyn ContentReader>,
    ) -> Self {
        Self {
            data_root: data_root.join("hosted"),
            catalog,
            branches,
            content,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Read the saved envelope + derive the launch plan (fail-closed on a non-hosted app).
    fn plan(&self, principal: &str, handle: &str) -> Result<LaunchPlan, GatewayError> {
        let (_, envelope_json) = self
            .catalog
            .get(principal, handle)?
            .ok_or(GatewayError::NotFound("hosted app not found"))?;
        let env = kx_app::AppEnvelope::from_json_slice(&envelope_json)
            .map_err(|_| GatewayError::Internal("stored envelope is invalid".into()))?;
        if env.kind() != kx_app::AppKind::Experience {
            return Err(GatewayError::InvalidArgument(
                "not a hosted (experience) app",
            ));
        }
        let hosted = env
            .hosted
            .ok_or(GatewayError::InvalidArgument("hosted app has no config"))?;
        let branch_handle = if env.branch_handle.is_empty() {
            handle.to_string()
        } else {
            env.branch_handle.clone()
        };
        // The working dir is keyed by (principal, handle) so restarts reuse node_modules.
        let dir_key = ContentRef::of(format!("{principal}\0{handle}").as_bytes()).to_hex();
        Ok(LaunchPlan {
            branch_handle,
            framework: hosted.framework.as_str().to_string(),
            install_cmd: hosted.install_cmd,
            dev_cmd: hosted.dev_cmd,
            // Unrecognized / empty (every app authored before this field) ⇒ Dev, so no
            // existing app is silently promoted to a lane it never asked for.
            serve_mode: HostedServeMode::from_label(&hosted.serve_mode),
            build_cmd: hosted.build_cmd,
            workdir: self.data_root.join(dir_key),
        })
    }
}

impl HostedAppSupervisor for HostedSupervisor {
    fn start(
        &self,
        principal: &str,
        handle: &str,
        rebuild: bool,
    ) -> Result<HostedStatus, GatewayError> {
        let plan = self.plan(principal, handle)?;
        let key: AppKey = (principal.to_string(), handle.to_string());

        let ra =
            {
                let mut map = self
                    .running
                    .lock()
                    .map_err(|_| GatewayError::Internal("hosted map poisoned".into()))?;
                Arc::clone(map.entry(key.clone()).or_insert_with(|| {
                    Arc::new(Mutex::new(RunningApp::new(plan.framework.clone())))
                }))
            };

        // Idempotent: an app already progressing/running returns its status unless a
        // rebuild is requested (which restarts from scratch).
        let generation = {
            let mut app = ra
                .lock()
                .map_err(|_| GatewayError::Internal("hosted app poisoned".into()))?;
            let busy = matches!(
                app.state,
                HostedState::Materializing
                    | HostedState::Installing
                    | HostedState::Building
                    | HostedState::Starting
                    | HostedState::Running
            );
            if busy && !rebuild {
                return Ok(app.snapshot(handle));
            }
            // Restart: kill any prior child, bump the generation (superseding any prior
            // background task), and reset to Materializing.
            if let Some(mut child) = app.child.take() {
                let _ = child.start_kill();
            }
            app.generation = app.generation.wrapping_add(1);
            app.state = HostedState::Materializing;
            app.detail = String::new();
            app.port = 0;
            app.framework.clone_from(&plan.framework);
            app.serve_mode = plan.serve_mode;
            if let Ok(mut logs) = app.logs.lock() {
                logs.clear();
            }
            app.generation
        };

        // The lifecycle (materialize → install → spawn → readiness) runs in the
        // background; `start` returns immediately with the initial status.
        let ctx = LifecycleCtx {
            ra: Arc::clone(&ra),
            branches: Arc::clone(&self.branches),
            content: Arc::clone(&self.content),
            principal: principal.to_string(),
            plan,
            generation,
            rebuild,
        };
        tokio::spawn(run_lifecycle(ctx));

        let app = ra
            .lock()
            .map_err(|_| GatewayError::Internal("hosted app poisoned".into()))?;
        Ok(app.snapshot(handle))
    }

    fn stop(&self, principal: &str, handle: &str) -> Result<bool, GatewayError> {
        let ra = {
            let map = self
                .running
                .lock()
                .map_err(|_| GatewayError::Internal("hosted map poisoned".into()))?;
            map.get(&(principal.to_string(), handle.to_string()))
                .map(Arc::clone)
        };
        let Some(ra) = ra else { return Ok(false) };
        let mut app = ra
            .lock()
            .map_err(|_| GatewayError::Internal("hosted app poisoned".into()))?;
        let was_running = app.child.is_some()
            || matches!(
                app.state,
                HostedState::Running | HostedState::Starting | HostedState::Installing
            );
        if let Some(mut child) = app.child.take() {
            let _ = child.start_kill();
        }
        // Bump the generation so the background task (if any) aborts its next write.
        app.generation = app.generation.wrapping_add(1);
        app.state = HostedState::Stopped;
        app.port = 0;
        app.detail = "stopped".into();
        Ok(was_running)
    }

    fn status(&self, principal: &str, handle: &str) -> Result<HostedStatus, GatewayError> {
        let ra = {
            let map = self
                .running
                .lock()
                .map_err(|_| GatewayError::Internal("hosted map poisoned".into()))?;
            map.get(&(principal.to_string(), handle.to_string()))
                .map(Arc::clone)
        };
        match ra {
            Some(ra) => {
                let app = ra
                    .lock()
                    .map_err(|_| GatewayError::Internal("hosted app poisoned".into()))?;
                Ok(app.snapshot(handle))
            }
            None => Ok(HostedStatus {
                handle: handle.to_string(),
                state: HostedState::Stopped,
                ..Default::default()
            }),
        }
    }

    fn list(&self, principal: &str) -> Result<Vec<HostedStatus>, GatewayError> {
        let entries: Vec<(String, Arc<Mutex<RunningApp>>)> = {
            let map = self
                .running
                .lock()
                .map_err(|_| GatewayError::Internal("hosted map poisoned".into()))?;
            map.iter()
                .filter(|((p, _), _)| p == principal)
                .map(|((_, h), ra)| (h.clone(), Arc::clone(ra)))
                .collect()
        };
        let mut out = Vec::with_capacity(entries.len());
        for (handle, ra) in entries {
            if let Ok(app) = ra.lock() {
                out.push(app.snapshot(&handle));
            }
        }
        out.sort_by(|a, b| a.handle.cmp(&b.handle));
        Ok(out)
    }
}

/// Everything the background lifecycle task needs.
struct LifecycleCtx {
    ra: Arc<Mutex<RunningApp>>,
    branches: Arc<dyn BranchStore>,
    content: Arc<dyn ContentReader>,
    principal: String,
    plan: LaunchPlan,
    generation: u64,
    rebuild: bool,
}

/// Set the app state IFF this task's generation is still current. Returns `false` when a
/// newer start/stop has superseded this task (so it must abort).
fn advance(ctx: &LifecycleCtx, state: HostedState, detail: &str) -> bool {
    let Ok(mut app) = ctx.ra.lock() else {
        return false;
    };
    if app.generation != ctx.generation {
        return false;
    }
    app.state = state;
    if !detail.is_empty() {
        app.detail = detail.to_string();
    }
    true
}

fn log_line(logs: &Arc<Mutex<VecDeque<String>>>, line: String) {
    if let Ok(mut l) = logs.lock() {
        if l.len() >= MAX_LOG_LINES {
            l.pop_front();
        }
        l.push_back(line);
    }
}

async fn run_lifecycle(ctx: LifecycleCtx) {
    let logs = {
        let Ok(app) = ctx.ra.lock() else { return };
        Arc::clone(&app.logs)
    };

    let production = ctx.plan.serve_mode == HostedServeMode::Production;

    // 1) Materialize the branch file tree to disk. Always rewritten: the template base is
    //    small and idempotent, and `node_modules` is never touched, so a prior install
    //    survives.
    if !advance(&ctx, HostedState::Materializing, "") {
        return;
    }
    if let Err(e) = materialize(&ctx, &logs) {
        advance(&ctx, HostedState::Failed, &format!("materialize: {e}"));
        return;
    }

    // 2) npm install (unless the sentinel skips it or node_modules already exists).
    if !advance(&ctx, HostedState::Installing, "installing dependencies") {
        return;
    }
    if let Err(e) = install(&ctx, &logs).await {
        advance(&ctx, HostedState::Failed, &format!("install: {e}"));
        return;
    }

    // 2.5) Serve-time type-check backstop. With deps installed, run the project's own
    //      `tsc --noEmit`: a dynamically scaffolded project whose files disagree across the
    //      seam (an import of a symbol a sibling never exported; props a component never
    //      declared) compiles-and-throws — it mounts and dies with a blank page. The
    //      author-time sibling-API summaries prevent most of this; here we catch the residue
    //      and fail LOUDLY with the compiler's own message. Self-skips without a toolchain
    //      or tsconfig, and honors KX_HOSTED_TYPECHECK={off,warn}.
    if let Err(e) = type_check(&ctx, &logs).await {
        advance(&ctx, HostedState::Failed, &format!("type-check: {e}"));
        return;
    }

    // 3) PRODUCTION ONLY: `npm run build`. The dev lane never enters this state, which is
    //    what makes the state honest — a client showing "building…" on a dev start would
    //    be describing something that is not happening.
    if production {
        if !advance(&ctx, HostedState::Building, "building for production") {
            return;
        }
        if let Err(e) = build(&ctx, &logs).await {
            advance(&ctx, HostedState::Failed, &format!("build: {e}"));
            return;
        }
    }

    // 4) Allocate a loopback port + spawn the server as a supervised child — the dev
    //    server, or the framework's preview/start server over the built output.
    let starting = if production {
        "starting production server"
    } else {
        "starting dev server"
    };
    if !advance(&ctx, HostedState::Starting, starting) {
        return;
    }
    let port = match alloc_port() {
        Ok(p) => p,
        Err(e) => {
            advance(&ctx, HostedState::Failed, &format!("port alloc: {e}"));
            return;
        }
    };
    let child = match spawn_server(&ctx, port, &logs) {
        Ok(c) => c,
        Err(e) => {
            advance(&ctx, HostedState::Failed, &format!("spawn: {e}"));
            return;
        }
    };
    {
        // Store the child + port (respecting the generation guard).
        let Ok(mut app) = ctx.ra.lock() else { return };
        if app.generation != ctx.generation {
            // Superseded — kill the just-spawned child and bail.
            let mut child = child;
            let _ = child.start_kill();
            return;
        }
        app.child = Some(child);
        app.port = port;
    }

    // 5) Wait for the server to accept connections, then mark Running.
    match wait_ready(&ctx, port).await {
        Readiness::Ready => {
            advance(
                &ctx,
                HostedState::Running,
                if production {
                    "running (production build)"
                } else {
                    "running"
                },
            );
        }
        // The child is already gone — nothing to kill, and the exit status plus its own last
        // words are the only useful thing we can say.
        Readiness::Exited { status, tail } => {
            let detail = if tail.is_empty() {
                format!("server exited before becoming ready ({status})")
            } else {
                format!("server exited before becoming ready ({status}): {tail}")
            };
            advance(&ctx, HostedState::Failed, &detail);
        }
        Readiness::TimedOut => {
            advance(
                &ctx,
                HostedState::Failed,
                "server did not become ready in time",
            );
            if let Ok(mut app) = ctx.ra.lock() {
                if let Some(mut child) = app.child.take() {
                    let _ = child.start_kill();
                }
            }
        }
    }
}

fn materialize(ctx: &LifecycleCtx, logs: &Arc<Mutex<VecDeque<String>>>) -> Result<(), String> {
    std::fs::create_dir_all(&ctx.plan.workdir).map_err(|e| e.to_string())?;

    // 1) Write the framework template BASE — static files verbatim, authored files with
    //    their runnable default body. This guarantees a complete, servable project even if
    //    the branch is empty. Idempotent + cheap (small source files); `node_modules` is
    //    never touched, so a prior install survives.
    for tf in hosted_template(&ctx.plan.framework) {
        let body = match tf.source {
            HostedFileSource::Static(s) => s,
            HostedFileSource::Authored { default, .. } => default,
        };
        write_file(&ctx.plan.workdir, tf.path, body.as_bytes())?;
    }

    // 2) Overlay the branch manifest — the model-authored page + any planned source file wins
    //    over the template default. Config is TEMPLATE-owned: the framework's STATIC files
    //    (package.json, build config, entry, base CSS) always win over a branch that (against
    //    the planner's instructions) authored one, so a hosted project is GUARANTEED to
    //    install/build/serve regardless of what the model emitted. The reserved plan marker is
    //    metadata, never a project file.
    let static_owned: std::collections::BTreeSet<&str> = hosted_template(&ctx.plan.framework)
        .iter()
        .filter(|f| matches!(f.source, HostedFileSource::Static(_)))
        .map(|f| f.path)
        .collect();
    let manifest = ctx
        .branches
        .get(&ctx.principal, &ctx.plan.branch_handle)
        .map_err(|e| format!("read branch: {e:?}"))?;
    let mut overlaid = 0usize;
    let mut skipped = 0usize;
    // Does the branch carry the framework's entry component, and does it carry a PROJECT at
    // all? Both, because only their combination is a defect.
    //
    // Step 1 wrote the template's placeholder body for the entry path, and for an App with no
    // project that is the DESIGNED outcome — `FileSource::Authored` promises a hosted project
    // is "always valid + servable even model-free", and the placeholder says so in its own
    // words ("Edit src/App.tsx to build it out"). Refusing there would break a working lane to
    // prevent an honest page.
    //
    // The silent wrong answer is the PARTIAL project: a scaffold that ran, wrote eight files,
    // and dropped the entry — now the placeholder sits among the user's real components and
    // the App looks finished while rendering the framework splash. Nothing on any surface says
    // otherwise. That is what this refuses.
    let entry = hosted_entry_path(&ctx.plan.framework);
    let mut has_entry = false;
    if let Some(manifest) = manifest {
        for item in &manifest.items {
            if item.path == entry {
                has_entry = true;
            }
            // Confinement: reject any path that escapes the workdir (defense-in-depth; the
            // scaffold only ever writes fixed relative paths).
            if item
                .path
                .split(['/', '\\'])
                .any(|c| c == ".." || c.is_empty())
            {
                return Err(format!("unsafe manifest path {:?}", item.path));
            }
            if item.path == MANIFEST_MARKER_PATH || static_owned.contains(item.path.as_str()) {
                skipped += 1;
                continue;
            }
            let bytes = ctx
                .content
                .get(&ContentRef::from_bytes(item.content_ref))
                .ok_or_else(|| format!("missing blob for {}", item.path))?;
            write_file(&ctx.plan.workdir, &item.path, &bytes)?;
            overlaid += 1;
        }
    }
    // `overlaid` is exactly the model-authored project file count — the loop already skipped
    // the plan marker and the template-owned statics. So `overlaid > 0 && !has_entry` is the
    // PARTIAL project, and nothing else: a scaffold that never finished, a branch imported
    // without its whole tree, or an entry deleted from the IDE. A branch with no authored
    // files at all is an App with no project, which the template placeholder serves by design.
    if overlaid > 0 && !has_entry {
        return Err(format!(
            "the project has {overlaid} file(s) but no entry component ({entry}), so serving it \
             would show the {} starter page next to this App's own components. Re-scaffold the \
             App to author it.",
            ctx.plan.framework
        ));
    }
    log_line(
        logs,
        format!(
            "materialized template ({}) + {overlaid} branch file(s) ({skipped} template-owned/marker skipped)",
            ctx.plan.framework
        ),
    );
    Ok(())
}

/// Write `bytes` to `workdir/rel`, creating parent directories.
fn write_file(workdir: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let dest = workdir.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&dest, bytes).map_err(|e| e.to_string())
}

async fn install(ctx: &LifecycleCtx, logs: &Arc<Mutex<VecDeque<String>>>) -> Result<(), String> {
    if ctx.plan.install_cmd == SKIP_INSTALL {
        return Ok(());
    }
    let node_modules = ctx.plan.workdir.join("node_modules");
    // A rebuild forces a clean reinstall (drop the cached node_modules first).
    if ctx.rebuild && node_modules.is_dir() {
        let _ = std::fs::remove_dir_all(&node_modules);
    }
    // Cache: skip when node_modules is already present (deps rarely change between runs).
    if node_modules.is_dir() {
        log_line(logs, "install: node_modules present, skipping".into());
        return Ok(());
    }
    let (prog, args) = if ctx.plan.install_cmd.is_empty() {
        ("npm".to_string(), vec!["install".to_string()])
    } else {
        split_cmd(&ctx.plan.install_cmd)
    };
    run_capture(&prog, &args, &ctx.plan.workdir, logs).await
}

/// `npm run build` — the production lane only. A build is a one-shot command, so it
/// reuses `run_capture` (the install primitive): its output lands in the same log ring,
/// and a non-zero exit fails the lifecycle with the compiler's own message rather than a
/// server that starts and serves nothing.
async fn build(ctx: &LifecycleCtx, logs: &Arc<Mutex<VecDeque<String>>>) -> Result<(), String> {
    // The install sentinel doubles as the BUILD skip, so the hermetic e2e can exercise the
    // Building state without Node on the box.
    if ctx.plan.install_cmd == SKIP_INSTALL {
        log_line(logs, "build: skipped (hermetic)".into());
        return Ok(());
    }
    let (prog, args) = if ctx.plan.build_cmd.is_empty() {
        ("npm".to_string(), build_command_args(&ctx.plan.framework))
    } else {
        split_cmd(&ctx.plan.build_cmd)
    };
    run_capture(&prog, &args, &ctx.plan.workdir, logs).await
}

/// How many trailing `tsc` lines to fold into the `Failed` detail when the type-check gate
/// trips. Enough to name the offending file + symbol; short enough for a status string (the
/// full diagnostic is in the log ring either way).
const TYPECHECK_ERR_TAIL_LINES: usize = 6;

/// Serve-time TypeScript gate — the backstop to the author-time sibling-API summaries.
///
/// A dynamically scaffolded hosted project can drift across the file seam: a hook imports a
/// symbol a sibling never exported, or a parent passes flat props to a component that declared
/// one object. Both compile-and-throw: the App mounts and then dies with a blank page and no
/// explanation. [`kx_gateway_core::distill_module_api`] PREVENTS most of that at authoring time;
/// this runs the project's OWN `tsc --noEmit` after install and fails the serve LOUDLY with the
/// compiler's message for whatever slips through — an honest error beats a white screen.
///
/// Runs only when it can be trusted and meaningful: skipped when install was skipped (the
/// hermetic e2e ships no toolchain), when the project has no `tsconfig.json`, or when no local
/// `tsc` is installed. `KX_HOSTED_TYPECHECK=off` disables it; `=warn` logs a failure but serves
/// anyway (for a loose-but-runnable project a dev bundler would tolerate).
async fn type_check(ctx: &LifecycleCtx, logs: &Arc<Mutex<VecDeque<String>>>) -> Result<(), String> {
    let mode = std::env::var("KX_HOSTED_TYPECHECK").unwrap_or_default();
    if mode == "off" || ctx.plan.install_cmd == SKIP_INSTALL {
        return Ok(());
    }
    if !ctx.plan.workdir.join("tsconfig.json").is_file() {
        return Ok(()); // not a TypeScript project — nothing to check
    }
    let tsc = ctx.plan.workdir.join("node_modules").join(".bin").join("tsc");
    if !tsc.is_file() {
        log_line(logs, "type-check: no local tsc, skipping".into());
        return Ok(());
    }
    log_line(logs, "$ tsc --noEmit".into());
    let output = Command::new(&tsc)
        .arg("--noEmit")
        .current_dir(&ctx.plan.workdir)
        .output()
        .await
        .map_err(|e| format!("cannot run tsc: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let mut lines: Vec<String> = Vec::new();
    for line in stdout.lines().chain(stderr.lines()) {
        log_line(logs, line.to_string());
        lines.push(line.to_string());
    }
    if output.status.success() {
        return Ok(());
    }
    let tail = lines
        .iter()
        .rev()
        .take(TYPECHECK_ERR_TAIL_LINES)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ");
    let msg = format!("the project does not type-check ({}): {tail}", output.status);
    if mode == "warn" {
        log_line(logs, format!("type-check: FAILED (warn mode, serving anyway): {msg}"));
        Ok(())
    } else {
        Err(msg)
    }
}

/// Spawn the app's server: the dev server (HMR) or, in production mode, the framework's
/// preview/start server over the freshly built output.
fn spawn_server(
    ctx: &LifecycleCtx,
    port: u16,
    logs: &Arc<Mutex<VecDeque<String>>>,
) -> Result<tokio::process::Child, String> {
    let production = ctx.plan.serve_mode == HostedServeMode::Production;
    let (prog, args) = if ctx.plan.dev_cmd.is_empty() {
        let args = if production {
            preview_command_args(&ctx.plan.framework, port)
        } else {
            dev_command_args(&ctx.plan.framework, port)
        };
        ("npm".to_string(), args)
    } else {
        // A custom command gets the port appended as its final argument. It overrides BOTH
        // lanes: an operator who pinned a command owns what it serves.
        let (prog, mut args) = split_cmd(&ctx.plan.dev_cmd);
        args.push(port.to_string());
        (prog, args)
    };
    log_line(logs, format!("$ {prog} {}", args.join(" ")));
    let mut cmd = Command::new(&prog);
    cmd.args(&args)
        .current_dir(&ctx.plan.workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot spawn {prog:?}: {e}"))?;
    // Pump stdout + stderr into the log ring.
    if let Some(out) = child.stdout.take() {
        pump(out, Arc::clone(logs));
    }
    if let Some(err) = child.stderr.take() {
        pump(err, Arc::clone(logs));
    }
    Ok(child)
}

/// Spawn a task that streams a child pipe's lines into the log ring.
fn pump<R>(reader: R, logs: Arc<Mutex<VecDeque<String>>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log_line(&logs, line);
        }
    });
}

async fn run_capture(
    prog: &str,
    args: &[String],
    workdir: &Path,
    logs: &Arc<Mutex<VecDeque<String>>>,
) -> Result<(), String> {
    log_line(logs, format!("$ {prog} {}", args.join(" ")));
    let output = Command::new(prog)
        .args(args)
        .current_dir(workdir)
        .output()
        .await
        .map_err(|e| format!("cannot run {prog:?}: {e}"))?;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        log_line(logs, line.to_string());
    }
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        log_line(logs, line.to_string());
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{prog} exited with {}", output.status))
    }
}

/// Allocate a free loopback port by binding `:0` and reading the assigned port. There is
/// a small race (the port is free between drop + child bind) — acceptable for a local
/// single-user runtime; the child fails loudly (`strictPort`) if it loses the race.
fn alloc_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    Ok(port)
}

/// How the readiness wait ended.
enum Readiness {
    /// The port accepted a connection.
    Ready,
    /// The child process exited first — with its status and the tail of its own log.
    Exited { status: String, tail: String },
    /// Neither happened within [`READINESS_TIMEOUT`].
    TimedOut,
}

/// How many trailing log lines to quote when a child dies during startup. Enough to carry a
/// stack trace's punchline; short enough to sit in a status string.
const EXIT_LOG_TAIL_LINES: usize = 8;

/// Poll `127.0.0.1:<port>` until it accepts a connection, the child exits, or the timeout
/// elapses.
///
/// Watching only the port made a crash indistinguishable from a slow start: a dev server
/// that died in 200 ms (a bad config, a missing dependency, a port collision) left the App
/// reporting "starting dev server" for the full two minutes and then failed with
/// "did not become ready in time" — which names the one thing that did NOT happen. The child
/// handle is right there in `ra`; ask it.
async fn wait_ready(ctx: &LifecycleCtx, port: u16) -> Readiness {
    let deadline = tokio::time::Instant::now() + READINESS_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Readiness::Ready;
        }
        // Check the child AFTER the connect attempt: a server that bound the port and then
        // exited in the same tick should still be reported as the exit it was.
        if let Ok(mut app) = ctx.ra.lock() {
            if app.generation == ctx.generation {
                if let Some(child) = app.child.as_mut() {
                    if let Ok(Some(status)) = child.try_wait() {
                        // Reaped here, so drop the handle — `Stop` must not later try to kill
                        // a pid that no longer exists (or, worse, a reused one).
                        app.child = None;
                        return Readiness::Exited {
                            status: status.to_string(),
                            tail: log_tail(&app.logs, EXIT_LOG_TAIL_LINES),
                        };
                    }
                }
            }
        }
        tokio::time::sleep(READINESS_POLL).await;
    }
    Readiness::TimedOut
}

/// The last `n` lines of a captured log, joined for a one-line status detail.
fn log_tail(logs: &Arc<Mutex<VecDeque<String>>>, n: usize) -> String {
    let Ok(buf) = logs.lock() else {
        return String::new();
    };
    let start = buf.len().saturating_sub(n);
    buf.iter()
        .skip(start)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Whitespace-split a command string into (program, args). Simple by design — the
/// override is operator-authored, not untrusted input.
fn split_cmd(cmd: &str) -> (String, Vec<String>) {
    let mut it = cmd.split_whitespace().map(str::to_string);
    let prog = it.next().unwrap_or_default();
    (prog, it.collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::branches::BranchesDb;
    use kx_content::{ContentStore, InMemoryContentStore};

    /// A branch store + content store wired the way the supervisor sees them, plus a
    /// `LifecycleCtx` pointed at a scratch workdir.
    struct Fixture {
        ctx: LifecycleCtx,
        content: Arc<InMemoryContentStore>,
        branches: Arc<BranchesDb<InMemoryContentStore>>,
        _workdir: tempfile::TempDir,
    }

    fn fixture(framework: &str) -> Fixture {
        let dbdir = tempfile::tempdir().unwrap();
        let workdir = tempfile::tempdir().unwrap();
        let content = Arc::new(InMemoryContentStore::default());
        let branches =
            Arc::new(BranchesDb::open(dbdir.path(), Arc::clone(&content), None).unwrap());
        std::mem::forget(dbdir); // keep the sqlite file alive for the test
        branches
            .create("alice", "apps/local/x", None, "test branch")
            .unwrap();
        let ctx = LifecycleCtx {
            ra: Arc::new(Mutex::new(RunningApp::new(framework.to_string()))),
            branches: Arc::clone(&branches) as Arc<dyn BranchStore>,
            content: Arc::clone(&content) as Arc<dyn ContentReader>,
            principal: "alice".into(),
            plan: LaunchPlan {
                branch_handle: "apps/local/x".into(),
                framework: framework.to_string(),
                install_cmd: SKIP_INSTALL.into(),
                dev_cmd: String::new(),
                serve_mode: HostedServeMode::default(),
                build_cmd: String::new(),
                workdir: workdir.path().to_path_buf(),
            },
            generation: 1,
            rebuild: false,
        };
        Fixture {
            ctx,
            content,
            branches,
            _workdir: workdir,
        }
    }

    fn put_file(f: &Fixture, path: &str, body: &[u8]) {
        let r = f.content.put(body).unwrap();
        f.branches
            .advance("alice", "apps/local/x", path, r.0)
            .unwrap();
    }

    fn logs() -> Arc<Mutex<VecDeque<String>>> {
        Arc::new(Mutex::new(VecDeque::new()))
    }

    #[test]
    fn materialize_refuses_a_partial_project_missing_its_entry() {
        // The silent-wrong-answer this guard exists for: a scaffold RAN and wrote real
        // components but dropped the entry. Step 1 has already put the template's placeholder
        // App.tsx on disk, so without the check the App looks finished and renders the Vite
        // splash next to the user's own components.
        let f = fixture("vite_react");
        put_file(
            &f,
            "src/components/Card.tsx",
            b"export const Card = () => null;",
        );
        let err = materialize(&f.ctx, &logs()).unwrap_err();
        assert!(
            err.contains("src/App.tsx") && err.contains("starter page"),
            "the refusal must name the missing entry and why it matters: {err}"
        );
        // And it refuses BEFORE anything could serve.
        assert!(!f.ctx.plan.workdir.join("node_modules").exists());
    }

    #[test]
    fn materialize_still_serves_an_app_that_has_no_project_at_all() {
        // The line the guard must NOT cross. `FileSource::Authored` promises a hosted project
        // is "always valid + servable even model-free", and the placeholder says as much in
        // its own words — an App you never scaffolded is not a defect, and refusing it would
        // break a working lane to prevent an honest page. Pinned because the first version of
        // this guard DID refuse here, and two shipped lifecycle e2e tests caught it.
        let f = fixture("vite_react");
        materialize(&f.ctx, &logs()).expect("an App with no project still materializes");
        let entry = std::fs::read_to_string(f.ctx.plan.workdir.join("src/App.tsx")).unwrap();
        assert!(entry.contains("Your hosted app is live"));
    }

    #[test]
    fn materialize_serves_a_branch_that_holds_only_template_statics() {
        // A scaffold that advanced the template statics and then failed before authoring
        // anything is still "no project": the statics are template-owned, not the model's
        // work, so the placeholder remains the honest answer.
        let f = fixture("vite_react");
        put_file(&f, "package.json", b"{}");
        put_file(&f, "src/main.tsx", b"// template-owned");
        materialize(&f.ctx, &logs()).expect("template statics alone are not a partial project");
    }

    #[test]
    fn materialize_accepts_and_overlays_a_project_that_has_its_entry() {
        let f = fixture("vite_react");
        put_file(
            &f,
            "src/App.tsx",
            b"export default function App() { return null; }",
        );
        put_file(
            &f,
            "src/components/Card.tsx",
            b"export const Card = () => null;",
        );
        materialize(&f.ctx, &logs()).expect("a project with its entry materializes");
        // The branch body WINS over the template's placeholder for the entry...
        let entry = std::fs::read(f.ctx.plan.workdir.join("src/App.tsx")).unwrap();
        assert_eq!(entry, b"export default function App() { return null; }");
        // ...the planned sibling lands...
        assert!(f.ctx.plan.workdir.join("src/components/Card.tsx").exists());
        // ...and the TEMPLATE still owns the static build config.
        assert!(f.ctx.plan.workdir.join("package.json").exists());
        assert!(f.ctx.plan.workdir.join("src/main.tsx").exists());
    }

    #[test]
    fn materialize_checks_the_entry_of_the_framework_it_was_given() {
        // A Next project satisfies the guard with app/page.tsx, not src/App.tsx.
        let f = fixture("next_js");
        put_file(&f, "src/App.tsx", b"// wrong framework's entry");
        let err = materialize(&f.ctx, &logs()).unwrap_err();
        assert!(err.contains("app/page.tsx"), "{err}");

        let g = fixture("next_js");
        put_file(
            &g,
            "app/page.tsx",
            b"export default function Page() { return null; }",
        );
        materialize(&g.ctx, &logs()).expect("next entry satisfies the guard");
    }
}
