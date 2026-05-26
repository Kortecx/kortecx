//! Spawn primitives for the platform backends (PR 9a-hardening-2).
//!
//! Provides `spawn_body` — fork + (optional) pre-exec hook + execvp + collect
//! stdout + waitpid. Returns the body's stdout bytes + exit status.
//!
//! **Unsafe-systems-code boundary**: this module contains the per-block
//! `unsafe { ... }` calls into `nix::unistd::fork`, `libc::sandbox_init`
//! (macOS), and the post-fork pre-exec hooks. Every `unsafe` block carries
//! a `// SAFETY:` comment naming the invariant it relies on. Higher layers
//! (`MacOsSandboxExecutor::run`, `BwrapExecutor::run`) consume this module
//! and remain `forbid(unsafe_code)`.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::CString;
use std::os::fd::IntoRawFd;

use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{fork, pipe, ForkResult};

use crate::executor_trait::MoteExecutorError;

/// Result of a successful body spawn — the bytes the body wrote to stdout
/// and its exit status.
#[derive(Debug, Clone)]
pub(crate) struct BodyOutcome {
    pub(crate) stdout: Vec<u8>,
    pub(crate) exit_code: i32,
}

/// Pre-exec hook signature. Runs in the child process AFTER fork and BEFORE
/// execvp; called from an async-signal-safe context (only async-signal-safe
/// libc functions are legal here per POSIX).
///
/// On error returns a non-zero exit code; the child immediately calls
/// `_exit(code)` without running destructors (which would be unsafe across
/// the fork boundary).
pub(crate) type PreExecHook = Box<dyn FnOnce() -> Result<(), i32> + Send>;

/// Fork + pre-exec hook + execvp + collect stdout + waitpid.
///
/// `body_path` is the absolute path to the Mote body binary. `argv` is the
/// argv to pass to execvp (argv\[0\] is conventionally the body's basename).
/// `pre_exec` runs in the child after fork, before execvp — typically loads
/// the sandbox profile (macOS) or applies setrlimit. `body_input` is the
/// bytes piped to the child's stdin (so the body can read its input without
/// touching the FS).
///
/// # Safety
///
/// This function calls `nix::unistd::fork`, which is `unsafe` because between
/// fork and exec, the child may only call async-signal-safe functions.
/// Callers MUST ensure the `pre_exec` hook only does async-signal-safe work
/// — `libc::sandbox_init` and `libc::setrlimit` are documented async-signal-
/// safe; allocating Rust types in the child is NOT. This module enforces
/// the discipline by calling `_exit` (NOT `std::process::exit` or `panic!`)
/// from the child path on any error.
pub(crate) fn spawn_body(
    body_path: &str,
    argv: &[String],
    pre_exec: PreExecHook,
) -> Result<BodyOutcome, MoteExecutorError> {
    // Pipe for the child's stdout → parent. We take ownership of both fd
    // ends via OwnedFd, then convert to RawFd so we can manually `libc::close`
    // them at fork boundaries without nix's RAII auto-closing them.
    let (stdout_read_owned, stdout_write_owned) =
        pipe().map_err(|e| MoteExecutorError::Internal {
            reason: format!("pipe: {e}"),
        })?;
    let stdout_read: libc::c_int = stdout_read_owned.into_raw_fd();
    let stdout_write: libc::c_int = stdout_write_owned.into_raw_fd();

    // Build argv as Vec<CString> so the child's execvp gets owned C strings.
    let argv_c: Vec<CString> = argv
        .iter()
        .map(|s| CString::new(s.as_str()).expect("argv contains no NUL bytes"))
        .collect();
    let argv_ptrs: Vec<*const libc::c_char> = argv_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let body_path_c = CString::new(body_path).expect("body_path contains no NUL bytes");

    // SAFETY: `fork()` is unsafe because between fork and exec the child may
    // only run async-signal-safe code. We immediately route the child into
    // a path that calls `_exit` on any error (NOT `panic!` or `std::process::
    // exit`), and the only Rust code in the child between fork and execvp
    // is the `pre_exec` hook (which is the caller's responsibility to keep
    // async-signal-safe) + the libc syscalls below (dup2, close — both
    // documented async-signal-safe).
    let fork_result =
        unsafe { fork() }.map_err(|e| MoteExecutorError::ProcessSpawnFailed { errno: e as i32 })?;

    match fork_result {
        ForkResult::Child => {
            // Child path: dup the write end of the pipe over stdout, close
            // both pipe fds, run the pre-exec hook, then execvp the body.
            // EVERY error path here MUST call `_exit` directly — NOT panic,
            // NOT std::process::exit (which runs Rust destructors that may
            // touch heap allocated state from the parent, which is undefined
            // behavior after fork).
            // SAFETY: libc::close + libc::dup2 are async-signal-safe;
            // stdout_read / stdout_write are valid open file descriptors
            // inherited from the parent's pipe() call.
            unsafe {
                libc::close(stdout_read);
            }
            // Replace stdout with the pipe's write end.
            // SAFETY: as above; libc::dup2 returns -1 on error.
            if unsafe { libc::dup2(stdout_write, libc::STDOUT_FILENO) } < 0 {
                // SAFETY: _exit is async-signal-safe; no destructors run.
                unsafe { libc::_exit(70) };
            }
            // Close the original pipe fd (stdout now points to the same kernel
            // resource via dup2).
            // SAFETY: stdout_write is the valid pipe-write fd we just dup'd.
            unsafe {
                libc::close(stdout_write);
            }

            // Run pre-exec hook (sandbox_init / setrlimit).
            if let Err(code) = pre_exec() {
                // SAFETY: as above.
                unsafe { libc::_exit(code) };
            }

            // execvp the body. argv_ptrs is a NULL-terminated C array of
            // C strings (we appended a null above). On success, execvp does
            // not return; on failure, it returns -1 and sets errno.
            // SAFETY: argv_ptrs is properly null-terminated; body_path_c is
            // a valid CStr. execvp is async-signal-safe per POSIX.
            unsafe {
                libc::execvp(body_path_c.as_ptr(), argv_ptrs.as_ptr());
            }
            // execvp returned → failure. _exit with an errno-marker code.
            // SAFETY: as above.
            unsafe { libc::_exit(71) };
        }
        ForkResult::Parent { child } => {
            // Parent path: close the write end of the pipe, read child's
            // stdout to EOF, then waitpid.
            // SAFETY: stdout_write is the parent's pipe-write fd; we no
            // longer need it (the child has its own duplicate).
            unsafe {
                libc::close(stdout_write);
            }
            let read_fd = stdout_read;
            let mut stdout_bytes = Vec::with_capacity(64);
            let mut buf = [0u8; 4096];
            loop {
                // SAFETY: read_fd is a valid open file descriptor (the read
                // end of the pipe we just created); buf is a valid mutable
                // slice. The signature is `read(fd, buf, count) -> ssize_t`.
                let n = unsafe {
                    libc::read(read_fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len())
                };
                if n < 0 {
                    // SAFETY: stdout_read is a valid open pipe-read fd we
                    // own; we close it before returning.
                    unsafe {
                        libc::close(stdout_read);
                    }
                    return Err(MoteExecutorError::Internal {
                        reason: format!("read from child stdout failed: errno {}", errno()),
                    });
                }
                if n == 0 {
                    break;
                }
                #[allow(clippy::cast_sign_loss)]
                stdout_bytes.extend_from_slice(&buf[..n as usize]);
            }
            // SAFETY: as above.
            unsafe {
                libc::close(stdout_read);
            }

            // Wait for the child to terminate and collect exit status.
            let status = waitpid(child, None).map_err(|e| MoteExecutorError::Internal {
                reason: format!("waitpid: {e}"),
            })?;
            let exit_code = match status {
                WaitStatus::Exited(_, code) => code,
                WaitStatus::Signaled(_, sig, _) => 128 + (sig as i32),
                other => {
                    return Err(MoteExecutorError::Internal {
                        reason: format!("unexpected wait status: {other:?}"),
                    });
                }
            };

            Ok(BodyOutcome {
                stdout: stdout_bytes,
                exit_code,
            })
        }
    }
}

