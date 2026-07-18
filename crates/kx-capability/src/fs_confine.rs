//! Shared, airtight filesystem-confinement primitives for the host-side read
//! capabilities (`fs-list@1`, `fs-read@1`). Extracted from `fs_list.rs` (D155
//! Phase-1) so Phase-2's `fs-read@1` reuses the EXACT canonicalize + in-mount
//! prefix-check — `..` / symlink escapes are refused in ONE place (behaviour
//! preserved; `fs_list`'s tests pin it).
//!
//! The kind check (directory vs regular file) is the only axis that differs
//! between the two tools, so it is the caller's choice via
//! [`resolve_confined_dir`] (fs-list) / [`resolve_confined_file`] (fs-read);
//! the prefix-check itself lives once in [`resolve_in_root`].

use std::path::{Path, PathBuf};

use kx_warrant::{FsMode, FsScope};

use crate::errors::CapabilityFailureReason;

/// The first mount in `fs` granting read access (`ReadOnly`/`ReadWrite`), in
/// canonical `BTreeMap` order (deterministic). `None` ⇒ no readable grant.
pub(crate) fn first_readable_mount(fs: &FsScope) -> Option<&PathBuf> {
    fs.mounts
        .iter()
        .find(|(_, mode)| matches!(mode, FsMode::ReadOnly | FsMode::ReadWrite))
        .map(|(path, _)| path)
}

/// The first mount in `fs` granting WRITE access (`ReadWrite` only — a read-only
/// grant never satisfies fs-write), in canonical `BTreeMap` order. `None` ⇒ no
/// writable grant, which fs-write treats as fail-closed.
pub(crate) fn first_writable_mount(fs: &FsScope) -> Option<&PathBuf> {
    fs.mounts
        .iter()
        .find(|(_, mode)| matches!(mode, FsMode::ReadWrite))
        .map(|(path, _)| path)
}

/// Resolve + confine a subpath for WRITING (`fs-write@1`). Unlike the read
/// variants, the target file need NOT exist yet (a create is the common case),
/// so `canonicalize()` on the target itself would spuriously fail. Instead the
/// airtight canonicalize + in-root prefix-check is applied to the target's
/// PARENT directory (which MUST already exist inside the granted root — fs-write
/// never `mkdir -p`s a new tree), plus a simple-leaf check, so `..` / symlink
/// escapes are refused with the exact same posture as the read path. If the
/// target already exists it must be a regular file (a directory or an escaping
/// symlink at the leaf is refused fail-closed).
pub(crate) fn resolve_confined_writable_path(
    root: &Path,
    sub: &str,
) -> Result<PathBuf, CapabilityFailureReason> {
    let rel = Path::new(sub);
    let candidate = if rel.is_absolute() {
        PathBuf::from(sub)
    } else {
        root.join(sub)
    };
    // The leaf must be a normal filename — a path ending in ""/"."/".."/"/" has
    // no `file_name`, so this refuses "write the directory itself" fail-closed.
    let file_name = candidate.file_name().ok_or_else(|| {
        CapabilityFailureReason::Other("fs-confine: write target has no file name".to_string())
    })?;
    let parent = candidate.parent().ok_or_else(|| {
        CapabilityFailureReason::Other("fs-confine: write target has no parent".to_string())
    })?;
    // The PARENT must exist + canonicalize (resolving every `..`/symlink in the
    // chain) + sit inside the canonical root — the single airtight prefix-check.
    let canon_parent = parent.canonicalize().map_err(|e| {
        CapabilityFailureReason::Other(format!("fs-confine: cannot resolve parent dir: {e}"))
    })?;
    let canon_root = root.canonicalize().map_err(|e| {
        CapabilityFailureReason::Other(format!("fs-confine: cannot resolve root: {e}"))
    })?;
    if !canon_parent.starts_with(&canon_root) {
        return Err(CapabilityFailureReason::Other(
            "fs-confine: write path escapes the granted root".to_string(),
        ));
    }
    if !canon_parent.is_dir() {
        return Err(CapabilityFailureReason::Other(
            "fs-confine: write parent is not a directory".to_string(),
        ));
    }
    let target = canon_parent.join(file_name);
    // If the leaf already exists, refuse a directory, and refuse a symlink whose
    // target escapes the root (a symlink planted at the leaf can't be a write-out).
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            let canon = target.canonicalize().map_err(|e| {
                CapabilityFailureReason::Other(format!(
                    "fs-confine: cannot resolve target symlink: {e}"
                ))
            })?;
            if !canon.starts_with(&canon_root) {
                return Err(CapabilityFailureReason::Other(
                    "fs-confine: write target symlink escapes the granted root".to_string(),
                ));
            }
        } else if meta.is_dir() {
            return Err(CapabilityFailureReason::Other(
                "fs-confine: write target is a directory".to_string(),
            ));
        }
    }
    Ok(target)
}

