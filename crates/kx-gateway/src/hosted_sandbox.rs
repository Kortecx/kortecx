//! The hosted-app SANDBOX seam (`hosted-apps` feature) — the ONE place a hosted
//! spawn gets its isolation treatment, by construction rather than convention:
//!
//! - **Env hygiene, always** (sandbox on or off): every hosted child —
//!   `npm install`, `npm run build`, `tsc`, the dev server — runs on a CLEARED
//!   environment with a minimal allowlist. The gateway's own environment can
//!   carry secrets, and a supervised npm tree inherited ALL of it before this.
//! - **Its own process group, always**: `npm run dev` forks the real server as a
//!   grandchild, which `start_kill` (direct-child SIGKILL) never reached — stop
//!   left the port's true owner running. Group-kill is the honest stop, and a
//!   prerequisite for honest sandboxing.
//! - **The platform sandbox for the DEV SERVER, where the platform can hold it**
//!   (macOS `sandbox-exec`, deny-default): RW confined to the app's workdir,
//!   exec confined to the node/npm roots, loopback only. `npm install` stays
//!   OUTSIDE the sandbox — it needs registry egress, which this platform's
//!   profile language cannot confine per-host, and granting the whole network
//!   would exceed anything the app declared (the `sandbox_probe` posture).
//!
//! The "pre-built base" this OSS lane ships is exactly this module's FIXED
//! profile template + pinned mount preset — deterministic and versioned in
//! source. A real rootfs/OCI image needs the (CI-frozen) executor spawn path and
//! stays cloud scope; nothing here pretends otherwise.
//!
//! FAIL-CLOSED posture (`KX_HOSTED_SANDBOX`):
//! - `on` ⇒ enforce; a platform that cannot confine REFUSES hosted start.
//! - unset ⇒ enforce where the probe says the platform can hold it; elsewhere
//!   run unconfined and SAY SO in the status detail + startup log — never claim
//!   confinement that is not enforced.
//! - `off` ⇒ run unconfined, loudly (the operator's explicit call).

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tokio::process::Command;

/// The env knob. Values: unset (probe-driven), `on` (fail-closed), `off` (loud).
pub(crate) const HOSTED_SANDBOX_ENV: &str = "KX_HOSTED_SANDBOX";
/// Colon-separated EXTRA exec roots (directories holding node/npm and friends)
/// granted read+exec inside the sandbox, for toolchains outside the defaults.
pub(crate) const HOSTED_EXEC_ROOTS_ENV: &str = "KX_HOSTED_EXEC_ROOTS";

/// The shell selector `/bin/sh` opens before it will run anything. Denying it
/// produces `Error opening /private/var/select/sh` and an unusable shell.
const SHELL_SELECTOR: &str = "/private/var/select";

/// How this serve isolates hosted children. Resolved ONCE at serve start (env +
/// probe), then a plain value — tests inject it directly, no env races.
#[derive(Clone, Debug)]
pub(crate) enum SandboxMode {
    /// Wrap the dev server in the platform sandbox (deny-default profile).
    Enforce,
    /// Run unconfined, with the honest reason (platform can't hold it / operator
    /// said off). Env hygiene + group-kill still apply.
    Off { reason: String },
    /// Hosted start REFUSES: the operator demanded a sandbox this platform
    /// cannot provide (`on` + probe refusal, or the sandbox binary is missing).
    Refuse { reason: String },
}

#[derive(Clone, Debug)]
pub(crate) struct SandboxPolicy {
    pub(crate) mode: SandboxMode,
    /// Directories granted read+exec (node/npm toolchain roots), canonicalized.
    exec_roots: Vec<PathBuf>,
}

impl SandboxPolicy {
    /// Resolve from the environment + the platform probe. `server.rs` calls this
    /// once and logs the outcome; everything downstream consumes the value.
    pub(crate) fn resolve() -> Self {
        let setting = std::env::var(HOSTED_SANDBOX_ENV).unwrap_or_default();
        let extra = std::env::var(HOSTED_EXEC_ROOTS_ENV).unwrap_or_default();
        Self::resolve_from(
            setting.trim(),
            &extra,
            crate::sandbox_probe::hosted_confinement(),
        )
    }

