// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags that would be
// needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! What the shim BINARY actually does when it is spawned.
//!
//! The unit tests next door cover the descriptor codec. They cannot cover the
//! part that matters at runtime: that the shim runs the script, persists exactly
//! its stdout, prints a ref the sandbox backend can parse, and fails closed on
//! every path where it might otherwise produce a plausible-looking result.
//!
//! These drive the real binary via `CARGO_BIN_EXE_*`, so nothing here is a
//! stand-in for the thing that ships.
//!
//! The environment test is a PAIR on purpose. "The variable was empty" is not
//! evidence of anything by itself — an empty read is also what a broken script,
//! a wrong variable name, or a shell quoting mistake produces. Only running the
//! same script twice, once with the pair declared and once without, separates
//! *cleared* from *never set in the first place*.

use std::fs;
use std::path::Path;
use std::process::Command;

use kx_script_runner::{hex32, result_ref_bytes, ScriptDescriptor};

/// The shim binary Cargo just built for this test.
const SHIM: &str = env!("CARGO_BIN_EXE_kx-script-runner");

/// A POSIX shell is the one interpreter every supported platform has at a fixed
/// path, so the fixtures do not need to probe for one.
const SH: &str = "/bin/sh";

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().into_owned()
    }

    /// Write a shell script and return its absolute path.
    fn script(&self, body: &str) -> String {
        let path = self.path("script.sh");
        fs::write(&path, body).unwrap();
        path
    }

    /// A descriptor running `script` with defaults a test can override.
    fn descriptor(&self, script_path: &str) -> ScriptDescriptor {
        ScriptDescriptor {
            interpreter_path: SH.into(),
            script_path: script_path.into(),
            out_path: self.path("result.bin"),
            argv: Vec::new(),
            stdin_bytes: Vec::new(),
            env: Vec::new(),
            wall_clock_ms: 10_000,
            mem_bytes: 0,
            max_output_bytes: 64 * 1024,
        }
    }

    /// Run the shim over `descriptor`; returns (success, stdout, stderr).
    fn run(&self, descriptor: &ScriptDescriptor) -> (bool, String, String) {
        let descriptor_path = self.path("descriptor.bin");
        fs::write(&descriptor_path, descriptor.encode()).unwrap();
        let out = Command::new(SHIM).arg(&descriptor_path).output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

#[test]
fn persists_the_script_output_and_prints_its_ref() {
    let fx = Fixture::new();
    let script = fx.script("printf 'hello from the sandbox'");
    let descriptor = fx.descriptor(&script);

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(ok, "shim failed: {stderr}");

    let written = fs::read(&descriptor.out_path).unwrap();
    assert_eq!(written, b"hello from the sandbox");

    // The printed ref must be the ref of what was written — this is the equality
    // the host re-checks before it commits anything.
    assert_eq!(stdout, hex32(&result_ref_bytes(&written)));
    assert_eq!(stdout.len(), 64, "the backend parses exactly 64 hex chars");
}

/// Only the ref may reach stdout. A script that writes to stderr must not
/// corrupt the bytes the backend is about to parse.
#[test]
fn a_chatty_script_cannot_corrupt_the_ref() {
    let fx = Fixture::new();
    let script = fx.script("echo 'noise on stderr' >&2; printf 'payload'");

    let (ok, stdout, _) = fx.run(&fx.descriptor(&script));
    assert!(ok);
    assert_eq!(stdout, hex32(&result_ref_bytes(b"payload")));
}

/// The PAIR that makes "the environment is cleared" a real assertion: the same
/// script, the same variable name, differing only in whether the descriptor
/// declares it.
#[test]
fn the_child_environment_is_cleared_not_inherited() {
    let fx = Fixture::new();
    let script = fx.script("printf '%s' \"$KX_TEST_SECRET\"");

    // Arm A — the parent holds the variable, the descriptor does not name it.
    let descriptor_path = fx.path("descriptor.bin");
    fs::write(&descriptor_path, fx.descriptor(&script).encode()).unwrap();
    let inherited = Command::new(SHIM)
        .arg(&descriptor_path)
        .env("KX_TEST_SECRET", "leaked-from-the-serve")
        .output()
        .unwrap();
    assert!(inherited.status.success());
    let from_parent = fs::read(fx.path("result.bin")).unwrap();
    assert!(
        from_parent.is_empty(),
        "the script read a variable the descriptor never declared: {:?}",
        String::from_utf8_lossy(&from_parent)
    );

    // Arm B — the descriptor declares it, the parent does not hold it. Same
    // script, so a non-empty read here is what proves arm A's emptiness came
    // from clearing rather than from the fixture never working at all.
    let mut declared = fx.descriptor(&script);
    declared.env = vec![("KX_TEST_SECRET".into(), "granted-by-the-runtime".into())];
    let (ok, _, stderr) = fx.run(&declared);
    assert!(ok, "shim failed: {stderr}");
    let from_descriptor = fs::read(&declared.out_path).unwrap();
    assert_eq!(from_descriptor, b"granted-by-the-runtime");
}

#[test]
fn stdin_and_argv_reach_the_script() {
    let fx = Fixture::new();
    let script = fx.script("read -r line; printf '%s|%s|%s' \"$line\" \"$1\" \"$2\"");
    let mut descriptor = fx.descriptor(&script);
    descriptor.stdin_bytes = b"from-stdin\n".to_vec();
    descriptor.argv = vec!["first".into(), "second".into()];

    let (ok, _, stderr) = fx.run(&descriptor);
    assert!(ok, "shim failed: {stderr}");
    assert_eq!(
        fs::read(&descriptor.out_path).unwrap(),
        b"from-stdin|first|second"
    );
}

/// A failing script must fail the run. Committing its partial stdout as a
/// successful result is the outcome this forbids.
#[test]
fn a_failing_script_fails_the_run_and_leaves_no_result() {
    let fx = Fixture::new();
    let script = fx.script("printf 'partial output'; echo 'it broke' >&2; exit 3");
    let descriptor = fx.descriptor(&script);

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(!ok, "a script that exited 3 was reported as success");
    assert!(stdout.is_empty(), "no ref may be printed for a failed run");
    assert!(
        stderr.contains("status 3") && stderr.contains("it broke"),
        "the failure diagnostic should name the status and the script's stderr: {stderr}"
    );
    assert!(
        !Path::new(&descriptor.out_path).exists(),
        "a failed run must not leave a result file behind"
    );
}

/// Over-cap output is REFUSED, never truncated. A truncated result reads as a
/// complete answer, and whatever consumes it cannot tell.
#[test]
fn oversized_output_is_refused_rather_than_truncated() {
    let fx = Fixture::new();
    let script = fx.script("printf '0123456789'");
    let mut descriptor = fx.descriptor(&script);
    descriptor.max_output_bytes = 4;

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(!ok, "output over the declared cap was accepted");
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("refused rather than truncated"),
        "unexpected diagnostic: {stderr}"
    );
    assert!(
        !Path::new(&descriptor.out_path).exists(),
        "no truncated result may be left behind"
    );
}

