//! The validated tool-call vocabulary: [`ToolCall`] (the decoded, warrant-granted
//! proposal) and [`DecodeError`] (the closed refusal vocabulary). Moved verbatim
//! from `kx-model-harness::toolcall` (PR-2d-1) — the authority gate is ONE crate.

use kx_mote::{ToolName, ToolVersion};

/// A validated, warrant-granted tool call the model proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// The tool's name — guaranteed `∈ warrant.tool_grants`.
    pub name: ToolName,
    /// The tool's pinned version — matched exactly against the grant.
    pub version: ToolVersion,
    /// The proposed arguments, verbatim JSON bytes (size-capped, never executed).
    pub args_bytes: Vec<u8>,
}

/// Why a model output that *looked like* a tool call was refused. (A normal
/// completion is `Ok(None)`, not an error.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The output began as a JSON object but the tool-call envelope was malformed,
    /// truncated, or carried trailing garbage. Fail-closed — a half-formed proposal
    /// never fires an effect.
    Malformed {
        /// A short structural diagnostic (never the raw payload).
        diagnostic: String,
    },
    /// The model named a tool that is not in `warrant.tool_grants` (SN-8: the model
    /// cannot authorize an action the runtime did not grant).
    UngrantedTool {
        /// The proposed (ungranted) tool name.
        name: ToolName,
        /// The proposed version.
        version: ToolVersion,
    },
    /// The model named a tool by a NON-UNIQUE alias — a bare leaf (`echo`) or a
    /// `/`-segment (`mcp-echo`) shared by two granted `<server>/<remote>` tools (e.g.
    /// the bundled `mcp-echo/echo` and a dialed `refconn/echo`). Fail-closed (SN-8:
    /// the runtime NEVER guesses which grant the model meant); the `candidates` carry
    /// the addressed full-ids so the react loop can re-prompt with an unambiguous
    /// disambiguation (T-CONNECTOR-AUTOGRANT-LIVE-DEADLETTER). A COMMITTED arm
    /// (marked/native/envelope) raises this; a markerless object degrades to a normal
    /// completion (no false-positive refusal) — same posture as [`Self::UngrantedTool`].
    Ambiguous {
        /// The proposed (ambiguous) name the model emitted, verbatim.
        name: ToolName,
        /// The granted full-ids the name addresses (≥2), in deterministic
        /// `tool_grants` (`BTreeSet`) order.
        candidates: Vec<ToolName>,
    },
    /// The proposed arguments exceed the per-call size cap (IMP-16).
    Oversize {
        /// Observed args size in bytes.
        got: usize,
        /// The cap.
        max: usize,
    },
}
