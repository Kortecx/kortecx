//! Real, sandboxed Mote body-execution SEAM for the embedded `kx serve` worker.
//!
//! Composes the EXISTING public `kx-executor` surface
//! ([`BwrapExecutor`]/[`MacOsSandboxExecutor`] + [`ContentStoreBodyResolver`])
//! behind a router the gateway binary owns — WITHOUT touching the frozen trio
//! (`kx-executor`/`kx-scheduler`/`kx-inference`).
//!
//! `RouterExecutor` dispatches per leased Mote:
//! - a Mote whose `def.logic_ref` is a registered **real body** → run it inside
//!   the platform sandbox (bwrap on Linux, sandbox-exec/Seatbelt on macOS), then
//!   **reconcile** its result bytes into the shared content store so the
//!   coordinator's D55 phantom-ref guard passes at commit;
//! - any other (a bodyless PURE Mote, e.g. `echo`) → the honest passthrough
//!   fallback (it commits the Mote's real input, never a placeholder).
//!
//! No Mote-body binary is provisioned on the serve path, so `real_body_ref` is
//! `None` and every leased Mote takes the passthrough fallback. That is still
//! correct: a bodyless PURE Mote has nothing to run, and passthrough commits its
//! real input rather than a placeholder.
//!
//! ## The sandbox is live, by a different door
//!
//! [`spawn_body_in_sandbox`] is the one place this serve spawns anything into a
//! platform sandbox, and it is reached from two directions:
//!
//! - [`RouterExecutor`], for a leased Mote whose `logic_ref` names a registered
//!   body — the Mote-body plane, which is dormant until a body is registered;
//! - the **script capability** ([`crate::scripts`]), on every script a tool call
//!   fires — the effect plane, which is live.
//!
//! The two differ in what reaches the body as `argv[1]` and in who proves the
//! printed ref names real bytes. A Mote body reads the Mote's identity and the
//! router reconstructs its result; a script's body is the shim, which reads a
//! descriptor the capability encoded and writes its result where only the
//! capability knows to look. Those are genuinely different contracts, so they
//! share the spawn and part ways at reconciliation rather than being forced into
//! one path.
//!
//! **Fail-closed (Golden Rule 9).** When the sandbox cannot run (no `bwrap`,
//! blocked user-namespaces, non-matching platform), the spawn returns
//! [`MoteExecutorError`]; the worker backs off and a script dispatch refuses.
//! Neither ever falls back to un-sandboxed host execution. The canonical-digest
//! engine path (`kx run`, a separate `TestMoteExecutor::deterministic()`) is
//! untouched.

use std::io::Write as _;
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_executor::{
    BodyResolver, BwrapExecutor, ContentStoreBodyResolver, MacOsSandboxExecutor,
    MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs,
};
use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

/// The content-prefix half of the `pure_body` contract
/// (`kx-executor/examples/pure_body.rs`): the body prints
/// `result_ref = BLAKE3(PURE_BODY_PREFIX ‖ input)` on stdout. By the
/// content-addressing identity, the object whose ref that IS equals exactly
/// `PURE_BODY_PREFIX ‖ input` — so the gateway reconstructs + `put`s it to
/// satisfy the coordinator's D55 ref-existence guard. Kept in lock-step with
/// the example's `b"kx-executor-pure-body-result"` literal.
const PURE_BODY_PREFIX: &[u8] = b"kx-executor-pure-body-result";

/// A [`MoteExecutor`] the gateway binary composes from the public `kx-executor`
/// surface. See the module docs. Holds the shared content store (for the body
/// resolver + the result reconciliation), the registered real-body ref to route
/// on, the embedded worker's executor class, and the deterministic fallback.
pub(crate) struct RouterExecutor {
    /// The shared content store — clones feed [`ContentStoreBodyResolver`] (body
    /// materialization) and back the result reconciliation `put` (D55).
    store: LocalFsContentStore,
    /// The content-ref of a registered real body (== its `logic_ref` bytes). A
    /// leased Mote carrying this `logic_ref` is dispatched to the sandbox. `None`
    /// in the OSS serve path (no body provisioned) — every Mote then takes the
    /// honest passthrough fallback.
    real_body_ref: Option<ContentRef>,
    /// The platform executor class the embedded worker registered as
    /// ([`crate::server::default_executor_class`]).
    exec_class: ExecutorClass,
    /// The honest passthrough fallback for bodyless PURE Motes (e.g. `echo`) —
    /// commits the Mote's real input, never a fabricated placeholder.
    fallback: Arc<dyn MoteExecutor>,
}