/// Exactly-at-the-cap output is fine — the refusal must trigger on overflow, not
/// on reaching the limit, or every cap would be off by one.
#[test]
fn output_exactly_at_the_cap_is_accepted() {
    let fx = Fixture::new();
    let script = fx.script("printf '0123456789'");
    let mut descriptor = fx.descriptor(&script);
    descriptor.max_output_bytes = 10;

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(ok, "output exactly at the cap was refused: {stderr}");
    assert_eq!(stdout, hex32(&result_ref_bytes(b"0123456789")));
}

#[test]
fn an_unresolvable_interpreter_fails_closed() {
    let fx = Fixture::new();
    let script = fx.script("printf 'never runs'");
    let mut descriptor = fx.descriptor(&script);
    descriptor.interpreter_path = "/nonexistent/interpreter".into();

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(!ok);
    assert!(stdout.is_empty());
    assert!(stderr.contains("could not spawn interpreter"), "{stderr}");
}

#[test]
fn a_corrupt_descriptor_fails_closed() {
    let fx = Fixture::new();
    let descriptor_path = fx.path("descriptor.bin");
    fs::write(&descriptor_path, b"not a descriptor").unwrap();

    let out = Command::new(SHIM).arg(&descriptor_path).output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn a_missing_descriptor_argument_is_a_usage_error() {
    let out = Command::new(SHIM).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

/// An empty result is a legitimate outcome (a script that prints nothing), and
/// it must still produce a well-formed ref rather than an error or a blank line.
#[test]
fn an_empty_output_still_produces_a_ref() {
    let fx = Fixture::new();
    let script = fx.script("exit 0");
    let descriptor = fx.descriptor(&script);

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(ok, "shim failed: {stderr}");
    assert_eq!(fs::read(&descriptor.out_path).unwrap(), b"");
    assert_eq!(stdout, hex32(&result_ref_bytes(b"")));
}

/// Binary output must survive byte-for-byte: the result is content-addressed, so
/// any normalization (a stray newline, a UTF-8 replacement) changes the ref.
#[test]
fn binary_output_survives_byte_for_byte() {
    let fx = Fixture::new();
    let script = fx.script("printf '\\001\\002\\377\\000\\n'");
    let descriptor = fx.descriptor(&script);

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(ok, "shim failed: {stderr}");
    let written = fs::read(&descriptor.out_path).unwrap();
    assert_eq!(written, b"\x01\x02\xff\x00\n");
    assert_eq!(stdout, hex32(&result_ref_bytes(&written)));
}

/// ★ A declared time budget must actually stop a runaway script.
///
/// The axis a fast script can never exercise: a word-count returns in
/// milliseconds, so a missing timeout and a working one look identical. Here the
/// script sleeps far past its budget, so the two are distinguishable — enforced
/// means a quick failure, ignored means the call blocks for the whole sleep and
/// then SUCCEEDS.
#[test]
fn a_runaway_script_is_stopped_at_its_budget() {
    let fx = Fixture::new();
    let script = fx.script("sleep 30; printf 'finished anyway'");
    let mut descriptor = fx.descriptor(&script);
    descriptor.wall_clock_ms = 1_500;

    let started = std::time::Instant::now();
    let (ok, stdout, stderr) = fx.run(&descriptor);
    let elapsed = started.elapsed();

    assert!(!ok, "a script that ran 20x its budget was allowed to finish");
    assert!(stdout.is_empty(), "no ref may be printed for a stopped run");
    assert!(
        stderr.contains("exceeded its 1500 ms budget"),
        "unexpected diagnostic: {stderr}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "the budget did not stop it — took {elapsed:?}, near the script's own 30s sleep"
    );
    assert!(
        !Path::new(&descriptor.out_path).exists(),
        "a stopped run must not leave a result behind"
    );
}

/// A script that finishes inside its budget is unaffected — the pair that keeps
/// the test above from passing merely because everything times out.
#[test]
fn a_script_within_its_budget_is_unaffected() {
    let fx = Fixture::new();
    let script = fx.script("printf 'quick'");
    let mut descriptor = fx.descriptor(&script);
    descriptor.wall_clock_ms = 10_000;

    let (ok, stdout, stderr) = fx.run(&descriptor);
    assert!(ok, "a prompt script was stopped anyway: {stderr}");
    assert_eq!(stdout, hex32(&result_ref_bytes(b"quick")));
}

/// A declared memory ceiling reaches the interpreter. The ceiling is set on the
/// shim and inherited across exec, so a script asking for far more than it is
/// allowed fails rather than being quietly granted it.
#[test]
fn a_memory_ceiling_is_inherited_by_the_interpreter() {
    let fx = Fixture::new();
    // Ask the shell to allocate a string far past the ceiling.
    let script = fx.script("x=$(printf 'a%.0s' $(seq 1 50000000)); printf '%s' \"${#x}\"");
    let mut descriptor = fx.descriptor(&script);
    descriptor.mem_bytes = 32 * 1024 * 1024;
    descriptor.wall_clock_ms = 20_000;

    let (ok, _, stderr) = fx.run(&descriptor);
    assert!(
        !ok,
        "a script allocating far past its {}-byte ceiling succeeded — the limit \
         did not reach the interpreter",
        descriptor.mem_bytes
    );
    assert!(!stderr.is_empty(), "a refused run should say why");
}
