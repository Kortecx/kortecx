//! [`FsReadCapability`] — the host-side READ-INTO-CAS filesystem tool
//! (fs-read@1, D155 Phase-2). Extends fs-list from "list names" to "read a
//! confined file's bytes": a model in a ReAct turn (or a snapshot-in) proposes
//! `{"path": <subpath>}`; this reads that ONE regular file's bytes (confined +
//! byte-bounded) and returns them. The broker's stage-then-commit path
//! content-addresses the bytes into the SAME content store, so the committed
//! `result_ref` **IS** the file's [`kx_content::ContentRef`] (dedup-for-free) —
//! the `{path → ref}` join a branch manifest records (D155 snapshot-in).
//!
//! # Security (the host-read surface — five layers, fail-closed)
//!
//! Identical posture to fs-list, plus a per-file byte cap:
//! 1. The server-issued recipe warrant grants a read-only root (`KX_SERVE_FS_ROOT`).
//! 2. The tool declares `fs_scope_required = {<root>: ReadOnly}`.
//! 3. Dispatch sets `request.fs_scope = declared ∩ warrant`.
//! 4. The broker `precheck` enforces `request.fs_scope ⊆ warrant.fs_scope`.
//! 5. **Here**: the model's path is canonicalized + confined to the granted
//!    mount (`..`/symlink escapes refused — shared with fs-list via
//!    [`crate::fs_confine`]), AND a **byte cap gates the read** (a metadata
//!    size-check BEFORE the read — never an unbounded host read / DoS).
//!
//! Default-OFF: nothing registers `fs-read` unless `KX_SERVE_FS_ROOT` is set, so
//! the runtime is byte-identical to today when unconfigured.

use kx_mote::{EffectPattern, ToolName, ToolVersion};
use serde::Deserialize;

use crate::capability::Capability;
use crate::errors::CapabilityFailureReason;
use crate::fs_confine::{first_readable_mount, resolve_confined_file};
use crate::request::EffectRequest;

/// fs-read observes the world (a read) and commits its bytes as a `result_ref` —
/// the same stage-then-commit content-addressed path fs-list / echo use.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// Default per-file byte ceiling (a read above this is refused — the DoS guard).
/// Operator-overridable at registration via [`FsReadCapability::with_max_bytes`].
pub const DEFAULT_MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

/// The bundled read-into-CAS filesystem capability (`fs-read@1`).
pub struct FsReadCapability {
    name: ToolName,
    version: ToolVersion,
    max_bytes: u64,
}

impl FsReadCapability {
    /// Construct `fs-read@1` with the default per-file byte cap
    /// ([`DEFAULT_MAX_READ_BYTES`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_bytes(DEFAULT_MAX_READ_BYTES)
    }

    /// Construct `fs-read@1` with an explicit per-file byte cap.
    #[must_use]
    pub fn with_max_bytes(max_bytes: u64) -> Self {
        Self {
            name: ToolName("fs-read".into()),
            version: ToolVersion("1".into()),
            max_bytes,
        }
    }
}

impl Default for FsReadCapability {
    fn default() -> Self {
        Self::new()
    }
}

/// The model's proposed argument bag — the REQUIRED subpath to read (the typed
/// `inputSchema` validated this upstream). `deny_unknown_fields` is fail-closed
/// against smuggled keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
}

impl Capability for FsReadCapability {
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
        // (1) The granted read root (already proven ⊆ warrant by the broker
        // precheck). No grant ⇒ fail-closed.
        let root = first_readable_mount(&request.fs_scope).ok_or_else(|| {
            CapabilityFailureReason::Other("fs-read: no readable mount in fs_scope".to_string())
        })?;

        // (2) Parse the REQUIRED subpath (fs-read has no "read the root" default).
        let sub = parse_path_arg(&request.payload)?;

        // (3) Resolve + canonicalize + CONFINE to a regular file in the root
        // (`..`/symlink escapes refused — shared with fs-list).
        let target = resolve_confined_file(root, Some(&sub))?;

