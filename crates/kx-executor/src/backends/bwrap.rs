//! `BwrapExecutor` — bubblewrap-based sandbox executor for Linux.
//! **PR 9a-hardening-3** wires the real fork+execvp(`bwrap`) path through
//! `crate::spawn::spawn_body`. The bwrap argv is assembled from the
//! `WarrantSpec`'s fs_scope (`--ro-bind` for ReadOnly + ExecOnly;
//! `--bind` for ReadWrite) + net_scope (`--unshare-net` for `None`) +
//! the body binary path + its argv. Resource ceilings (cpu / mem / fd /
//! disk) are applied via `setrlimit` in the child between fork and
//! execvp (per `crate::spawn::apply_rlimits`).
//!
//! Per D31, bubblewrap is the Linux default executor. The constructor
//! optionally takes the path to the `bwrap` binary; `with_body_default_bwrap`
//! probes `PATH` at construction time + refuses with `BackendUnsupported`
//! if `bwrap` is not installed. The integration test runtime-skips on
//! Linux machines without bwrap.

use std::path::{Path, PathBuf};

use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

#[cfg(target_os = "linux")]
use kx_warrant::{FsMode, NetScope};

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};

/// Bubblewrap-based sandbox executor (Linux default per D41).
#[derive(Debug, Default, Clone)]
#[allow(clippy::struct_field_names)] // each *_path field carries distinct semantics
pub struct BwrapExecutor {
    /// Absolute path to the body binary the spawned child will execvp into
    /// after `bwrap` sets up the sandbox. When `None`, `run()` returns
    /// `BackendUnsupported` (the PR 9a skeleton shape preserved for
    /// back-compat with `default_executor()`).
    #[allow(dead_code)] // read only on target_os = "linux" via `run_linux`
    body_path: Option<PathBuf>,
    /// Absolute path to a file the body will read as its input (passed as
    /// `argv[1]`).
    #[allow(dead_code)] // read only on target_os = "linux" via `run_linux`
    input_path: Option<PathBuf>,
    /// Path to the `bwrap` binary on disk. Defaults to `bwrap` (resolved
    /// via `PATH`); production callers can override.
    #[allow(dead_code)] // read only on target_os = "linux" via `run_linux`
    bwrap_path: PathBuf,
}

impl BwrapExecutor {
    /// Construct a new `BwrapExecutor` with no configured body
    /// (preserves the PR 9a `BackendUnsupported`-on-run shape). Defaults
    /// `bwrap_path` to `"bwrap"` (PATH-resolved at exec time).
    #[must_use]
    pub fn new() -> Self {
        Self {
            body_path: None,
            input_path: None,
            bwrap_path: PathBuf::from("bwrap"),
        }
    }

    /// Construct a `BwrapExecutor` with a configured body binary. The
    /// spawned child execvps `bwrap` with argv that ends in
    /// `body_path input_file_path`.
    #[must_use]
    pub fn with_body(body_path: PathBuf) -> Self {
        Self {
            body_path: Some(body_path),
            input_path: None,
            bwrap_path: PathBuf::from("bwrap"),
        }
    }

    /// Set the input file path passed as the body's `argv[1]`.
    #[must_use]
    pub fn with_input_file(mut self, input_path: PathBuf) -> Self {
        self.input_path = Some(input_path);
        self
    }

    /// Override the `bwrap` binary path (e.g., for testing or non-PATH
    /// installations).
    #[must_use]
    pub fn with_bwrap_path(mut self, bwrap_path: PathBuf) -> Self {
        self.bwrap_path = bwrap_path;
        self
    }
}

impl MoteExecutor for BwrapExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        #[cfg(target_os = "linux")]
        {
            self.run_linux(mote, warrant, env.as_ref())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (mote, warrant, env);
            Err(MoteExecutorError::BackendUnsupported {
                class: ExecutorClass::Bwrap,
                reason: "Bwrap backend only runs on target_os = \"linux\"".into(),
            })
        }
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        // The Bwrap variant is the Linux default; on non-Linux targets the
        // backend exists for trait-object uniformity but reports unsupported.
        cfg!(target_os = "linux") && executor_class == ExecutorClass::Bwrap
    }
}

#[cfg(target_os = "linux")]
impl BwrapExecutor {
    /// Real Linux spawn path. Assembles the bwrap argv from the warrant,
    /// forks, applies setrlimit in the child, execvps `bwrap` with the
    /// assembled argv, reads the body's stdout, parses the 64-hex-char
    /// result_ref, returns `MoteExecutionResult`.
    fn run_linux(
        &self,
        _mote: &Mote,
        warrant: &WarrantSpec,
        _env: Option<&Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        let body_path = self
            .body_path
            .as_ref()
            .ok_or(MoteExecutorError::BackendUnsupported {
                class: ExecutorClass::Bwrap,
                reason: "no body_path configured — construct via BwrapExecutor::with_body(path)"
                    .into(),
            })?;
        let input_path = self
            .input_path
            .as_ref()
            .ok_or(MoteExecutorError::Internal {
                reason: "no input_path configured — call .with_input_file(path) on the executor"
                    .into(),
            })?;

        // 1. Assemble bwrap argv from the warrant.
        let bwrap_path_str = self.bwrap_path.to_string_lossy().into_owned();
        let body_path_str = body_path.to_string_lossy().into_owned();
        let input_path_str = input_path.to_string_lossy().into_owned();
        let argv =
            bwrap_argv_from_warrant(&bwrap_path_str, warrant, &body_path_str, &input_path_str);

        // 2. Spawn — fork + pre-exec(setrlimit) + execvp(bwrap).
        // bwrap itself applies the sandbox (no need for a sandbox_init-
        // equivalent on Linux; the kernel honors bwrap's user-namespace
        // + capability drops).
        let started_at_epoch_ms = now_epoch_ms();
        let ceiling = warrant.resource_ceiling;
        let wall_clock_ms = if warrant.resource_ceiling.wall_clock_ms > 0 {
            Some(warrant.resource_ceiling.wall_clock_ms)
        } else {
            None
        };
        let outcome = crate::spawn::spawn_body(
            &bwrap_path_str,
            &argv,
            Box::new(move || crate::spawn::apply_rlimits(&ceiling)),
            wall_clock_ms,
        )?;
        let finished_at_epoch_ms = now_epoch_ms();

        // 3. Parse the body's stdout: 64 hex chars → 32-byte ContentRef.
        if outcome.exit_code != 0 {
            return Err(MoteExecutorError::BodyExited {
                code: outcome.exit_code,
            });
        }
        let result_ref =
            crate::backends::macos_sandbox::parse_hex_ref(&outcome.stdout).map_err(|e| {
                MoteExecutorError::Internal {
                    reason: format!("body stdout parse: {e}"),
                }
            })?;

        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms,
            finished_at_epoch_ms,
        })
    }
}

