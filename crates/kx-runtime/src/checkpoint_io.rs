//! Durable, crash-safe persistence for the discardable [`FoldCheckpoint`]
//! (M2.2b — the live wiring of the M2.2 checkpoint).
//!
//! ## Why this is safe by construction
//!
//! A [`FoldCheckpoint`] is **never authoritative** (D92(b)): on *any* anomaly —
//! a missing, truncated, corrupt, stale, or wrong-run sidecar — recovery falls
//! back to a full fold and is bit-identical regardless. So this module's only job
//! is to make the *common-case* restart fast without ever being able to corrupt
//! recovery. Two properties carry that:
//!
//! - **Atomic replace.** The bytes are written to a fixed sibling temp file, then
//!   `rename(2)`'d over the sidecar. A crash mid-write leaves either the old
//!   complete checkpoint or the new complete checkpoint, never a torn one — so a
//!   crash can never *clobber a good checkpoint with garbage* (which would
//!   silently degrade us to a full fold exactly when resume matters most). The
//!   temp lives in the **same directory** as the target so the rename stays within
//!   one filesystem (a cross-device rename is `EXDEV`, non-atomic).
//! - **Best-effort durability via fsync.** We `sync_all` the temp file before the
//!   rename and `fsync` the parent directory after it (Unix), so the checkpoint
//!   and its directory entry survive power loss. These are *durability of the
//!   optimization*, not correctness: a checkpoint that renamed but didn't survive
//!   a power cut just means the next restart full-folds — still correct.
//!
//! ## Trust boundary
//!
//! The sidecar lives in the journal's own data directory, with the journal's file
//! permissions. Anyone who can write `<journal>.ckpt` can already write the
//! authoritative journal, so the sidecar adds **no new attacker** under the
//! single-node deployment model. The one residual — a *forged-but-self-consistent*
//! sidecar that seeds a wrong base state the tail folds onto — is closed by the
//! journaled digest seal (M2.2c), which anchors the post-recovery `state_digest`
//! to a digest committed *in* the journal (the trust root). Until then this is the
//! standard checkpoint-sidecar posture.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use kx_projection::FoldCheckpoint;

/// The sidecar path for a journal: the journal path with `.ckpt` **appended**
/// (e.g. `run.sqlite` → `run.sqlite.ckpt`). Appending (not replacing the
/// extension) keeps the full journal name visible and avoids collisions between
/// two journals that differ only by extension.
#[must_use]
pub fn sidecar_path(journal_path: &Path) -> PathBuf {
    append_suffix(journal_path, ".ckpt")
}

/// Atomically (re)write `bytes` to `path`: write a fixed sibling temp, fsync it,
/// `rename` over `path`, then fsync the parent directory.
///
/// A single fixed temp name (`<path>.tmp`) is reused each call (truncate-on-open),
/// bounding orphaned temp files to at most one — left only by a crash mid-write,
/// and overwritten by the next write.
///
/// # Errors
/// Any I/O failure from create / write / fsync / rename. The caller treats a
/// failure as non-fatal (the checkpoint is an optimization) and logs it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = append_suffix(path, ".tmp");
    {
        // `create` truncates a stale temp left by a previous crashed write.
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        // fsync the temp's data + metadata before it becomes the live sidecar.
        f.sync_all()?;
    }
    // Atomic on POSIX within one filesystem (the temp is a sibling of `path`).
    fs::rename(&tmp, path)?;
    // Make the rename's directory entry durable. Best-effort: the checkpoint is
    // never authoritative, so a lost dirent only costs a full fold next restart.
    sync_parent_dir(path);
    Ok(())
}

/// Read and parse a checkpoint sidecar, returning `None` for **every** failure —
/// the file is absent (first run), unreadable, truncated, corrupt, or carries an
/// unknown version/codec. All of these mean the same thing to recovery: *full
/// fold*. The parse goes through [`FoldCheckpoint::from_bytes`], which is
/// panic-free and fully validating, so a hostile blob can only ever be discarded.
#[must_use]
pub fn read_checkpoint(path: &Path) -> Option<FoldCheckpoint> {
    // A missing sidecar (the common first-run case) is silent.
    let Ok(bytes) = fs::read(path) else {
        return None;
    };
    match FoldCheckpoint::from_bytes(&bytes) {
        Ok(cp) => Some(cp),
        // A present-but-unparseable sidecar (corrupt/truncated/tampered/old
        // version) is worth a warning — it signals disk corruption or tampering,
        // not the normal absent case — but recovery still safely full-folds.
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                bytes = bytes.len(),
                %error,
                "checkpoint sidecar present but unparseable; discarding (full fold)"
            );
            None
        }
    }
}

/// Append a literal suffix to a path's full name (not the OS "extension"), staying
/// in the same directory.
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) {
    let parent = match path.parent() {
        // An empty parent means "the current directory" — fsync `.` instead.
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    if let Ok(dir) = File::open(&parent) {
        // Best-effort — a failure here cannot affect recovery correctness.
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) {
    // Directory fsync is not portable; skipped on non-Unix. The checkpoint is
    // never authoritative, so the weaker durability guarantee is still safe.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_appends_ckpt() {
        assert_eq!(
            sidecar_path(Path::new("/tmp/run.sqlite")),
            PathBuf::from("/tmp/run.sqlite.ckpt")
        );
    }

    #[test]
    fn write_then_read_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.sqlite.ckpt");
        write_atomic(&path, b"hello-checkpoint-bytes").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello-checkpoint-bytes");
    }

    #[test]
    fn read_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_checkpoint(&dir.path().join("absent.ckpt")).is_none());
    }

    #[test]
    fn read_corrupt_bytes_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.ckpt");
        // Too short to be a valid envelope -> from_bytes Err -> None.
        write_atomic(&path, b"\x01\x02\x03").unwrap();
        assert!(read_checkpoint(&path).is_none());
    }

    #[test]
    fn rewrite_overwrites_and_leaves_at_most_one_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.ckpt");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        // The temp must not survive a successful write (renamed away).
        assert!(!append_suffix(&path, ".tmp").exists());
    }
}