    /// The pure core (unit-tested without env mutation).
    pub(crate) fn resolve_from(
        setting: &str,
        extra_roots: &str,
        probe: Result<(), String>,
    ) -> Self {
        let exec_roots = exec_roots(extra_roots);
        let mode = match (setting, probe) {
            ("off", _) => SandboxMode::Off {
                reason: "sandbox off by operator (KX_HOSTED_SANDBOX=off)".into(),
            },
            ("on", Err(reason)) => SandboxMode::Refuse { reason },
            ("on" | "", Ok(())) => SandboxMode::Enforce,
            ("", Err(reason)) => SandboxMode::Off {
                reason: format!("hosted isolation is not enforceable on this platform: {reason}"),
            },
            (other, probe) => {
                // An unknown value never silently widens: treat it as unset.
                tracing::warn!(value = %other, "unknown KX_HOSTED_SANDBOX value; using the probe default");
                match probe {
                    Ok(()) => SandboxMode::Enforce,
                    Err(reason) => SandboxMode::Off {
                        reason: format!(
                            "hosted isolation is not enforceable on this platform: {reason}"
                        ),
                    },
                }
            }
        };
        Self { mode, exec_roots }
    }

    /// The posture string a status detail / startup log carries. One source so
    /// the two surfaces can never disagree.
    pub(crate) fn posture(&self) -> String {
        match &self.mode {
            SandboxMode::Enforce => "sandboxed (platform sandbox, deny-default)".into(),
            SandboxMode::Off { reason } => format!("UNSANDBOXED — {reason}"),
            SandboxMode::Refuse { reason } => {
                format!(
                    "hosted start refused — {reason}; set KX_HOSTED_SANDBOX=off to run unconfined"
                )
            }
        }
    }
}

/// The node/npm toolchain roots granted read+exec: the DIRECTORIES of `node` and
/// `npm` as resolved from the current process PATH (captured once, at policy
/// resolution), the conventional system bins, plus the operator's extras.
/// Directories, never binaries — an SBPL `subpath` naming a binary matches
/// nothing (the #393 lesson).
fn exec_roots(extra: &str) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for prog in ["node", "npm"] {
        if let Some(p) = which(prog) {
            if let Some(dir) = p.parent() {
                // The INSTALL PREFIX, not just `bin`: `npm` is a symlink into
                // `<prefix>/lib/node_modules/npm`, and it requires its own
                // library tree from there. A bin-only grant loads the launcher
                // and then dies with `Cannot find module '../lib/cli.js'`.
                // Reading the toolchain you are already permitted to EXECUTE is
                // not a widening — it is the same tree, resolved.
                push_canonical(&mut roots, dir.parent().unwrap_or(dir));
                push_canonical(&mut roots, dir);
            }
        }
    }
    for dir in ["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"] {
        push_canonical(&mut roots, Path::new(dir));
    }
    for dir in extra.split(':').filter(|s| !s.trim().is_empty()) {
        push_canonical(&mut roots, Path::new(dir));
    }
    roots
}

fn push_canonical(roots: &mut Vec<PathBuf>, dir: &Path) {
    // The kernel matches RESOLVED paths — a symlink rule silently never matches.
    if let Ok(c) = dir.canonicalize() {
        if !roots.contains(&c) {
            roots.push(c);
        }
    }
}

/// Resolve `prog` against the exec roots (the minimal PATH the child will see).
fn which(prog: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(prog))
        .find(|c| c.is_file())
}

