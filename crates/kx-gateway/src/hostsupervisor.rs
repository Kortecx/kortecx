//! D213 Experience lane — the hosted-app run/build/serve supervisor (host impl).
//!
//! Materializes a hosted (Experience) App's branch file tree to a working directory,
//! `npm install`s it (once, cached), and runs its dev server (`vite`/`next dev`) on a
//! loopback port as a supervised child. The dev server is exposed DIRECTLY at
//! `http://127.0.0.1:<port>/` — the console's Run button opens it in a new browser tab
//! (native HMR, no proxy). A single-user, local mechanism (a public URL / a reverse
//! proxy / multi-tenant isolation are Cloud).
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
    dev_command_args, hosted_template, AppCatalog, BranchStore, ContentReader, GatewayError,
    HostedAppSupervisor, HostedFileSource, HostedState, HostedStatus,
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
    /// The supervised dev-server child (`kill_on_drop`). Taken + killed on stop.
    child: Option<tokio::process::Child>,
    port: u16,
    framework: String,
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
        }
    }
}

/// The resolved per-app launch config (read from the envelope + branch at `start`).
struct LaunchPlan {
    branch_handle: String,
    framework: String,
    install_cmd: String,
    dev_cmd: String,
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

    // 1) Materialize the branch file tree to disk (skip when unchanged unless rebuild).
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

    // 3) Allocate a loopback port + spawn the dev server as a supervised child.
    if !advance(&ctx, HostedState::Starting, "starting dev server") {
        return;
    }
    let port = match alloc_port() {
        Ok(p) => p,
        Err(e) => {
            advance(&ctx, HostedState::Failed, &format!("port alloc: {e}"));
            return;
        }
    };
    let child = match spawn_dev(&ctx, port, &logs) {
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

    // 4) Wait for the dev server to accept connections, then mark Running.
    if wait_ready(port).await {
        advance(&ctx, HostedState::Running, "running");
    } else {
        advance(
            &ctx,
            HostedState::Failed,
            "dev server did not become ready in time",
        );
        if let Ok(mut app) = ctx.ra.lock() {
            if let Some(mut child) = app.child.take() {
                let _ = child.start_kill();
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

    // 2) Overlay the branch manifest — the model-authored page (and any edited file) wins
    //    over the template default.
    let manifest = ctx
        .branches
        .get(&ctx.principal, &ctx.plan.branch_handle)
        .map_err(|e| format!("read branch: {e:?}"))?;
    let mut overlaid = 0usize;
    if let Some(manifest) = manifest {
        for item in &manifest.items {
            // Confinement: reject any path that escapes the workdir (defense-in-depth; the
            // scaffold only ever writes fixed relative paths).
            if item
                .path
                .split(['/', '\\'])
                .any(|c| c == ".." || c.is_empty())
            {
                return Err(format!("unsafe manifest path {:?}", item.path));
            }
            let bytes = ctx
                .content
                .get(&ContentRef::from_bytes(item.content_ref))
                .ok_or_else(|| format!("missing blob for {}", item.path))?;
            write_file(&ctx.plan.workdir, &item.path, &bytes)?;
            overlaid += 1;
        }
    }
    log_line(
        logs,
        format!(
            "materialized template ({}) + {overlaid} branch file(s)",
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

fn spawn_dev(
    ctx: &LifecycleCtx,
    port: u16,
    logs: &Arc<Mutex<VecDeque<String>>>,
) -> Result<tokio::process::Child, String> {
    let (prog, args) = if ctx.plan.dev_cmd.is_empty() {
        (
            "npm".to_string(),
            dev_command_args(&ctx.plan.framework, port),
        )
    } else {
        // A custom dev command gets the port appended as its final argument.
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

/// Poll `127.0.0.1:<port>` until it accepts a connection or the timeout elapses.
async fn wait_ready(port: u16) -> bool {
    let deadline = tokio::time::Instant::now() + READINESS_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(READINESS_POLL).await;
    }
    false
}

/// Whitespace-split a command string into (program, args). Simple by design — the
/// override is operator-authored, not untrusted input.
fn split_cmd(cmd: &str) -> (String, Vec<String>) {
    let mut it = cmd.split_whitespace().map(str::to_string);
    let prog = it.next().unwrap_or_default();
    (prog, it.collect())
}
