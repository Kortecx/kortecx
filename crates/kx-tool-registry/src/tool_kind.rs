//! [`ToolKind`] — what kind of tool this is and where it lives. Reflected in
//! [`crate::ToolResolutionEvent::resolved_kind`] so replay can verify the same
//! tier resolved the same tool.

use kx_content::ContentRef;
use kx_mote::MoteId;
use serde::{Deserialize, Serialize};

use crate::ids::McpEndpointId;

/// What kind of tool this is, and how it was sourced.
///
/// Reflected in [`crate::ToolResolutionEvent::resolved_kind`] so replay can verify
/// the same tier resolved the same tool.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolKind {
    /// A built-in tool that ships with the OSS runtime (`fs-read`,
    /// `fs-write`, `http-get`, `text-summarize`, …).
    Builtin,
    /// A local script registered against this registry. The bytes of the
    /// script live in the content store at the given `script_ref`.
    LocalScript {
        /// Content-store reference to the script bytes.
        script_ref: ContentRef,
    },
    /// An external tool sourced from a URL (e.g., a hosted registry entry).
    External {
        /// Origin URL (opaque to this crate; resolved by the broker).
        source_url: String,
    },
    /// A tool exposed via MCP at the given endpoint with the given remote
    /// name. **Granting an MCP tool requires the warrant's `net_scope` to
    /// permit the MCP endpoint's host** — enforced by the subset check at
    /// resolution time.
    Mcp {
        /// Which MCP endpoint serves this tool.
        endpoint: McpEndpointId,
        /// The tool's name on the remote MCP server.
        remote_name: String,
    },
    /// A self-generated tool emitted by a Mote at the given identity. INERT
    /// until human review per D32; capability ⊆ generating lineage's warrant
    /// at approve time.
    SelfGenerated {
        /// The Mote that emitted this tool.
        generated_at_mote: MoteId,
    },
}