        // (4) Byte cap: a metadata size-check BEFORE the read (no unbounded read).
        let meta = std::fs::metadata(&target)
            .map_err(|e| CapabilityFailureReason::Other(format!("fs-read: metadata: {e}")))?;
        if meta.len() > self.max_bytes {
            return Err(CapabilityFailureReason::Other(format!(
                "fs-read: file is {} bytes, exceeds the {}-byte cap",
                meta.len(),
                self.max_bytes
            )));
        }

        // (5) Read the raw bytes — the broker content-addresses them into CAS;
        // the committed result_ref IS the file's ContentRef.
        std::fs::read(&target)
            .map_err(|e| CapabilityFailureReason::Other(format!("fs-read: read: {e}")))
    }
}

/// Parse the model's `{"path": <subpath>}` arg. fs-read REQUIRES a non-empty
/// path (unlike fs-list's optional subpath). Empty payload / missing key /
/// empty value / unknown key ⇒ fail-closed.
fn parse_path_arg(payload: &[u8]) -> Result<String, CapabilityFailureReason> {
    if payload.is_empty() {
        return Err(CapabilityFailureReason::Other(
            "fs-read: missing required 'path' arg".to_string(),
        ));
    }
    let args: ReadArgs = serde_json::from_slice(payload)
        .map_err(|e| CapabilityFailureReason::Other(format!("fs-read: bad args: {e}")))?;
    if args.path.is_empty() {
        return Err(CapabilityFailureReason::Other(
            "fs-read: empty 'path' arg".to_string(),
        ));
    }
    Ok(args.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::{FsMode, FsScope, NetScope, SecretScope};
    use std::collections::BTreeMap;
    use std::path::Path;

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
    fn reads_a_confined_file_raw_bytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), b"hello world").unwrap();
        let cap = FsReadCapability::new();
        let out = cap
            .invoke(&req(br#"{"path":"hello.txt"}"#, dir.path()))
            .unwrap();
        // RAW file bytes (not a JSON wrapper) — so result_ref == ContentRef::of(file).
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn refuses_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let cap = FsReadCapability::new();
        assert!(cap.invoke(&req(br#"{"path":"sub"}"#, dir.path())).is_err());
    }

    #[test]
    fn refuses_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsReadCapability::new();
        let err = cap.invoke(&req(br#"{"path":"../../etc/hosts"}"#, dir.path()));
        assert!(err.is_err(), "path escape must be refused");
    }

    #[test]
    fn refuses_missing_path_arg() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsReadCapability::new();
        // empty payload + explicit empty/absent path both fail closed.
        assert!(cap.invoke(&req(b"", dir.path())).is_err());
        assert!(cap.invoke(&req(br#"{"path":""}"#, dir.path())).is_err());
        assert!(cap.invoke(&req(br"{}", dir.path())).is_err());
    }

    #[test]
    fn refuses_unknown_arg_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"x").unwrap();
        let cap = FsReadCapability::new();
        assert!(cap
            .invoke(&req(br#"{"path":"f","evil":"x"}"#, dir.path()))
            .is_err());
    }

    #[test]
    fn refuses_over_the_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big"), vec![0u8; 1024]).unwrap();
        let cap = FsReadCapability::with_max_bytes(512);
        let err = cap.invoke(&req(br#"{"path":"big"}"#, dir.path()));
        assert!(err.is_err(), "a file over the byte cap must be refused");
        // a file at/under the cap is fine.
        std::fs::write(dir.path().join("small"), vec![0u8; 512]).unwrap();
        assert!(cap.invoke(&req(br#"{"path":"small"}"#, dir.path())).is_ok());
    }

    #[test]
    fn refuses_when_no_fs_grant() {
        let cap = FsReadCapability::new();
        let r = EffectRequest {
            payload: br#"{"path":"x"}"#.to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
            secret_scope: SecretScope::None,
        };
        assert!(cap.invoke(&r).is_err());
    }
}