#[cfg(target_os = "linux")]
fn now_epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| {
            #[allow(clippy::cast_possible_truncation)]
            let ms = d.as_millis() as u64;
            ms
        })
        .unwrap_or(0)
}

/// Compose the bwrap argv from a `WarrantSpec` + body binary path +
/// input file path.
///
/// **Wire format**:
/// `bwrap [--die-with-parent] [--proc /proc --dev /dev]
///        [--ro-bind /usr /usr] [--ro-bind /lib /lib] [--ro-bind /lib64 /lib64]
///        [--ro-bind /etc /etc]
///        [--ro-bind <ro_path> <ro_path>]* (per fs_scope ReadOnly + ExecOnly)
///        [--bind <rw_path> <rw_path>]* (per fs_scope ReadWrite)
///        [--unshare-net]? (when net_scope is None)
///        -- body_path input_path`
///
/// The `--die-with-parent` flag ensures the body terminates when the
/// executor parent dies. `--unshare-net` provides full network isolation
/// when `net_scope = None`; EgressAllowlist on Linux requires per-host
/// firewall rules (iptables / nftables) which need root — out of scope
/// for PR 9a-hardening-3.
///
/// System libraries (`/usr`, `/lib`, `/lib64`, `/etc`) are bound
/// unconditionally so dyld can resolve dynamic libraries; this is the
/// minimum-viable rootfs for a typical Linux binary. Production warrants
/// would narrow these via an explicit `warrant.environment_ref`-resolved
/// rootfs (out of scope here; the OCI rootfs extraction path lands in a
/// later hardening sweep).
#[cfg(target_os = "linux")]
pub(crate) fn bwrap_argv_from_warrant(
    bwrap_path: &str,
    warrant: &WarrantSpec,
    body_path: &str,
    input_path: &str,
) -> Vec<String> {
    let mut argv: Vec<String> = Vec::with_capacity(32);
    argv.push(bwrap_path.to_string());
    argv.push("--die-with-parent".into());
    argv.push("--proc".into());
    argv.push("/proc".into());
    argv.push("--dev".into());
    argv.push("/dev".into());

    // System library baseline (so dyld can resolve libsystem / glibc / etc).
    // Each is conditional: only emit if the path exists at the executor
    // construction site (we can't probe from inside this pure function).
    // For PR 9a-hardening-3 we emit unconditionally; bwrap silently no-ops
    // on missing source paths.
    for sys_path in ["/usr", "/lib", "/lib64", "/etc"] {
        argv.push("--ro-bind".into());
        argv.push(sys_path.into());
        argv.push(sys_path.into());
    }

    // fs_scope mounts.
    for (path, mode) in &warrant.fs_scope.mounts {
        let path_str = path.to_string_lossy().into_owned();
        match mode {
            FsMode::ReadOnly | FsMode::ExecOnly => {
                argv.push("--ro-bind".into());
                argv.push(path_str.clone());
                argv.push(path_str);
            }
            FsMode::ReadWrite => {
                argv.push("--bind".into());
                argv.push(path_str.clone());
                argv.push(path_str);
            }
        }
    }

    // net_scope. `None` => full network isolation via user-namespace.
    if matches!(warrant.net_scope, NetScope::None) {
        argv.push("--unshare-net".into());
    }
    // EgressAllowlist on Linux requires a parent-side per-host firewall
    // (iptables/nftables/netfilter) which needs root or NET_ADMIN. Out
    // of scope for PR 9a-hardening-3; the bwrap argv leaves net access
    // un-isolated in that case (deferred to the production rootfs
    // hardening sweep).

    // End-of-bwrap-flags marker.
    argv.push("--".into());
    // Body binary + its argv.
    argv.push(body_path.to_string());
    argv.push(input_path.to_string());

    argv
}

impl BwrapExecutor {
    /// Borrow the configured `body_path`. Returns `None` if the
    /// executor was constructed without one (via `BwrapExecutor::new()`).
    #[must_use]
    pub fn body_path(&self) -> Option<&Path> {
        self.body_path.as_deref()
    }

    /// Borrow the configured `input_path`. Returns `None` if not yet
    /// set via `with_input_file`.
    #[must_use]
    pub fn input_path(&self) -> Option<&Path> {
        self.input_path.as_deref()
    }
}