/// Read the current thread's errno value. The libc symbol differs by
/// platform: macOS exposes `__error`; Linux exposes `__errno_location`.
/// Both return a pointer to a thread-local int.
fn errno() -> i32 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `__error` returns a pointer to thread-local errno storage
        // valid for the lifetime of the calling thread.
        unsafe { *libc::__error() }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `__errno_location` is the Linux equivalent; same safety
        // contract.
        unsafe { *libc::__errno_location() }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Other Unix targets: best-effort fallback via nix.
        nix::errno::Errno::last_raw()
    }
}

// ============================================================================
// macOS-specific: sandbox_init wrapper
// ============================================================================

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CString;

    // sandbox_init: load an SBPL profile in the current process. Returns 0
    // on success, -1 on error (with details in `*errorbuf`).
    extern "C" {
        fn sandbox_init(
            profile: *const libc::c_char,
            flags: u64,
            errorbuf: *mut *mut libc::c_char,
        ) -> libc::c_int;
        fn sandbox_free_error(errorbuf: *mut libc::c_char);
    }

    /// Load the given SBPL profile bytes into the calling process.
    ///
    /// **MUST be called in the child after fork, before execvp.** Calling in
    /// the parent would sandbox the executor itself.
    ///
    /// # Safety
    ///
    /// `sandbox_init` is documented async-signal-safe; safe to call from a
    /// post-fork child. The caller MUST hold the SBPL bytes as a valid
    /// C string (NUL-terminated) for the duration of this call. The
    /// returned `errorbuf` (if non-null on failure) is freed via
    /// `sandbox_free_error` before returning the error code.
    pub(crate) fn load_profile(profile_bytes: &[u8]) -> Result<(), i32> {
        // Construct a NUL-terminated C string from the SBPL bytes. The
        // bytes contain printable ASCII (per D46's deny-default template
        // + the per-axis builder); embedded NULs would be a programming
        // error worth refusing here rather than papering over.
        let Ok(profile_c) = CString::new(profile_bytes) else {
            return Err(72); // embedded NUL in SBPL — bug
        };
        let mut errorbuf: *mut libc::c_char = std::ptr::null_mut();
        let errorbuf_ptr: *mut *mut libc::c_char = &raw mut errorbuf;
        // SAFETY: profile_c lives until end of this function; errorbuf_ptr is
        // a stack-allocated pointer the FFI writes to. `sandbox_init` is
        // async-signal-safe per Apple's documentation.
        let rc = unsafe { sandbox_init(profile_c.as_ptr(), 0, errorbuf_ptr) };
        if rc == 0 {
            // Success path: errorbuf is conventionally null; nothing to free.
            return Ok(());
        }
        // Failure: free the error buffer (if any) and return a marker code.
        if !errorbuf.is_null() {
            // SAFETY: errorbuf was allocated by sandbox_init; sandbox_free_error
            // is the documented deallocator.
            unsafe { sandbox_free_error(errorbuf) };
        }
        Err(73)
    }

    // ABI placeholder to silence "unused" warnings when no caller is wired.
    pub(crate) const _ABI_CHECK: () = ();
}

