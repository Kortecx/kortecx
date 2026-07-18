//! [`FsWriteCapability`] — the host-side WRITE-A-CONFINED-FILE filesystem tool
//! (fs-write@1). The write sibling of D155's fs-read: a model in a ReAct turn
//! proposes `{"path": <subpath>, "content": <text>}`; this writes those bytes to
//! ONE regular file (confined + byte-bounded) under the operator-granted WRITE
//! root and commits a small JSON confirmation (`{"path", "bytes_written"}`) as
//! its `result_ref`.
//!
//! # Security (the host-write surface — fail-closed, HITL-gated)
//!
//! A file write is IRREVERSIBLE, so fs-write is deliberately narrower and more
//! gated than its read sibling:
//! 1. A SEPARATE operator env (`KX_SERVE_FS_WRITE_ROOT`) grants a `ReadWrite`
//!    root — enabling reads (`KX_SERVE_FS_ROOT`) never silently implies writes.
//! 2. The tool declares `fs_scope_required = {<root>: ReadWrite}`; dispatch sets
//!    `request.fs_scope = declared ∩ warrant`; the broker `precheck` enforces
//!    `request.fs_scope ⊆ warrant.fs_scope`.
//! 3. `IdempotencyClass::Staged` (set at registration) ⇒ the write stages an
//!    intent the autonomy approval gate (`require_approval`) can hold for human
//!    review before it commits — the policy control this tool exists to honor.
//! 4. The path is confined to the granted root via the target's PARENT (`..` /
//!    symlink / directory-overwrite escapes refused, shared with the read tools
//!    via [`crate::fs_confine`]); fs-write never `mkdir -p`s a new tree.
//! 5. A per-write byte cap gates the content BEFORE any fs touch (no DoS).
//!
//! Default-OFF: nothing registers `fs-write` unless `KX_SERVE_FS_WRITE_ROOT` is
//! set, so the runtime is byte-identical to today when the write root is unset.

use kx_mote::{EffectPattern, ToolName, ToolVersion};
use serde::Deserialize;

use crate::capability::Capability;
use crate::errors::CapabilityFailureReason;
use crate::fs_confine::{first_writable_mount, resolve_confined_writable_path};
use crate::request::EffectRequest;

/// fs-write mutates the world (a file write) and commits a small confirmation as
/// its `result_ref` — the same stage-then-commit content-addressed path fs-read
/// / echo use ("pure-output WORLD-MUTATING work where the effect IS the write").
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// Default per-write byte ceiling (a content bag above this is refused — the DoS
/// guard). Operator-overridable at registration via [`FsWriteCapability::with_max_bytes`].
pub const DEFAULT_MAX_WRITE_BYTES: u64 = 4 * 1024 * 1024;

/// The bundled write-a-confined-file capability (`fs-write@1`).
pub struct FsWriteCapability {
    name: ToolName,
    version: ToolVersion,
    max_bytes: u64,
}

impl FsWriteCapability {
    /// Construct `fs-write@1` with the default per-write byte cap
    /// ([`DEFAULT_MAX_WRITE_BYTES`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_bytes(DEFAULT_MAX_WRITE_BYTES)
    }

    /// Construct `fs-write@1` with an explicit per-write byte cap.
    #[must_use]
    pub fn with_max_bytes(max_bytes: u64) -> Self {
        Self {
            name: ToolName("fs-write".into()),
            version: ToolVersion("1".into()),
            max_bytes,
        }
    }
}

impl Default for FsWriteCapability {
    fn default() -> Self {
        Self::new()
    }
}

/// The model's proposed argument bag — the REQUIRED subpath + text content (the
/// typed `inputSchema` validated this upstream). `deny_unknown_fields` is
/// fail-closed against smuggled keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs {
    path: String,
    content: String,
}

impl Capability for FsWriteCapability {
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
        // (1) The granted WRITE root (already proven ⊆ warrant by the broker
        // precheck). A read-only grant never satisfies write ⇒ fail-closed.
        let root = first_writable_mount(&request.fs_scope).ok_or_else(|| {
            CapabilityFailureReason::Other("fs-write: no writable mount in fs_scope".to_string())
        })?;

        // (2) Parse the REQUIRED path + content.
        let args = parse_write_args(&request.payload)?;

        // (3) Byte cap BEFORE touching the fs (no unbounded write / DoS).
        let n = args.content.len() as u64;
        if n > self.max_bytes {
            return Err(CapabilityFailureReason::Other(format!(
                "fs-write: content is {} bytes, exceeds the {}-byte cap",
                n, self.max_bytes
            )));
        }

        // (4) Resolve + canonicalize + CONFINE the target (parent must exist in
        // the root; `..`/symlink/dir-overwrite escapes refused).
        let target = resolve_confined_writable_path(root, &args.path)?;