impl RouterExecutor {
    /// Compose the router. `real_body_ref` is the ref the gateway `put` the body
    /// bytes under at startup (`None` ⇒ pure fallback behavior).
    pub(crate) fn new(
        store: LocalFsContentStore,
        real_body_ref: Option<ContentRef>,
        exec_class: ExecutorClass,
        fallback: Arc<dyn MoteExecutor>,
    ) -> Self {
        Self {
            store,
            real_body_ref,
            exec_class,
            fallback,
        }
    }

    /// Whether this Mote's `logic_ref` is the registered real body.
    fn is_real_body(&self, mote: &Mote) -> bool {
        self.real_body_ref.is_some_and(|registered| {
            ContentRef::from_bytes(*mote.def.logic_ref.as_bytes()) == registered
        })
    }

    /// Run the Mote's body inside the platform sandbox, then reconcile its result
    /// bytes into the store (delegates to [`run_body_in_sandbox`], shared with the
    /// startup probe).
    fn run_sandboxed(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        run_body_in_sandbox(&self.store, self.exec_class, mote, warrant, env)
    }
}

/// Run `mote`'s body in the platform sandbox under `warrant`, then reconcile its
/// output into `store`. The body program is materialized from `logic_ref` by
/// [`ContentStoreBodyResolver`]; its per-Mote input is the Mote's identity bytes
/// (deterministic ⇒ exactly-once-per-input). Used by [`RouterExecutor`] when a real
/// body is registered.
fn run_body_in_sandbox(
    store: &LocalFsContentStore,
    exec_class: ExecutorClass,
    mote: &Mote,
    warrant: &WarrantSpec,
    env: Option<Rootfs>,
) -> Result<MoteExecutionResult, MoteExecutorError> {
    let input_bytes = mote.id.as_bytes().to_vec();
    let result = spawn_body_in_sandbox(store, exec_class, mote, warrant, env, &input_bytes, None)?;

    // Reconcile (D55): the result object IS `PURE_BODY_PREFIX ‖ input`. `put` it
    // so the coordinator can verify the committed ref exists, then assert the
    // body's printed ref matches (a mismatch ⇒ a phantom; fail closed).
    let mut object = Vec::with_capacity(PURE_BODY_PREFIX.len() + input_bytes.len());
    object.extend_from_slice(PURE_BODY_PREFIX);
    object.extend_from_slice(&input_bytes);
    let put_ref = store
        .put(&object)
        .map_err(|e| internal(&format!("reconcile put: {e}")))?;
    if put_ref != result.result_ref {
        return Err(internal(
            "sandbox result_ref != reconstructed object ref (phantom result rejected)",
        ));
    }
    Ok(result)
}