/// The one spawn assembler for hosted children.
///
/// Always: `env_clear` + the minimal allowlist (PATH over the exec roots,
/// workdir-scoped HOME / npm cache / TMPDIR — created here), and its own process
/// group so stop can kill the whole tree.
///
/// `confine` additionally wraps the command in the platform sandbox — used for
/// the LONG-LIVED dev server; the one-shot install/build/typecheck children get
/// hygiene + group only (they need registry egress the sandbox cannot express).
///
/// # Errors
/// A human-readable refusal: the policy demands a sandbox that is unavailable,
/// the program cannot be resolved, or the workdir dirs cannot be created.
pub(crate) fn wrap_spawn(
    policy: &SandboxPolicy,
    prog: &str,
    args: &[String],
    workdir: &Path,
    confine: bool,
) -> Result<Command, String> {
    if let SandboxMode::Refuse { reason } = &policy.mode {
        return Err(format!(
            "{reason}; set KX_HOSTED_SANDBOX=off to run unconfined"
        ));
    }
    // The child's private, workdir-scoped homes (npm insists on a writable HOME
    // + cache; scoping them keeps everything inside the one RW grant).
    let home = workdir.join(".kx-home");
    let cache = workdir.join(".kx-npm-cache");
    let tmp = workdir.join(".kx-tmp");
    for d in [&home, &cache, &tmp] {
        std::fs::create_dir_all(d).map_err(|e| format!("hosted child dirs: {e}"))?;
    }
    let path_env = std::env::join_paths(policy.exec_roots.iter())
        .map_err(|e| format!("hosted PATH assembly: {e}"))?;

    let sandboxed = confine && matches!(policy.mode, SandboxMode::Enforce);
    let mut cmd = if sandboxed {
        // An absolute program stands as-is (`join` with an absolute RHS yields it);
        // a bare name resolves under the exec roots — the child's whole PATH.
        let resolved = policy
            .exec_roots
            .iter()
            .map(|d| d.join(prog))
            .find(|c| c.is_file())
            .ok_or_else(|| format!("cannot resolve {prog:?} under the hosted exec roots"))?;
        let resolved = resolved
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize {prog:?}: {e}"))?;
        let sandbox_exec = Path::new("/usr/bin/sandbox-exec");
        if !sandbox_exec.is_file() {
            return Err(
                "the platform sandbox binary (/usr/bin/sandbox-exec) is missing; set \
                 KX_HOSTED_SANDBOX=off to run unconfined"
                    .into(),
            );
        }
        // The program's OWN directory joins the exec grants: a custom dev command
        // (or a test fixture) may live outside the node roots, and an SBPL rule
        // must name the resolved DIRECTORY or it matches nothing.
        let mut roots = policy.exec_roots.clone();
        if let Some(dir) = resolved.parent() {
            push_canonical(&mut roots, dir);
        }
        let profile = dev_server_profile(workdir, &roots)?;
        let mut c = Command::new(sandbox_exec);
        c.arg("-p").arg(profile).arg(resolved).args(args);
        c
    } else {
        let mut c = Command::new(prog);
        c.args(args);
        c
    };
    cmd.current_dir(workdir)
        .env_clear()
        .env("PATH", path_env)
        .env("HOME", &home)
        .env("npm_config_cache", &cache)
        .env("TMPDIR", &tmp)
        .env("LANG", "en_US.UTF-8");
    // Its OWN process group: stop kills the group, reaching the vite/next
    // grandchild `start_kill` never did. Safe std builder (no pre_exec).
    #[cfg(unix)]
    cmd.process_group(0);
    Ok(cmd)
}

/// SIGKILL the whole process GROUP `pgid` (unix). Best-effort; the caller still
/// `start_kill`s the direct child (belt and braces — kill_on_drop only ever
/// reached the direct child).
#[cfg(unix)]
pub(crate) fn kill_group(pgid: i32) {
    let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), nix::sys::signal::SIGKILL);
}
#[cfg(not(unix))]
pub(crate) fn kill_group(_pgid: i32) {}

