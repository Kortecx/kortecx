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
//! - **Only the hex ref reaches stdout.** Every diagnostic goes to stderr, which
//!   the sandbox does not capture — so a chatty script cannot corrupt the ref
//!   the backend is about to parse.

use std::fs;
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

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

    let stderr_reader = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.take(MAX_STDERR_BYTES).read_to_end(&mut buf);
            buf
        })
    });

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "the child's stdout pipe was not available".to_string())?;
    // Read one byte beyond the cap so an overflow is detectable rather than
    // silently clipped at exactly the limit.
    let limit = descriptor.max_output_bytes.saturating_add(1);
    let mut collected = Vec::new();
    let read_result = stdout.take(limit).read_to_end(&mut collected);

    let status = child
        .wait()
        .map_err(|e| format!("could not wait for the script: {e}"))?;
    let stderr_bytes = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    read_result.map_err(|e| format!("could not read the script's output: {e}"))?;

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