/// Spawn `mote`'s body in the platform sandbox under `warrant` and return what it
/// printed — WITHOUT reconciling, which is the contract-specific step each caller
/// owns (see the module docs).
///
/// The body program is materialized from `mote.def.logic_ref` by
/// [`ContentStoreBodyResolver`], and `input_bytes` becomes the tempfile the body
/// reads as `argv[1]`.
///
/// `input_dir` places that tempfile in a caller-chosen directory instead of the
/// system temp dir. It matters because the sandbox denies by default: the caller
/// must have granted a mount covering wherever the input lands, and a caller with
/// its own scratch directory can grant that one narrowly rather than opening the
/// whole system temp tree for reads.
///
/// This is the ONLY place the serve spawns into a sandbox. Keeping the backend
/// selection and its fail-closed refusal in one function is what stops the
/// Mote-body plane and the script plane from drifting into two different
/// definitions of "sandboxed".
pub(crate) fn spawn_body_in_sandbox(
    store: &LocalFsContentStore,
    exec_class: ExecutorClass,
    mote: &Mote,
    warrant: &WarrantSpec,
    env: Option<Rootfs>,
    input_bytes: &[u8],
    input_dir: Option<&std::path::Path>,
) -> Result<MoteExecutionResult, MoteExecutorError> {
    // 1. The body's input → a tempfile it reads as argv[1]. The NamedTempFile
    //    MUST outlive `run()` (the child reads it), so it stays in scope until
    //    the end of this function.
    let mut input_file = match input_dir {
        Some(dir) => tempfile::NamedTempFile::new_in(dir),
        None => tempfile::NamedTempFile::new(),
    }
    .map_err(|e| internal(&format!("input tempfile: {e}")))?;
    input_file
        .write_all(input_bytes)
        .map_err(|e| internal(&format!("write input: {e}")))?;
    input_file
        .flush()
        .map_err(|e| internal(&format!("flush input: {e}")))?;
    let input_path = input_file.path().to_path_buf();

    // 2. The body resolver materializes `logic_ref` → a chmod-+x tempfile.
    let resolver: Arc<dyn BodyResolver> = Arc::new(ContentStoreBodyResolver::new(store.clone()));

    // 3. The platform sandbox, constructed per-call (so each lease gets its own
    //    per-Mote input). Only the two real-spawn backends are wired into serve;
    //    anything else fails closed.
    let result = match exec_class {
        ExecutorClass::MacOsSandbox => MacOsSandboxExecutor::new()
            .with_body_resolver(resolver)
            .with_input_file(input_path)
            .run(mote, warrant, env),
        ExecutorClass::Bwrap => BwrapExecutor::new()
            .with_body_resolver(resolver)
            .with_input_file(input_path)
            .run(mote, warrant, env),
        other => Err(MoteExecutorError::BackendUnsupported {
            class: other,
            reason: "kx serve wires only the bwrap/macOS sandbox backends".into(),
        }),
    }?;

    // Keep the input tempfile alive until here (the sandboxed child read it).
    drop(input_file);
    Ok(result)
}

/// The directories a script run needs beyond the caller's own grants — the
/// runtime's plumbing, not the caller's authority.
///
/// Kept separate from the warrant on purpose. `warrant.fs_scope` says what the
/// *caller* may reach and is the thing the broker already proved; these are the
/// paths the *mechanism* needs to function (where the shim lives, where the
/// script was materialized, where the result goes, which interpreter runs). None
/// of them carries user data, so keeping the two apart means reading the warrant
/// still answers "what can this script reach" without the plumbing muddying it.
pub(crate) struct ScriptPlumbing {
    /// Directories that must be readable AND executable (the shim's directory,
    /// the interpreter's).
    pub(crate) exec_dirs: Vec<std::path::PathBuf>,
    /// Directories that must be readable (the script, the descriptor, the
    /// interpreter's standard library).
    pub(crate) read_dirs: Vec<std::path::PathBuf>,
    /// The single writable directory — where the result is written.
    pub(crate) write_dir: std::path::PathBuf,
}

