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
    IdempotencyClass, InMemoryToolRegistry, InputSchema, McpEndpointId, ParamSpec, ParamType,
    ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};

/// The bundled tool's identity — `mcp-echo@1` (the vocabulary the PR-2d-1
/// substrate tests froze).
#[must_use]
pub(crate) fn echo_tool() -> (ToolName, ToolVersion) {
    (ToolName("mcp-echo".into()), ToolVersion("1".into()))
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

/// A tool registry carrying the OSS built-ins PLUS the bundled echo tool —
/// shared by the coordinator (settle-time `validate_args` + lease-time args
/// re-derivation) and the recipe seeding, so the grant, the registry, and the
/// broker agree by construction.
#[must_use]
pub(crate) fn registry_with_echo() -> InMemoryToolRegistry {
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        echo_tool_def(),
        ToolProvenance::HumanAuthored {
            author: "kx-gateway".into(),
        },
    );
    reg
}

/// Locate the bundled `kx-mcp-echo` binary and register its capability on the
/// serve broker. Returns the tool identity when registered (⇒ the react recipe
/// can be provisioned), `None` when no binary is available (fail-soft — the
/// `register_demo_body` precedent).
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