/// The profile for a hosted DEV SERVER (macOS SBPL): the SAME deny-default base
/// every sandboxed body goes through (`profile_from_warrant` — `system.sb` for
/// the runtime substrate, then exactly the granted mounts: RW over the app's own
/// workdir), plus the mechanism rules a dev server needs and the mapping has no
/// axis for: process-fork (npm → node → workers), read+exec over the node
/// roots + shells, exec inside the workdir (`node_modules/.bin` shims), and
/// loopback in AND out (the server LISTENS on 127.0.0.1; the gateway dials it).
/// Everything else stays denied — a model-authored page reading `~/.ssh`
/// server-side is a violation, observably.
fn dev_server_profile(workdir: &Path, exec_roots: &[PathBuf]) -> Result<String, String> {
    let workdir = workdir
        .canonicalize()
        .map_err(|e| format!("hosted workdir canonicalize: {e}"))?;
    let mut fs = kx_warrant::FsScope::empty();
    fs.mounts.insert(workdir, kx_warrant::FsMode::ReadWrite);
    let spec = kx_warrant::WarrantSpec {
        fs_scope: fs,
        executor_class: kx_warrant::ExecutorClass::MacOsSandbox,
        ..Default::default()
    };
    let base = kx_executor::backends::macos_sandbox::profile_from_warrant(&spec);
    let mut p = String::from_utf8(base.as_bytes().to_vec())
        .map_err(|e| format!("hosted profile encode: {e}"))?;
    p.push_str("\n;; hosted dev-server mechanism (not caller authority)\n");
    p.push_str("(allow process-fork)\n");
    p.push_str("(allow signal (target same-sandbox))\n");
    for root in exec_roots {
        push_subpath(&mut p, "process-exec", root);
        push_subpath(&mut p, "file-read*", root);
    }
    // The selector `/bin/sh` opens before it will run anything — denying it
    // yields `Error opening /private/var/select/sh` and an unusable shell, which
    // npm needs for every lifecycle script. (Same grant, same reason, as the
    // script sandbox.)
    push_subpath(&mut p, "file-read*", Path::new(SHELL_SELECTOR));
    // Shells for npm lifecycle scripts, and the workdir's own .bin shims.
    for dir in ["/bin", "/usr/bin"] {
        push_subpath(&mut p, "process-exec", Path::new(dir));
        push_subpath(&mut p, "file-read*", Path::new(dir));
    }
    let workdir = spec_workdir_path(&spec);
    push_subpath(&mut p, "process-exec", &workdir);
    // Node resolves its main module to a REAL path before running it, and that
    // walk `lstat`s every component ABOVE the workdir — none of which any grant
    // above names. Without this the dev server dies at startup with a bare
    // `EPERM: lstat '/Users'` and never binds. Granted as `literal`, one path
    // each: stat-ing a directory is not reading what is under it. (The script
    // sandbox carries the same rule for the same reason — one failure mode, one
    // remedy.)
    for dir in [&[workdir][..], exec_roots].concat() {
        for ancestor in ancestors_of(&dir) {
            push_literal(&mut p, "file-read-metadata", &ancestor);
        }
    }
    // Loopback both ways: the dev server LISTENS locally; nothing else.
    p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
    p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
    p.push_str("(allow network-bind (local ip \"localhost:*\"))\n");
    Ok(p)
}

/// The single RW mount the spec carries (the workdir) — kept as a helper so the
/// exec grant and the RW grant can never name different paths.
fn spec_workdir_path(spec: &kx_warrant::WarrantSpec) -> PathBuf {
    spec.fs_scope
        .mounts
        .keys()
        .next()
        .cloned()
        .unwrap_or_default()
}

fn push_subpath(profile: &mut String, op: &str, path: &Path) {
    let _ = writeln!(profile, "(allow {op} (subpath \"{}\"))", escape(path));
}

/// One exact path, never its subtree — what a metadata grant should be.
fn push_literal(profile: &mut String, op: &str, path: &Path) {
    let _ = writeln!(profile, "(allow {op} (literal \"{}\"))", escape(path));
}

fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Every directory above `path`, excluding the filesystem root — the components
/// a `realpath` walk stats on its way down. The root is excluded deliberately:
/// it needs no grant, and naming it would put a rule about `/` in a profile
/// whose whole point is that nothing is granted by default.
fn ancestors_of(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir.parent().is_none() {
            break;
        }
        out.push(dir.to_path_buf());
        current = dir.parent();
    }
    out
}

