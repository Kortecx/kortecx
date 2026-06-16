//! [`FsListCapability`] — the first real, host-side, READ-ONLY filesystem tool
//! (fs-list@1, PR-6a / D155 Phase 1). A model in a ReAct turn proposes a
//! `{"path": <subpath>}` arg; this lists that directory's immediate entries
//! (names + kind + size — NEVER file contents) as a bounded, deterministic JSON
//! payload, committed as the Observation Mote's `result_ref` (R49: re-decoded on
//! replay, never re-listed).
//!
//! # Security (the new host-read surface — five layers, fail-closed)
//!
//! 1. The server-issued recipe warrant grants a read-only root (`KX_SERVE_FS_ROOT`).
//! 2. The tool declares `fs_scope_required = {<root>: ReadOnly}`.
//! 3. Dispatch sets `request.fs_scope = declared ∩ warrant` (the seam this PR opens).
//! 4. The broker `precheck` enforces `request.fs_scope ⊆ warrant.fs_scope`.
//! 5. **Here**: the model's path is canonicalized + confined to a granted mount
//!    (`..`/symlink escapes refused). Names-only, entry/byte-bounded.
//!
//! Default-OFF: nothing registers `fs-list` unless `KX_SERVE_FS_ROOT` is set, so
//! the runtime is byte-identical to today (echo-only) when unconfigured.

use std::path::{Path, PathBuf};

use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_warrant::{FsMode, FsScope};
use serde::{Deserialize, Serialize};

use crate::capability::Capability;
use crate::errors::CapabilityFailureReason;
use crate::request::EffectRequest;

/// fs-list observes the world (a read) and commits its bytes as a `result_ref` —
/// the same stage-then-commit content-addressed path echo uses.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// The max directory entries returned in one listing (excess ⇒ `truncated`).
const MAX_ENTRIES: usize = 1000;

/// The bundled read-only filesystem listing capability (`fs-list@1`).
pub struct FsListCapability {
    name: ToolName,
    version: ToolVersion,
}

impl FsListCapability {
    /// Construct the `fs-list@1` capability.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: ToolName("fs-list".into()),
            version: ToolVersion("1".into()),
        }
    }
}

impl Default for FsListCapability {
    fn default() -> Self {
        Self::new()
    }
}

/// The model's proposed argument bag (the typed `inputSchema` validated this
/// upstream; we only need the optional subpath). `deny_unknown_fields` is
/// fail-closed against smuggled keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    path: Option<String>,
}

/// One directory entry (names + kind + size — never contents).
#[derive(Serialize)]
struct Entry {
    name: String,
    kind: &'static str, // "file" | "dir" | "symlink" | "other"
    size: u64,
}

/// The committed listing payload.
#[derive(Serialize)]
struct Listing {
    /// The requested subpath (echoed; never the absolute host path — no leak).
    path: String,
    entries: Vec<Entry>,
    /// `true` iff more than [`MAX_ENTRIES`] entries existed (the rest are cut).
    truncated: bool,
}

impl Capability for FsListCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn version(&self) -> &ToolVersion {
        &self.version
    }

    fn supported_patterns(&self) -> &[EffectPattern] {
        PATTERNS
    }

    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        // (1) The granted read root = the first readable mount in fs_scope (already
        // proven ⊆ warrant by the broker precheck). No grant ⇒ fail-closed.
        let root = first_readable_mount(&request.fs_scope).ok_or_else(|| {
            CapabilityFailureReason::Other("fs-list: no readable mount in fs_scope".to_string())
        })?;

        // (2) Parse the model's optional subpath (empty args ⇒ list the root).
        let sub = parse_path_arg(&request.payload)?;

        // (3) Resolve + canonicalize + CONFINE to the granted root (reject
        // `..`/symlink escapes — airtight prefix check on canonical paths).
        let target = resolve_confined(root, sub.as_deref())?;

        // (4) Bounded, deterministic (name-sorted) listing — names + kind + size.
        let entries_path = sub.unwrap_or_else(|| ".".to_string());
        let listing = list_dir(&target, entries_path)?;

        serde_json::to_vec(&listing)
            .map_err(|e| CapabilityFailureReason::Other(format!("fs-list: encode: {e}")))
    }
}

/// The first mount in `fs` granting read access (`ReadOnly`/`ReadWrite`), in
/// canonical `BTreeMap` order (deterministic).
fn first_readable_mount(fs: &FsScope) -> Option<&PathBuf> {
    fs.mounts
        .iter()
        .find(|(_, mode)| matches!(mode, FsMode::ReadOnly | FsMode::ReadWrite))
        .map(|(path, _)| path)
}

