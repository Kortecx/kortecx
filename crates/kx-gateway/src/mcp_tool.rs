//! PR-2d-2 (react-tools-live) — wire the BUNDLED deterministic stdio MCP tool
//! (`mcp-echo@1`, the `kx-mcp-echo` bin) into a `kx serve`: locate the binary,
//! register its [`McpCapability`] on the serve broker (the first
//! kx-gateway→MCP edge, behind `--features inference`), and describe it to the
//! tool registry with a typed `inputSchema` so the coordinator's settle
//! validates every model-proposed args bag FAIL-CLOSED (D110.4) before a
//! `Tool` decision is frozen.
//!
//! The tool is the SMALLEST possible "Act" surface: deterministic in its args
//! (exactly-once at the world boundary by content-addressing, D58 §7) and
//! `net_scope: None` (no egress — SSRF vetting is N/A). Fail-soft: no binary on
//! this host/image ⇒ no capability, no `kx/recipes/react` provisioning, and the
//! serve behaves exactly as before.

use std::path::PathBuf;

use kx_capability::LocalCapabilityBroker;
use kx_content::{ContentRef, ContentStore};
use kx_mcp::{McpCapability, StdioTransport};
use kx_mote::{ToolName, ToolVersion};
use kx_tool_registry::{
    IdempotencyClass, InputSchema, McpEndpointId, ParamSpec, ParamType, ToolDef, ToolKind,
};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};

/// The bundled tool's identity — `mcp-echo/echo@1`. The id follows the
/// `<server>/<remote>` convention every other MCP tool uses (a dialed/local tool
/// registers `<server>/<remote>`, e.g. `kxlocal-a1b2c3d4/multiply` or a dialed
/// `pr2echo/echo`), so the BUG-32 name-resolution leaf rule
/// (`kx_toolcall::resolve_granted_name`) resolves the short remote leaf `echo`
/// the model naturally emits. BEFORE this fix the bundled tool was a flat
/// `mcp-echo` (server+remote conflated into one hyphenated token with no `/`),
/// so a capable model that proposed the bare `echo` was refused `UngrantedTool`
/// and the live ReAct chain dead-lettered with no answer (PR-2 deep-test campaign
/// finding A1 / BUG-33). The remote method name passed to the MCP server stays
/// `echo` (see [`register_echo_capability`]), so firing is unchanged.
#[must_use]
pub(crate) fn echo_tool() -> (ToolName, ToolVersion) {
    (ToolName("mcp-echo/echo".into()), ToolVersion("1".into()))
}

