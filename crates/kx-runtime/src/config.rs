//! Runtime configuration + CLI parsing for the `kx-runtime` binary.

use std::path::PathBuf;

use crate::crash::CrashPoint;
use crate::error::RuntimeError;

/// What the binary should do this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Run the demo workflow from scratch against a fresh (or empty) journal.
    Run,
    /// Recover: fold the existing on-disk journal, then drive the workflow to
    /// completion (re-reading committed Motes, never re-running them).
    Replay,
    /// Print the deterministic projection digest of the on-disk journal and
    /// exit. Used by the kill-and-replay harness to compare runs across
    /// processes / machines.
    Digest,
}

/// Resolved runtime configuration for one invocation.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Path to the on-disk SQLite journal file.
    pub journal_path: PathBuf,
    /// Directory backing the local-FS content store.
    pub content_root: PathBuf,
    /// What to do this invocation.
    pub mode: Mode,
    /// Optional deterministic crash injection (run mode only).
    pub crash_at: Option<CrashPoint>,
}

impl RuntimeConfig {
    /// Parse `argv` (excluding the program name) into a [`RuntimeConfig`].
    ///
    /// Grammar: `<run|replay|digest> --journal <path> --content <dir>
    /// [--crash-at <pre-commit-stc|post-commit-vtc>]`. `--crash-at` is honored
    /// only in `run` mode (a crash point in replay/digest is a config error).
    pub fn from_args<I, S>(args: I) -> Result<Self, RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = args.into_iter().map(Into::into);
        let mode = match args.next().as_deref() {
            Some("run") => Mode::Run,
            Some("replay") => Mode::Replay,
            Some("digest") => Mode::Digest,
            Some(other) => {
                return Err(RuntimeError::Config(format!(
                    "unknown mode {other:?} (expected run | replay | digest)"
                )))
            }
            None => {
                return Err(RuntimeError::Config(
                    "missing mode (run | replay | digest)".into(),
                ))
            }
        };

        let mut journal_path: Option<PathBuf> = None;
        let mut content_root: Option<PathBuf> = None;
        let mut crash_at: Option<CrashPoint> = None;

        while let Some(flag) = args.next() {
            let mut take_value = |name: &str| -> Result<String, RuntimeError> {
                args.next()
                    .ok_or_else(|| RuntimeError::Config(format!("{name} requires a value")))
            };
            match flag.as_str() {
                "--journal" => journal_path = Some(PathBuf::from(take_value("--journal")?)),
                "--content" => content_root = Some(PathBuf::from(take_value("--content")?)),
                "--crash-at" => {
                    let v = take_value("--crash-at")?;
                    crash_at = Some(v.parse::<CrashPoint>().map_err(RuntimeError::Config)?);
                }
                other => {
                    return Err(RuntimeError::Config(format!("unknown flag {other:?}")));
                }
            }
        }

        let journal_path =
            journal_path.ok_or_else(|| RuntimeError::Config("--journal is required".into()))?;
        let content_root =
            content_root.ok_or_else(|| RuntimeError::Config("--content is required".into()))?;

        if crash_at.is_some() && mode != Mode::Run {
            return Err(RuntimeError::Config(
                "--crash-at is only valid in `run` mode".into(),
            ));
        }

        Ok(RuntimeConfig {
            journal_path,
            content_root,
            mode,
            crash_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_with_paths_and_crash() {
        let c = RuntimeConfig::from_args([
            "run",
            "--journal",
            "/tmp/j.sqlite",
            "--content",
            "/tmp/c",
            "--crash-at",
            "post-commit-vtc",
        ])
        .unwrap();
        assert_eq!(c.mode, Mode::Run);
        assert_eq!(c.journal_path, PathBuf::from("/tmp/j.sqlite"));
        assert_eq!(c.content_root, PathBuf::from("/tmp/c"));
        assert_eq!(c.crash_at, Some(CrashPoint::PostCommitVtc));
    }

    #[test]
    fn crash_at_outside_run_is_rejected() {
        let err = RuntimeConfig::from_args([
            "replay",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--crash-at",
            "pre-commit-stc",
        ]);
        assert!(matches!(err, Err(RuntimeError::Config(_))));
    }

    #[test]
    fn missing_required_paths_are_rejected() {
        assert!(RuntimeConfig::from_args(["run", "--journal", "/tmp/j"]).is_err());
        assert!(RuntimeConfig::from_args(["run"]).is_err());
        assert!(RuntimeConfig::from_args(Vec::<String>::new()).is_err());
    }
}
