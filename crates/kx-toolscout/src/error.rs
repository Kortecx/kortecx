//! The crate error — every refusal is loud and names what was refused.

use kx_mote::{ToolName, ToolVersion};
use kx_workflow::CompileError;

/// Lowering / compilation failures. Fail-closed: an empty bundle or an
/// ungranted tool refuses BEFORE any step is built.
#[derive(Debug, thiserror::Error)]
pub enum ToolScoutError {
    /// The bundle's `tool_sequence` is empty — nothing to lower.
    #[error("the task bundle names no tools (empty tool_sequence)")]
    EmptyBundle,

    /// A sequenced tool is not in the warrant's grant set. EXACT
    /// `(name, version)` membership (SN-8) — a matching name with a different
    /// version is just as refused as an unknown tool.
    #[error("tool {name}@{version} is not granted by the warrant (exact-match refusal)", name = .name.0, version = .version.0)]
    UngrantedTool {
        /// The refused tool's name.
        name: ToolName,
        /// The refused tool's version.
        version: ToolVersion,
    },

    /// The frozen structural gate rejected the lowered definition.
    #[error("the lowered workflow failed to compile")]
    Compile(#[from] CompileError),
}
