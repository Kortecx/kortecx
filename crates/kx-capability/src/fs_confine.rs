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
