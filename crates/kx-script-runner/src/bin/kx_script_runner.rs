//! `kx-script-runner` — the body binary the platform sandbox spawns when a
//! registered script fires.
//!
//! By the time `main` runs, the sandbox is already applied: on macOS the child
//! loaded the warrant's SBPL profile between `fork` and `execvp`; on Linux
//! `bwrap` set up the namespace and bind mounts. Everything below therefore
//! executes under the caller's own filesystem, network, and resource limits —
//! this binary cannot widen them, and does not try to.
//!
//! What it adds is the translation the sandbox contract needs: a body must print
//! a 64-character hex content ref, and a script prints arbitrary bytes. See the
//! crate docs for the full contract.
//!
//! Three properties are deliberate:
//!
//! - **The environment is cleared, not filtered.** The child gets exactly the
//!   pairs the descriptor names and nothing else, so no credential or operator
//!   knob the serve happens to hold can be read by a script.
//! - **Oversized output is refused, never truncated.** A truncated result reads
//!   as a complete answer, and whatever consumes it has no way to tell.
//! - **The declared ceiling is enforced HERE.** The memory limit is applied to
//!   this process before it execs, so the interpreter inherits it, and the time
//!   budget is timed against the child directly — this process is the script's
//!   parent, so it can stop precisely the thing that overran. The host keeps an
//!   outer deadline as a backstop for a wedged shim.
//! - **Only the hex ref reaches stdout.** Every diagnostic goes to stderr, which
//!   the sandbox does not capture — so a chatty script cannot corrupt the ref
//!   the backend is about to parse.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use kx_script_runner::{hex32, result_ref_bytes, ScriptDescriptor};

/// Bound on the child's stderr kept for diagnostics. Stderr never becomes the
/// result, so this only needs to be big enough to hold a traceback.
const MAX_STDERR_BYTES: u64 = 64 * 1024;

/// Exit code for a usage error (no descriptor path given).
const EXIT_USAGE: u8 = 2;
/// Exit code for any failure after that point. The host reads the failure from
/// the non-zero status; the reason goes to stderr.
const EXIT_FAILED: u8 = 1;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(descriptor_path) = args.next() else {
        report("usage: kx-script-runner <descriptor-file>");
        return ExitCode::from(EXIT_USAGE);
    };

    match run(&descriptor_path) {
        Ok(hex) => {
            // The ONLY write to stdout: the backend parses this and nothing else.
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            if out.write_all(hex.as_bytes()).is_err() || out.flush().is_err() {
                report("could not write the result ref to stdout");
                return ExitCode::from(EXIT_FAILED);
            }
            ExitCode::SUCCESS
        }
        Err(reason) => {
            report(&reason);
            ExitCode::from(EXIT_FAILED)
        }
    }
}

/// Write a diagnostic to stderr, which the serve inherits. Failures to report
/// are ignored — there is nowhere left to report them.
fn report(message: &str) {
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    let _ = writeln!(err, "kx-script-runner: {message}");
}

/// Read the descriptor, run the script, persist its stdout, and return the hex
/// ref of the result object.
fn run(descriptor_path: &str) -> Result<String, String> {
    let raw = fs::read(descriptor_path)
        .map_err(|e| format!("could not read descriptor {descriptor_path}: {e}"))?;
    let descriptor =
        ScriptDescriptor::decode(&raw).map_err(|e| format!("could not decode descriptor: {e}"))?;

    let output = execute(&descriptor)?;
    write_atomically(&descriptor.out_path, &output)?;
    Ok(hex32(&result_ref_bytes(&output)))
}