/// Run the script shim in the platform sandbox and return the 64-hex ref it
/// printed.
///
/// Two implementations, because the two platforms restrict different things:
///
/// - **Linux** delegates to [`spawn_body_in_sandbox`]. `bwrap` runs the body in a
///   user namespace where `fork` is unrestricted, so the warrant-derived argv is
///   sufficient and there is nothing to add.
/// - **macOS** builds the profile here. `sandbox_init` starts from
///   `(deny default)` and `system.sb` does not grant `process-fork`, so a shim
///   that spawns an interpreter — or any script that spawns a subprocess — dies
///   with a bare `EPERM` naming no rule. The warrant→SBPL mapping has no axis for
///   that: it renders `fs_scope` and `net_scope` and nothing else. So this reuses
///   that mapping verbatim for policy and appends the mechanism rules it cannot
///   express.
///
/// The split is deliberate and narrow: the *policy* (what the caller may reach)
/// comes from one shared pure function on both platforms; only the *mechanism*
/// differs, which is what actually differs between the two sandboxes.
pub(crate) fn run_script_body(
    store: &LocalFsContentStore,
    exec_class: ExecutorClass,
    shim_ref: ContentRef,
    mote: &Mote,
    warrant: &WarrantSpec,
    descriptor_bytes: &[u8],
    plumbing: &ScriptPlumbing,
) -> Result<String, MoteExecutorError> {
    #[cfg(target_os = "macos")]
    {
        let _ = (exec_class, mote);
        macos::run(store, shim_ref, warrant, descriptor_bytes, plumbing)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (shim_ref, plumbing);
        let result = spawn_body_in_sandbox(
            store,
            exec_class,
            mote,
            warrant,
            None,
            descriptor_bytes,
            Some(&plumbing.write_dir),
        )?;
        Ok(hex32(result.result_ref.as_bytes()))
    }
}

/// Render 32 bytes as the 64 lowercase-hex characters a body prints. Only the
/// Linux arm needs it — the macOS arm reads the body's printed string directly.
#[cfg(not(target_os = "macos"))]
fn hex32(bytes: &[u8; 32]) -> String {
    const NIBBLES: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(char::from(NIBBLES[usize::from(byte >> 4)]));
        out.push(char::from(NIBBLES[usize::from(byte & 0x0F)]));
    }
    out
}

/// The macOS spawn: the frozen warrant→SBPL mapping plus the mechanism rules it
/// cannot express. See [`run_script_body`].
#[cfg(target_os = "macos")]
mod macos {
    use std::fmt::Write as _;
    use std::process::{Command, Stdio};

    use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
    use kx_executor::backends::macos_sandbox::profile_from_warrant;
    use kx_executor::MoteExecutorError;
    use kx_warrant::WarrantSpec;

    use super::{internal, ScriptPlumbing};

    /// The shell selector `/bin/sh` opens before it will run anything. Denying it
    /// produces `Error opening /private/var/select/sh` and an unusable shell.
    const SHELL_SELECTOR: &str = "/private/var/select";