#[cfg(target_os = "macos")]
pub(crate) use macos::load_profile;

// ============================================================================
// Cross-platform: setrlimit pre-exec helper (PR 9a-hardening-3)
// ============================================================================

/// Apply a `ResourceCeiling` as `setrlimit` calls in the calling process.
/// **MUST be called from the post-fork child between fork and execvp** —
/// `setrlimit` is async-signal-safe per POSIX and the child inherits the
/// limits across the subsequent execvp.
///
/// Maps:
/// - `mem_bytes` → `RLIMIT_AS` (virtual-memory limit; closest portable
///   approximation to "RSS cap" without diving into platform-specific
///   `RLIMIT_RSS`).
/// - `fd_count` → `RLIMIT_NOFILE` (max open file descriptors).
/// - `disk_bytes` → `RLIMIT_FSIZE` (max file size the process can create).
/// - `cpu_milli` → `RLIMIT_CPU` (CPU-seconds, NOT wall-clock; converted by
///   rounding up to whole seconds — sub-second precision is unavailable
///   on POSIX `setrlimit`).
/// - `wall_clock_ms` → NOT enforced here. Wall-clock enforcement requires
///   either a parent-side `setitimer` + `SIGALRM` or a timer thread that
///   `kill`s the child after the budget. Ships in PR 9a-hardening-4.
///
/// Zero values are treated as "no ceiling on this axis" (do not call
/// `setrlimit` for that resource). Recovery / replay never sets zero
/// implicitly — the workflow author either declares a non-zero limit or
/// explicitly accepts the no-ceiling shape.
///
/// # Errors
///
/// Returns marker exit codes the caller passes to `_exit` in the child:
/// 80 = mem_bytes setrlimit failed, 81 = fd_count, 82 = disk_bytes,
/// 83 = cpu_milli.
#[cfg(unix)]
pub(crate) fn apply_rlimits(ceiling: &kx_warrant::ResourceCeiling) -> Result<(), i32> {
    if ceiling.mem_bytes > 0 {
        let rlim = libc::rlimit {
            rlim_cur: ceiling.mem_bytes,
            rlim_max: ceiling.mem_bytes,
        };
        // SAFETY: setrlimit is async-signal-safe per POSIX; rlim is a
        // valid stack-allocated struct.
        if unsafe { libc::setrlimit(libc::RLIMIT_AS, &raw const rlim) } != 0 {
            return Err(80);
        }
    }
    if ceiling.fd_count > 0 {
        let rlim = libc::rlimit {
            rlim_cur: u64::from(ceiling.fd_count),
            rlim_max: u64::from(ceiling.fd_count),
        };
        // SAFETY: as above.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const rlim) } != 0 {
            return Err(81);
        }
    }
    if ceiling.disk_bytes > 0 {
        let rlim = libc::rlimit {
            rlim_cur: ceiling.disk_bytes,
            rlim_max: ceiling.disk_bytes,
        };
        // SAFETY: as above.
        if unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &raw const rlim) } != 0 {
            return Err(82);
        }
    }
    if ceiling.cpu_milli > 0 {
        // RLIMIT_CPU is CPU-seconds. Round up so a 500-ms budget translates
        // to "1 second of CPU time" rather than "0 seconds = kill on first
        // tick."
        let cpu_seconds = u64::from(ceiling.cpu_milli).div_ceil(1000);
        let rlim = libc::rlimit {
            rlim_cur: cpu_seconds,
            rlim_max: cpu_seconds,
        };
        // SAFETY: as above.
        if unsafe { libc::setrlimit(libc::RLIMIT_CPU, &raw const rlim) } != 0 {
            return Err(83);
        }
    }
    Ok(())
}