/// The rolling posture line every hosted status carries — pushed into the app's
/// log ring at start so a reviewer sees the isolation stance next to the npm
/// output it governs.
pub(crate) fn log_posture(
    logs: &std::sync::Arc<std::sync::Mutex<VecDeque<String>>>,
    policy: &SandboxPolicy,
) {
    if let Ok(mut l) = logs.lock() {
        l.push_back(format!("[isolation] {}", policy.posture()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_is_fail_closed_where_it_must_be_and_honest_where_it_cannot() {
        // Operator OFF: unconfined, loudly, regardless of the probe.
        let off = SandboxPolicy::resolve_from("off", "", Ok(()));
        assert!(matches!(off.mode, SandboxMode::Off { .. }));
        assert!(off.posture().contains("UNSANDBOXED"));
        // Operator ON + platform cannot: REFUSE (never a silent passthrough).
        let refuse = SandboxPolicy::resolve_from("on", "", Err("no loopback confinement".into()));
        assert!(matches!(refuse.mode, SandboxMode::Refuse { .. }));
        assert!(refuse.posture().contains("refused"));
        // Unset: probe-driven — enforce where possible, honest where not.
        assert!(matches!(
            SandboxPolicy::resolve_from("", "", Ok(())).mode,
            SandboxMode::Enforce
        ));
        let honest = SandboxPolicy::resolve_from("", "", Err("bwrap cannot confine".into()));
        match &honest.mode {
            SandboxMode::Off { reason } => assert!(reason.contains("bwrap cannot confine")),
            other => panic!("expected Off, got {other:?}"),
        }
        // Garbage value: never widens — behaves as unset.
        assert!(matches!(
            SandboxPolicy::resolve_from("banana", "", Ok(())).mode,
            SandboxMode::Enforce
        ));
    }

    #[test]
    fn a_refusing_policy_refuses_the_spawn_with_the_remedy() {
        let policy = SandboxPolicy {
            mode: SandboxMode::Refuse {
                reason: "platform cannot confine".into(),
            },
            exec_roots: vec![],
        };
        let dir = tempfile::tempdir().unwrap();
        let err = wrap_spawn(&policy, "npm", &[], dir.path(), true).unwrap_err();
        assert!(err.contains("platform cannot confine"));
        assert!(
            err.contains("KX_HOSTED_SANDBOX=off"),
            "names the remedy: {err}"
        );
    }

    #[test]
    fn the_profile_pins_workdir_rw_node_exec_and_loopback_only() {
        let dir = tempfile::tempdir().unwrap();
        let roots = vec![PathBuf::from("/usr/bin")];
        let p = dev_server_profile(dir.path(), &roots).unwrap();
        // The SAME deny-default base every sandboxed body uses (system.sb for the
        // runtime substrate), never a second hand-rolled one.
        assert!(p.starts_with("(version 1)\n(import \"system.sb\")\n(deny default)\n"));
        let canonical_workdir = dir.path().canonicalize().unwrap();
        let wd = canonical_workdir.to_string_lossy();
        assert!(p.contains(&format!("(allow file-write* (subpath \"{wd}\"))")));
        assert!(
            p.contains(&format!("(allow process-exec (subpath \"{wd}\"))")),
            "node_modules/.bin shims are execed inside the workdir"
        );
        assert!(p.contains("(allow process-exec (subpath \"/usr/bin\"))"));
        assert!(
            p.contains("(allow process-fork)"),
            "dev servers fork — a stated grant"
        );
        assert!(
            p.contains("network-inbound (local ip \"localhost:*\")"),
            "the server LISTENS"
        );
        assert!(
            !p.contains("(allow network-outbound (remote ip \"*"),
            "no open egress, ever"
        );
        assert!(!p.contains("(allow network*)"), "no blanket network");

        // Every ancestor of the workdir is stat-able — a `realpath` walk touches
        // each one, and without this node dies resolving its own main module
        // (`EPERM: lstat '/Users'`) before it ever binds a port. Found live: the
        // fixture binary the e2e runs needs no module resolution, so only a REAL
        // dev server exercises this path.
        for ancestor in ancestors_of(&canonical_workdir) {
            let a = ancestor.to_string_lossy();
            assert!(
                p.contains(&format!("(allow file-read-metadata (literal \"{a}\"))")),
                "the path walk down to the workdir stats {a}"
            );
            assert!(
                !p.contains(&format!("(allow file-read* (subpath \"{a}\"))")),
                "stat-ing {a} must not become permission to READ what is under it"
            );
        }
    }

    #[test]
    fn ancestors_stop_short_of_the_filesystem_root() {
        let got = ancestors_of(Path::new("/a/b/c/d"));
        assert_eq!(
            got,
            vec![
                PathBuf::from("/a/b/c"),
                PathBuf::from("/a/b"),
                PathBuf::from("/a")
            ],
            "`/` needs no grant, and naming it in a deny-default profile reads as a hole"
        );
    }

    #[test]
    fn wrap_spawn_clears_the_environment_to_the_allowlist() {
        let policy = SandboxPolicy {
            mode: SandboxMode::Off {
                reason: "test".into(),
            },
            exec_roots: vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")],
        };
        let dir = tempfile::tempdir().unwrap();
        let cmd = wrap_spawn(&policy, "/bin/echo", &["x".into()], dir.path(), false).unwrap();
        let std_cmd = cmd.as_std();
        // env_clear ⇒ the ONLY vars are the allowlist (a None value = removed).
        let keys: Vec<String> = std_cmd
            .get_envs()
            .filter(|(_, v)| v.is_some())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["HOME", "LANG", "PATH", "TMPDIR", "npm_config_cache"]
        );
        // The workdir-scoped homes were created.
        assert!(dir.path().join(".kx-home").is_dir());
        assert!(dir.path().join(".kx-npm-cache").is_dir());
    }
}