/// Resolve a (possibly absolute) subpath against `root`, canonicalize it (which
/// resolves `..` + symlinks), and confine it inside the canonical root. Any
/// escape, or a non-existent target, is refused fail-closed. This is the single
/// airtight prefix-check; the caller adds the dir/file kind check.
fn resolve_in_root(root: &Path, sub: Option<&str>) -> Result<PathBuf, CapabilityFailureReason> {
    let candidate = match sub {
        Some(s) if Path::new(s).is_absolute() => PathBuf::from(s),
        Some(s) => root.join(s),
        None => root.to_path_buf(),
    };
    let canon = candidate.canonicalize().map_err(|e| {
        CapabilityFailureReason::Other(format!("fs-confine: cannot resolve path: {e}"))
    })?;
    let canon_root = root.canonicalize().map_err(|e| {
        CapabilityFailureReason::Other(format!("fs-confine: cannot resolve root: {e}"))
    })?;
    if !canon.starts_with(&canon_root) {
        return Err(CapabilityFailureReason::Other(
            "fs-confine: path escapes the granted root".to_string(),
        ));
    }
    Ok(canon)
}

/// Resolve + confine a subpath that MUST be a directory (the fs-list contract).
pub(crate) fn resolve_confined_dir(
    root: &Path,
    sub: Option<&str>,
) -> Result<PathBuf, CapabilityFailureReason> {
    let canon = resolve_in_root(root, sub)?;
    if !canon.is_dir() {
        return Err(CapabilityFailureReason::Other(
            "fs-confine: path is not a directory".to_string(),
        ));
    }
    Ok(canon)
}

/// Resolve + confine a subpath that MUST be a regular file (the fs-read
/// contract). Exported so the D155 snapshot-in path (`kx-gateway`) reuses the
/// EXACT canonicalize + prefix-check rather than hand-rolling confinement.
pub fn resolve_confined_file(
    root: &Path,
    sub: Option<&str>,
) -> Result<PathBuf, CapabilityFailureReason> {
    let canon = resolve_in_root(root, sub)?;
    if !canon.is_file() {
        return Err(CapabilityFailureReason::Other(
            "fs-confine: path is not a regular file".to_string(),
        ));
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_variant_confines_and_kind_checks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        // a directory resolves; a file is rejected by the dir kind-check.
        assert!(resolve_confined_dir(dir.path(), Some("sub")).is_ok());
        assert!(resolve_confined_dir(dir.path(), Some("f.txt")).is_err());
    }

    #[test]
    fn file_variant_confines_and_kind_checks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        // a regular file resolves; a directory is rejected by the file kind-check.
        assert!(resolve_confined_file(dir.path(), Some("f.txt")).is_ok());
        assert!(resolve_confined_file(dir.path(), Some("sub")).is_err());
    }

    #[test]
    fn both_variants_refuse_dotdot_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_confined_dir(dir.path(), Some("../../etc")).is_err());
        assert!(resolve_confined_file(dir.path(), Some("../../etc/hosts")).is_err());
    }

    #[test]
    fn writable_path_allows_a_new_file_in_the_root() {
        let dir = tempfile::tempdir().unwrap();
        // The target does NOT exist yet — a create must resolve (parent is the root).
        let target = resolve_confined_writable_path(dir.path(), "note.txt").unwrap();
        assert_eq!(target, dir.path().canonicalize().unwrap().join("note.txt"));
    }

    #[test]
    fn writable_path_allows_a_new_file_in_an_existing_subdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("out")).unwrap();
        assert!(resolve_confined_writable_path(dir.path(), "out/report.md").is_ok());
    }

    #[test]
    fn writable_path_refuses_a_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        // fs-write never `mkdir -p`s — a write into a non-existent subtree fails closed.
        assert!(resolve_confined_writable_path(dir.path(), "nope/report.md").is_err());
    }

    #[test]
    fn writable_path_refuses_dotdot_and_absolute_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_confined_writable_path(dir.path(), "../evil.txt").is_err());
        assert!(resolve_confined_writable_path(dir.path(), "/etc/evil.txt").is_err());
    }

    #[test]
    fn writable_path_refuses_writing_over_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("adir")).unwrap();
        assert!(resolve_confined_writable_path(dir.path(), "adir").is_err());
    }

    #[test]
    fn writable_path_refuses_a_symlink_leaf_escaping_the_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, b"top secret").unwrap();
        let link = root.path().join("escape");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, &link).unwrap();
            assert!(
                resolve_confined_writable_path(root.path(), "escape").is_err(),
                "a write through a symlink escaping the root must be refused"
            );
        }
        let _ = link;
    }

    #[test]
    fn first_writable_mount_ignores_read_only_grants() {
        use std::collections::BTreeMap;
        let ro = FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/ro"), FsMode::ReadOnly)]),
        };
        assert!(first_writable_mount(&ro).is_none());
        let rw = FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/rw"), FsMode::ReadWrite)]),
        };
        assert_eq!(first_writable_mount(&rw), Some(&PathBuf::from("/rw")));
    }

    #[test]
    fn file_variant_refuses_symlink_escape() {
        // A symlink inside the root pointing OUTSIDE it: canonicalize resolves the
        // link target, the prefix-check then refuses it (airtight).
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, b"top secret").unwrap();
        let link = root.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        #[cfg(unix)]
        assert!(
            resolve_confined_file(root.path(), Some("escape")).is_err(),
            "a symlink escaping the root must be refused"
        );
        let _ = link;
    }
}