        // (5) Write the bytes — the world mutation.
        std::fs::write(&target, args.content.as_bytes())
            .map_err(|e| CapabilityFailureReason::Other(format!("fs-write: write: {e}")))?;

        // The committed result_ref is a small deterministic confirmation the model
        // observes to know the write landed.
        serde_json::to_vec(&serde_json::json!({ "path": args.path, "bytes_written": n }))
            .map_err(|e| CapabilityFailureReason::Other(format!("fs-write: confirm encode: {e}")))
    }
}

/// Parse the model's `{"path": <subpath>, "content": <text>}` arg. fs-write
/// REQUIRES a non-empty path. Empty payload / missing key / empty path / unknown
/// key ⇒ fail-closed.
fn parse_write_args(payload: &[u8]) -> Result<WriteArgs, CapabilityFailureReason> {
    if payload.is_empty() {
        return Err(CapabilityFailureReason::Other(
            "fs-write: missing required args".to_string(),
        ));
    }
    let args: WriteArgs = serde_json::from_slice(payload)
        .map_err(|e| CapabilityFailureReason::Other(format!("fs-write: bad args: {e}")))?;
    if args.path.is_empty() {
        return Err(CapabilityFailureReason::Other(
            "fs-write: empty 'path' arg".to_string(),
        ));
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::{FsMode, FsScope, NetScope, SecretScope};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn req(payload: &[u8], root: &Path, mode: FsMode) -> EffectRequest {
        let mut mounts = BTreeMap::new();
        mounts.insert(root.to_path_buf(), mode);
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
    fn writes_a_confined_file() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsWriteCapability::new();
        let out = cap
            .invoke(&req(
                br#"{"path":"note.txt","content":"hello world"}"#,
                dir.path(),
                FsMode::ReadWrite,
            ))
            .unwrap();
        // the file landed with the exact bytes...
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "hello world"
        );
        // ...and the confirmation reports the byte count.
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["bytes_written"], 11);
        assert_eq!(v["path"], "note.txt");
    }

    #[test]
    fn overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"old").unwrap();
        let cap = FsWriteCapability::new();
        cap.invoke(&req(
            br#"{"path":"f","content":"new"}"#,
            dir.path(),
            FsMode::ReadWrite,
        ))
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f")).unwrap(),
            "new"
        );
    }

    #[test]
    fn refuses_a_read_only_grant() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsWriteCapability::new();
        // a ReadOnly mount must NOT satisfy fs-write (enabling reads never grants writes).
        let err = cap.invoke(&req(
            br#"{"path":"x","content":"y"}"#,
            dir.path(),
            FsMode::ReadOnly,
        ));
        assert!(err.is_err(), "a read-only grant must not permit a write");
        assert!(!dir.path().join("x").exists());
    }

    #[test]
    fn refuses_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsWriteCapability::new();
        assert!(cap
            .invoke(&req(
                br#"{"path":"../evil.txt","content":"x"}"#,
                dir.path(),
                FsMode::ReadWrite
            ))
            .is_err());
    }

    #[test]
    fn refuses_over_the_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsWriteCapability::with_max_bytes(4);
        let err = cap.invoke(&req(
            br#"{"path":"big","content":"toolong"}"#,
            dir.path(),
            FsMode::ReadWrite,
        ));
        assert!(err.is_err(), "content over the byte cap must be refused");
        assert!(
            !dir.path().join("big").exists(),
            "nothing is written on refusal"
        );
    }

    #[test]
    fn refuses_missing_or_unknown_args() {
        let dir = tempfile::tempdir().unwrap();
        let cap = FsWriteCapability::new();
        assert!(cap
            .invoke(&req(b"", dir.path(), FsMode::ReadWrite))
            .is_err());
        assert!(cap
            .invoke(&req(
                br#"{"path":""," content":"x"}"#,
                dir.path(),
                FsMode::ReadWrite
            ))
            .is_err());
        // missing 'content'
        assert!(cap
            .invoke(&req(br#"{"path":"f"}"#, dir.path(), FsMode::ReadWrite))
            .is_err());
        // smuggled unknown key
        assert!(cap
            .invoke(&req(
                br#"{"path":"f","content":"x","evil":"z"}"#,
                dir.path(),
                FsMode::ReadWrite
            ))
            .is_err());
    }

    #[test]
    fn refuses_when_no_fs_grant() {
        let cap = FsWriteCapability::new();
        let r = EffectRequest {
            payload: br#"{"path":"x","content":"y"}"#.to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
            secret_scope: SecretScope::None,
        };
        assert!(cap.invoke(&r).is_err());
    }
}
