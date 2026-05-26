//! [`AssemblyError`] — typed refusal vocabulary for [`crate::assemble`].

use kx_content::ContentRef;
use kx_mote::MoteId;
use kx_tool_registry::ResolutionError;
use kx_warrant::ToolGrant;

/// Reason [`crate::assemble`] refused.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum AssemblyError {
    /// A declared Data-edge parent has no committed `result_ref` in the
    /// snapshot. Indicates the scheduler dispatched this Mote prematurely.
    #[error("declared Data-edge parent has no committed result_ref: {parent_mote_id:?}")]
    UpstreamNotCommitted {
        /// The parent that should be Committed but isn't.
        parent_mote_id: MoteId,
    },

    /// The content store doesn't have bytes for a `ContentRef` that the
    /// projection said exists. Indicates content-store inconsistency
    /// (operational issue).
    #[error("content store has no entry for ref: {content_ref:?}")]
    ContentStoreMiss {
        /// The missing ref.
        content_ref: ContentRef,
    },

    /// A granted tool failed to resolve through the registry. Surfaces both
    /// `NotFound` and `CapabilityExceedsWarrant` and `PendingHumanReview`
    /// from the registry layer.
    #[error("tool resolution failed: {reason}")]
    ToolNotResolvable {
        /// The tool grant that failed to resolve.
        grant: ToolGrant,
        /// The underlying registry error (typed; see `kx_tool_registry`).
        reason: ResolutionError,
    },

    /// The closure of resolvable content exceeds `window_bytes`. Carries the
    /// MEASURED closure size — the caller picks a deterministic ranking or
    /// summarization strategy (per D33 §5).
    #[error("context closure exceeds window: closure={closure_size_bytes} window={window_bytes}")]
    OverflowDecisionRequired {
        /// Measured total bytes that would be assembled.
        closure_size_bytes: usize,
        /// The window cap.
        window_bytes: usize,
    },
}
