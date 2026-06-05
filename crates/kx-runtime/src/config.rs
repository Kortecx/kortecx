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

/// Default checkpoint cadence: capture a [`kx_projection::FoldCheckpoint`] every
/// 256 folded journal entries. Coarse by design so the `O(live-state)` encode +
/// fsync stays off the hot path (M2.1's fold is ~0.5µs/Mote); the canonical demo
/// (8 Motes) never checkpoints mid-run — only on graceful completion.
pub const DEFAULT_CHECKPOINT_EVERY: u64 = 256;

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
    /// Persist a discardable `FoldCheckpoint` sidecar every N folded journal
    /// entries (M2.2b). `None` disables checkpoint writing entirely (recovery
    /// still *reads* any existing sidecar — the read path is always safe). The
    /// sidecar only ever speeds up resume; it can never change the outcome.
    pub checkpoint_every: Option<u64>,
    /// Off-truth-path audit log path (R4). `Some(path)` writes a best-effort JSONL
    /// trail of the run's lifecycle, truncated fresh per run; `None` disables
    /// auditing (the byte-identity-without-overhead path). Audit is never journaled
    /// and never feeds the digest, so this never changes the run outcome.
    pub audit_log: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Parse `argv` (excluding the program name) into a [`RuntimeConfig`].
    ///
    /// Grammar: `<run|replay|digest> --journal <path> --content <dir>
    /// [--crash-at <pre-commit-stc|post-commit-vtc>] [--checkpoint-every <N>]
    /// [--audit-log <path>]`.
    /// `--crash-at` is honored only in `run` mode (a crash point in replay/digest
    /// is a config error). `--checkpoint-every 0` disables checkpoint writing;
    /// omitting it uses [`DEFAULT_CHECKPOINT_EVERY`]. `--audit-log <path>` enables
    /// the off-truth-path JSONL audit trail for `run`/`replay` (ignored by
    /// `digest`, which only prints a digest and exits).
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
        let mut checkpoint_every: Option<u64> = Some(DEFAULT_CHECKPOINT_EVERY);
        let mut audit_log: Option<PathBuf> = None;

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
                "--checkpoint-every" => {
                    let v = take_value("--checkpoint-every")?;
                    let n = v.parse::<u64>().map_err(|_| {
                        RuntimeError::Config(format!(
                            "--checkpoint-every expects a non-negative integer, got {v:?}"
                        ))
                    })?;
                    // 0 == disabled; any positive N is the cadence.
                    checkpoint_every = (n != 0).then_some(n);
                }
                "--audit-log" => audit_log = Some(PathBuf::from(take_value("--audit-log")?)),
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
            checkpoint_every,
            audit_log,
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
        // Checkpointing is on by default at the coarse cadence.
        assert_eq!(c.checkpoint_every, Some(DEFAULT_CHECKPOINT_EVERY));
        // Auditing is OFF by default (the byte-identity-without-overhead path).
        assert_eq!(c.audit_log, None);
    }

    #[test]
    fn audit_log_flag_parses_and_defaults_off() {
        let on = RuntimeConfig::from_args([
            "run",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--audit-log",
            "/tmp/audit.jsonl",
        ])
        .unwrap();
        assert_eq!(on.audit_log, Some(PathBuf::from("/tmp/audit.jsonl")));

        // Honored in replay too.
        let replay = RuntimeConfig::from_args([
            "replay",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--audit-log",
            "/tmp/a.jsonl",
        ])
        .unwrap();
        assert_eq!(replay.audit_log, Some(PathBuf::from("/tmp/a.jsonl")));

        // Absent by default.
        let off = RuntimeConfig::from_args(["run", "--journal", "/tmp/j", "--content", "/tmp/c"])
            .unwrap();
        assert_eq!(off.audit_log, None);

        // Missing value is a config error.
        let bad = RuntimeConfig::from_args([
            "run",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--audit-log",
        ]);
        assert!(matches!(bad, Err(RuntimeError::Config(_))));
    }

    #[test]
    fn checkpoint_every_parses_and_zero_disables() {
        let on = RuntimeConfig::from_args([
            "run",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--checkpoint-every",
            "2",
        ])
        .unwrap();
        assert_eq!(on.checkpoint_every, Some(2));

        let off = RuntimeConfig::from_args([
            "replay",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--checkpoint-every",
            "0",
        ])
        .unwrap();
        assert_eq!(off.checkpoint_every, None);

        let bad = RuntimeConfig::from_args([
            "run",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--checkpoint-every",
            "notanumber",
        ]);
        assert!(matches!(bad, Err(RuntimeError::Config(_))));
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