/// The bundled tool's [`ToolDef`]: an MCP stdio tool with NO egress requirement
/// and a typed one-param schema (`q: Str`, required, unknown keys refused) —
/// so `validate_args` genuinely gates every proposed call.
#[must_use]
pub(crate) fn echo_tool_def() -> ToolDef {
    let (tool_id, tool_version) = echo_tool();
    ToolDef {
        tool_id,
        tool_version,
        kind: ToolKind::Mcp {
            endpoint: McpEndpointId("stdio://kx-mcp-echo".into()),
            remote_name: "echo".into(),
        },
        required_capability: ToolRequirement {
            net_scope_required: NetScope::None,
            fs_scope_required: FsScope::empty(),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: "Bundled deterministic stdio echo tool (the live ReAct demo Act step).".into(),
        idempotency_class: IdempotencyClass::Staged,
        input_schema: Some(InputSchema {
            params: vec![ParamSpec {
                name: "q".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: true,
            }],
            deny_unknown: true,
        }),
    }
}

/// The bundled filesystem-listing tool's identity — `fs-list@1` (PR-6a / D155).
#[must_use]
pub(crate) fn fs_list_tool() -> (ToolName, ToolVersion) {
    (ToolName("fs-list".into()), ToolVersion("1".into()))
}

/// The `fs-list@1` [`ToolDef`]: a read-only host filesystem tool whose declared
/// `fs_scope_required` is exactly the operator-granted read root (`KX_SERVE_FS_ROOT`),
/// with a typed optional `path` subpath param. The declared fs_scope MUST equal the
/// `fs_list_warrant` grant so the broker's `precheck` subset gate passes and the
/// capability receives the root via `request.fs_scope` (NO egress — `net_scope: None`).
#[must_use]
pub(crate) fn fs_list_tool_def(root: &std::path::Path) -> ToolDef {
    let (tool_id, tool_version) = fs_list_tool();
    let mut mounts = std::collections::BTreeMap::new();
    mounts.insert(root.to_path_buf(), kx_warrant::FsMode::ReadOnly);
    ToolDef {
        tool_id,
        tool_version,
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
            net_scope_required: NetScope::None,
            fs_scope_required: FsScope { mounts },
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: "List a directory's immediate entries (names + kind + size; NEVER file contents) under the operator-granted read-only root. Arg: {\"path\": <subpath>} (optional; defaults to the root). Read-only; naturally idempotent.".into(),
        idempotency_class: IdempotencyClass::Readback,
        input_schema: Some(InputSchema {
            params: vec![ParamSpec {
                name: "path".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: false,
            }],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled read-only [`FsListCapability`] (`fs-list@1`) on the serve
/// broker. Operator-gated: the caller registers it ONLY when `KX_SERVE_FS_ROOT`
/// is set (default-OFF ⇒ no capability, no `kx/recipes/react-fs`, byte-identical
/// serve). The root comes via each dispatch's `request.fs_scope` (the warrant
/// grant ∩ the tool's declared scope), so the capability itself is root-agnostic.
pub(crate) fn register_fs_list_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
) {
    broker.register_capability(Box::new(kx_capability::FsListCapability::new()));
    tracing::info!("PR-6a/D155: read-only fs-list@1 capability registered (kx/recipes/react-fs)");
}

/// The bundled filesystem-read tool's identity — `fs-read@1` (D155 Phase-A).
#[must_use]
pub(crate) fn fs_read_tool() -> (ToolName, ToolVersion) {
    (ToolName("fs-read".into()), ToolVersion("1".into()))
}

/// The `fs-read@1` [`ToolDef`]: a read-only host filesystem tool that reads ONE
/// confined file's bytes INTO the content store (the snapshot-in leg). Its
/// declared `fs_scope_required` is exactly the operator-granted read root, with a
/// typed REQUIRED `path` param. The bytes become the Observation Mote's
/// `result_ref` (= the file's `ContentRef`); read-only, byte-capped, no egress.
#[must_use]
pub(crate) fn fs_read_tool_def(root: &std::path::Path) -> ToolDef {
    let (tool_id, tool_version) = fs_read_tool();
    let mut mounts = std::collections::BTreeMap::new();
    mounts.insert(root.to_path_buf(), kx_warrant::FsMode::ReadOnly);
    ToolDef {
        tool_id,
        tool_version,
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
            net_scope_required: NetScope::None,
            fs_scope_required: FsScope { mounts },
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: "Read a single file's raw bytes under the operator-granted read-only root. Arg: {\"path\": <subpath>} (REQUIRED). The content is committed to the store; the result is the file's content hash. Read-only; per-file byte-capped; naturally idempotent.".into(),
        idempotency_class: IdempotencyClass::Readback,
        input_schema: Some(InputSchema {
            params: vec![ParamSpec {
                name: "path".into(),
                ty: ParamType::Str { max_len: 4096 },
                required: true,
            }],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled read-into-CAS [`FsReadCapability`] (`fs-read@1`) on the
/// serve broker. Operator-gated identically to `fs-list@1` (only when
/// `KX_SERVE_FS_ROOT` is set). Root-agnostic — the confined root arrives via each
/// dispatch's `request.fs_scope`.
pub(crate) fn register_fs_read_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
) {
    broker.register_capability(Box::new(kx_capability::FsReadCapability::new()));
    tracing::info!("D155 Phase-A: read-only fs-read@1 capability registered (kx/recipes/react-fs)");
}

/// Locate the bundled `kx-mcp-echo` binary and register its capability on the
/// serve broker. Returns the tool identity when registered (⇒ the react recipe
/// can be provisioned), `None` when no binary is available (fail-soft — a missing
/// bundled binary never breaks serve).
pub(crate) fn register_echo_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
) -> Option<(ToolName, ToolVersion)> {
    let path = echo_binary_path()?;
    let (tool_id, tool_version) = echo_tool();
    broker.register_capability(Box::new(McpCapability::new(
        tool_id.clone(),
        tool_version.clone(),
        McpEndpointId("stdio://kx-mcp-echo".into()),
        "echo",
        Box::new(StdioTransport::new(path.to_string_lossy().as_ref())),
    )));
    tracing::info!(
        bin = %path.display(),
        "PR-2d-2: bundled stdio tool registered (kx/recipes/react is live)"
    );
    Some((tool_id, tool_version))
}

// `KX_SERVE_FS_ROOT` is resolved by `server::serve_fs_root()` (non-gated by
// `inference`, so the D155 branch snapshot path gets the root without a model);
// this module's fs-list/fs-read registration receives the resolved root.

/// PR-6b-4: the operator opt-in for the autonomous-loop tool AUTO-GRANT
/// (`KX_SERVE_AUTOGRANT`). Default-OFF (unset / `"0"` / `"false"` / empty ⇒
/// `false`) — mirrors `KX_SERVE_FS_ROOT`'s deny-by-default. ON ⇒ the
/// `kx/recipes/react-auto` recipe is seeded + the binder rebuilds its warrant
/// from the LIVE registry at bind (the model picks from ALL registered/dialed
/// tools). OFF ⇒ react-auto is NOT seeded ⇒ byte-identical serve.
pub(crate) fn autogrant_enabled() -> bool {
    match std::env::var_os("KX_SERVE_AUTOGRANT") {
        Some(v) => {
            let v = v.to_string_lossy();
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        }
        None => false,
    }
}

/// Resolve the bundled tool binary's path: an explicit `KX_MCP_ECHO_PATH`
/// override first, then the fixed in-image path, then a dev/test walk up to the
/// workspace `target/` dir (the `real_body_binary_path` precedent). `None` ⇒
/// no tool available on this host/image.
fn echo_binary_path() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("KX_MCP_ECHO_PATH") {
        let path = PathBuf::from(over);
        if path.exists() {
            return Some(path);
        }
    }
    let in_image = PathBuf::from("/usr/local/libexec/kx/kx-mcp-echo");
    if in_image.exists() {
        return Some(in_image);
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join("kx-mcp-echo");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-33 (PR-2 deep-test campaign finding A1) regression guard: the bundled
    /// echo MUST be granted as `<server>/<remote>` (a `/`-bearing id) whose last
    /// segment equals the MCP remote method name — so `kx_toolcall`'s leaf rule
    /// resolves the bare remote name (`echo`) a capable model naturally emits. A
    /// flat id (no `/`, the pre-fix shape) reintroduces the `UngrantedTool`
    /// dead-letter that left the live ReAct chain with no answer.
    #[test]
    fn bundled_tool_id_is_server_slash_remote_so_the_model_leaf_resolves() {
        let (name, _ver) = echo_tool();
        assert!(
            name.0.contains('/'),
            "bundled tool id must be <server>/<remote>, got {:?}",
            name.0
        );
        let leaf = name.0.rsplit('/').next().unwrap();
        match &echo_tool_def().kind {
            ToolKind::Mcp { remote_name, .. } => assert_eq!(
                leaf, remote_name,
                "the id leaf must equal the MCP remote name so the model's bare \
                 remote name resolves to the grant"
            ),
            _ => panic!("bundled echo must be an MCP tool"),
        }
    }
}