    pub(super) fn run(
        store: &LocalFsContentStore,
        shim_ref: ContentRef,
        warrant: &WarrantSpec,
        descriptor_bytes: &[u8],
        plumbing: &ScriptPlumbing,
    ) -> Result<String, MoteExecutorError> {
        // The shim, materialized executable in its own directory so the exec rule
        // covering it covers nothing else.
        let bin_dir =
            tempfile::tempdir().map_err(|e| internal(&format!("shim dir: {e}")))?;
        let shim_path = bin_dir.path().join("shim");
        let shim_bytes = store
            .get(&shim_ref)
            .map_err(|e| internal(&format!("read shim: {e}")))?;
        std::fs::write(&shim_path, &shim_bytes)
            .map_err(|e| internal(&format!("write shim: {e}")))?;
        set_executable(&shim_path)?;

        let descriptor_path = plumbing.write_dir.join("descriptor");
        std::fs::write(&descriptor_path, descriptor_bytes)
            .map_err(|e| internal(&format!("write descriptor: {e}")))?;

        let profile = build_profile(warrant, plumbing, bin_dir.path())?;

        // `sandbox-exec -p <profile> <shim> <descriptor>`. The child's cwd is the
        // writable directory: it is inherited, and a shell whose cwd it cannot
        // stat fails `getcwd` before it runs a line.
        let output = Command::new("/usr/bin/sandbox-exec")
            .arg("-p")
            .arg(&profile)
            .arg(&shim_path)
            .arg(&descriptor_path)
            .current_dir(&plumbing.write_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| internal(&format!("sandbox-exec: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(MoteExecutorError::BodyExited {
                code: output.status.code().unwrap_or(-1),
            })
            .map_err(|e| internal(&format!("{e}; {}", stderr.trim())));
        }
        let printed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if printed.len() != 64 || !printed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(internal(
                "the sandboxed body did not print a 64-character content ref",
            ));
        }
        drop(bin_dir);
        Ok(printed)
    }

    /// The warrant's own SBPL, plus the rules the mapping has no axis for.
    fn build_profile(
        warrant: &WarrantSpec,
        plumbing: &ScriptPlumbing,
        bin_dir: &std::path::Path,
    ) -> Result<String, MoteExecutorError> {
        // Policy: exactly what the caller granted, rendered by the same pure
        // function every other sandboxed body goes through.
        let mut profile = String::from_utf8(profile_from_warrant(warrant).as_bytes().to_vec())
            .map_err(|e| internal(&format!("profile encode: {e}")))?;

        // Mechanism. `process-fork` is the one that blocks everything: without it
        // the shim cannot spawn the interpreter, and no script can spawn a
        // subprocess.
        profile.push_str("\n;; runtime plumbing (not caller authority)\n");
        profile.push_str("(allow process-fork)\n");
        push_rule(&mut profile, "file-read*", SHELL_SELECTOR);

        for dir in std::iter::once(&bin_dir.to_path_buf()).chain(&plumbing.exec_dirs) {
            let path = canonical(dir);
            // Read AND exec: `subpath` is a directory prefix matcher, so a rule
            // naming a binary matches nothing — and dyld must read what it runs.
            push_rule(&mut profile, "process-exec", &path);
            push_rule(&mut profile, "file-read*", &path);
        }
        for dir in &plumbing.read_dirs {
            push_rule(&mut profile, "file-read*", &canonical(dir));
        }
        let write_dir = canonical(&plumbing.write_dir);
        push_rule(&mut profile, "file-read*", &write_dir);
        push_rule(&mut profile, "file-write*", &write_dir);
        Ok(profile)
    }

    /// `(allow <op> (subpath "<path>"))`, escaped for SBPL's Lisp strings.
    fn push_rule(profile: &mut String, op: &str, path: &str) {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(profile, "(allow {op} (subpath \"{escaped}\"))");
    }

    /// The sandbox matches against the kernel's resolved path, so a rule written
    /// for a symlink (`/tmp/...` rather than `/private/tmp/...`) silently never
    /// matches and the run fails with no diagnostic.
    fn canonical(path: &std::path::Path) -> String {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    fn set_executable(path: &std::path::Path) -> Result<(), MoteExecutorError> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| internal(&format!("chmod shim: {e}")))
    }
}

/// Locate a binary the serve ships alongside itself — an operator override first,
/// then the container image path, then the sibling of the running executable in a
/// cargo `target/` tree (the developer case).
///
/// Returns `None` when no candidate exists. Every caller treats that as
/// "the capability this binary backs is unavailable", never as a reason to run
/// something else: a missing sandbox shim means scripts do not register at all.
pub(crate) fn bundled_binary_path(bin: &str, env_override: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    if let Some(over) = std::env::var_os(env_override) {
        let path = PathBuf::from(over);
        if path.exists() {
            return Some(path);
        }
    }
    let in_image = PathBuf::from(format!("/usr/local/libexec/kx/{bin}"));
    if in_image.exists() {
        return Some(in_image);
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join(bin);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

impl MoteExecutor for RouterExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        if self.is_real_body(mote) {
            self.run_sandboxed(mote, warrant, env)
        } else {
            self.fallback.run(mote, warrant, env)
        }
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        // The embedded worker leases on a single class; both the real sandbox path
        // and the storing fallback serve it.
        executor_class == self.exec_class || self.fallback.supports(executor_class)
    }
}

/// A fail-closed [`MoteExecutorError::Internal`] from a `&str` diagnostic.
fn internal(reason: &str) -> MoteExecutorError {
    MoteExecutorError::Internal {
        reason: reason.to_string(),
    }
}