/// Spawn `<interpreter> <script> <argv…>` and collect its stdout.
///
/// stdout is read in this thread while a helper drains stderr, so neither pipe
/// can fill and deadlock the child. Both reads are bounded.
fn execute(descriptor: &ScriptDescriptor) -> Result<Vec<u8>, String> {
    let mut command = Command::new(&descriptor.interpreter_path);
    command
        .arg(&descriptor.script_path)
        .args(&descriptor.argv)
        // Cleared, then set — the child inherits nothing.
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &descriptor.env {
        command.env(key, value);
    }

    // Put the script in its OWN process group, so stopping it stops everything it
    // started. A shell that spawns a helper (`sleep`, a pipeline stage) leaves that
    // helper holding the stdout pipe open, so killing only the direct child neither
    // ends the work nor unblocks the reader — the run would still take as long as
    // the script felt like taking.
    //
    // SAFETY: `pre_exec` runs in the forked child, where only async-signal-safe
    // calls are permitted. `setpgid` is async-signal-safe per POSIX and touches
    // only the calling process.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }

    // The memory ceiling, applied to THIS process before the fork so the
    // interpreter inherits it. Set here rather than by the host because the host
    // crate forbids unsafe, and because a limit set on the direct parent is the
    // one the child actually gets.
    apply_memory_ceiling(descriptor.mem_bytes)?;

    let mut child = command.spawn().map_err(|e| {
        format!(
            "could not spawn interpreter {}: {e}",
            descriptor.interpreter_path
        )
    })?;

    // Write stdin and drop the handle so the child sees EOF. A script that never
    // reads stdin closes the pipe early; a broken pipe here is its choice, not a
    // failure of the run.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&descriptor.stdin_bytes);
        let _ = stdin.flush();
    }

    // BOTH pipes are drained on their own threads, and the deadline is watched
    // here. Reading stdout inline would block until the child closed it — which a
    // runaway script never does — so the budget check would not be reached until
    // after the script had already finished. Killing the child closes the pipes,
    // which is what lets these threads finish.
    let limit = descriptor.max_output_bytes.saturating_add(1);
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "the child's stdout pipe was not available".to_string())?;
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let outcome = stdout.by_ref().take(limit).read_to_end(&mut buf);
        (buf, outcome.err())
    });
    let stderr_reader = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.take(MAX_STDERR_BYTES).read_to_end(&mut buf);
            buf
        })
    });

    let status = wait_within_budget(&mut child, descriptor.wall_clock_ms);
    let (collected, read_error) = out_reader.join().unwrap_or_else(|_| (Vec::new(), None));
    let stderr_bytes = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    // Report the budget failure with whatever the script managed to say first.
    let status = status.map_err(|reason| format!("{reason}{}", tail(&stderr_bytes)))?;
    if let Some(e) = read_error {
        return Err(format!("could not read the script's output: {e}"));
    }

    if !status.success() {
        return Err(format!(
            "the script exited with {}{}",
            status
                .code()
                .map_or_else(|| "a signal".to_string(), |c| format!("status {c}")),
            tail(&stderr_bytes)
        ));
    }
    if collected.len() as u64 > descriptor.max_output_bytes {
        return Err(format!(
            "the script produced more than the {} bytes its declaration allows \
             (refused rather than truncated)",
            descriptor.max_output_bytes
        ));
    }
    Ok(collected)
}

/// Apply the address-space ceiling to this process. 0 ⇒ unset.
///
/// `RLIMIT_AS` is inherited across fork+exec, so setting it here bounds the
/// interpreter and everything it spawns.
fn apply_memory_ceiling(mem_bytes: u64) -> Result<(), String> {
    if mem_bytes == 0 {
        return Ok(());
    }
    let rlim = libc::rlimit {
        rlim_cur: mem_bytes,
        rlim_max: mem_bytes,
    };
    // SAFETY: `setrlimit` takes a pointer to a fully-initialized, stack-allocated
    // `rlimit`; it is called before any thread is spawned and its only effect is
    // on this process's own limits.
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &raw const rlim) } != 0 {
        return Err(format!(
            "could not apply the {mem_bytes}-byte memory ceiling: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Wait for the script, stopping it if it outlives its budget. 0 ⇒ no budget.
///
/// Escalates SIGTERM → SIGKILL: a script that installs a handler, or is mid-write,
/// gets a chance to exit cleanly, but cannot decline to stop.
fn wait_within_budget(
    child: &mut std::process::Child,
    wall_clock_ms: u64,
) -> Result<std::process::ExitStatus, String> {
    if wall_clock_ms == 0 {
        return child
            .wait()
            .map_err(|e| format!("could not wait for the script: {e}"));
    }
    let deadline = Instant::now() + Duration::from_millis(wall_clock_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    stop(child);
                    return Err(format!(
                        "the script exceeded its {wall_clock_ms} ms budget and was stopped"
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(format!("could not wait for the script: {e}")),
        }
    }
}

/// Terminate the script's whole process GROUP: ask, then insist.
///
/// The group, not the process — see the `setpgid` note in `execute`. A negative
/// pid signals the group led by that pid.
fn stop(child: &mut std::process::Child) {
    #[allow(clippy::cast_possible_wrap)]
    let pgid = child.id() as libc::pid_t;
    // SAFETY: signalling a process group this process created is safe; a failed
    // signal (the group is already gone) is reported through the return value,
    // deliberately ignored because the outcome is the same either way.
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }
    let grace = Instant::now() + Duration::from_millis(500);
    while Instant::now() < grace {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: as above — the insistent half.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.wait();
}

/// Render captured stderr as a trailing diagnostic, or nothing when it is empty.
fn tail(stderr_bytes: &[u8]) -> String {
    if stderr_bytes.is_empty() {
        return String::new();
    }
    format!(
        "; stderr: {}",
        String::from_utf8_lossy(stderr_bytes).trim_end()
    )
}

/// Write `bytes` to `path` via a sibling temporary file and a rename.
///
/// The host re-hashes whatever it reads back, so a partial write is already
/// caught. Renaming into place additionally means a reader never observes a
/// half-written file at all.
fn write_atomically(path: &str, bytes: &[u8]) -> Result<(), String> {
    let temp_path = format!("{path}.partial");
    let mut file = fs::File::create(&temp_path)
        .map_err(|e| format!("could not create {temp_path}: {e}"))?;
    file.write_all(bytes)
        .map_err(|e| format!("could not write {temp_path}: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("could not flush {temp_path}: {e}"))?;
    drop(file);
    fs::rename(&temp_path, path).map_err(|e| {
        let _ = fs::remove_file(&temp_path);
        format!("could not move {temp_path} into place at {path}: {e}")
    })
}