/// Parse the model's `{"path": <subpath>}` arg. Empty payload ⇒ `None` (list the
/// root). Malformed JSON or an unknown key ⇒ fail-closed.
fn parse_path_arg(payload: &[u8]) -> Result<Option<String>, CapabilityFailureReason> {
    if payload.is_empty() {
        return Ok(None);
    }
    let args: ListArgs = serde_json::from_slice(payload)
        .map_err(|e| CapabilityFailureReason::Other(format!("fs-list: bad args: {e}")))?;
    Ok(args.path.filter(|p| !p.is_empty()))
}

/// Resolve a (possibly absolute) subpath against `root`, canonicalize it (which
/// resolves `..` + symlinks), and confine it inside the canonical root. Any
/// escape, or a non-existent / non-directory target, is refused fail-closed.
fn resolve_confined(root: &Path, sub: Option<&str>) -> Result<PathBuf, CapabilityFailureReason> {
    let candidate = match sub {
        Some(s) if Path::new(s).is_absolute() => PathBuf::from(s),
        Some(s) => root.join(s),
        None => root.to_path_buf(),
    };
    let canon = candidate.canonicalize().map_err(|e| {
        CapabilityFailureReason::Other(format!("fs-list: cannot resolve path: {e}"))
    })?;
    let canon_root = root.canonicalize().map_err(|e| {
        CapabilityFailureReason::Other(format!("fs-list: cannot resolve root: {e}"))
    })?;
    if !canon.starts_with(&canon_root) {
        return Err(CapabilityFailureReason::Other(
            "fs-list: path escapes the granted root".to_string(),
        ));
    }
    if !canon.is_dir() {
        return Err(CapabilityFailureReason::Other(
            "fs-list: path is not a directory".to_string(),
        ));
    }
    Ok(canon)
}

/// Read a directory's immediate entries (names + kind + size; NO contents),
/// sorted by name, bounded to [`MAX_ENTRIES`].
fn list_dir(dir: &Path, echo_path: String) -> Result<Listing, CapabilityFailureReason> {
    let rd = std::fs::read_dir(dir)
        .map_err(|e| CapabilityFailureReason::Other(format!("fs-list: read_dir: {e}")))?;
    let mut entries: Vec<Entry> = Vec::new();
    for item in rd {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let (kind, size) = match item.metadata() {
            Ok(md) => {
                let kind = if md.is_dir() {
                    "dir"
                } else if md.is_file() {
                    "file"
                } else if md.file_type().is_symlink() {
                    "symlink"
                } else {
                    "other"
                };
                let size = if md.is_file() { md.len() } else { 0 };
                (kind, size)
            }
            Err(_) => ("other", 0),
        };
        entries.push(Entry { name, kind, size });
    }
    // Deterministic order (the projection/replay re-reads the committed bytes;
    // determinism keeps a re-list — were one to ever happen — byte-stable).
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    Ok(Listing {
        path: echo_path,
        entries,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::EffectPattern;
    use kx_warrant::{FsMode, FsScope, NetScope, SecretScope};
    use std::collections::BTreeMap;

    fn req(payload: &[u8], root: &Path) -> EffectRequest {
        let mut mounts = BTreeMap::new();
        mounts.insert(root.to_path_buf(), FsMode::ReadOnly);
        EffectRequest {
            payload: payload.to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope { mounts },
            secret_scope: SecretScope::None,
        }
    }

    #[test]
    fn lists_a_directory_sorted_names_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), b"hello").unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let cap = FsListCapability::new();
        let out = cap.invoke(&req(b"", dir.path())).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let names: Vec<&str> = v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "sub"]);
        // names + kind + size only — never contents.
        assert_eq!(v["entries"][1]["kind"], "file");
        assert_eq!(v["entries"][1]["size"], 5); // b.txt = "hello"
        assert_eq!(v["entries"][2]["kind"], "dir");
        assert_eq!(v["truncated"], false);
    }

    #[test]
    fn lists_a_subdir_via_path_arg() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("x"), b"").unwrap();
        let cap = FsListCapability::new();
        let out = cap.invoke(&req(br#"{"path":"sub"}"#, dir.path())).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["entries"][0]["name"], "x");
    }

    #[test]
    fn refuses_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsListCapability::new();
        // `..` would climb above the granted root — canonicalize + prefix-check refuses.
        let err = cap.invoke(&req(br#"{"path":"../../etc"}"#, dir.path()));
        assert!(err.is_err(), "path escape must be refused");
    }

    #[test]
    fn refuses_when_no_fs_grant() {
        let cap = FsListCapability::new();
        let r = EffectRequest {
            payload: b"".to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
            secret_scope: SecretScope::None,
        };
        assert!(cap.invoke(&r).is_err());
    }

    #[test]
    fn refuses_unknown_arg_key() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsListCapability::new();
        assert!(cap.invoke(&req(br#"{"evil":"x"}"#, dir.path())).is_err());
    }
}
