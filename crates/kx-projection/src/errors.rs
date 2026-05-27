//! [`ProjectionError`] — errors raised by [`crate::Projection`] operations.

use kx_content::ContentRef;
use kx_mote::MoteId;

/// Errors raised by [`crate::Projection`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// The fold detected two `Committed` entries for the same `MoteId` — this is
    /// a journal-layer bug (the dedupe-by-key path failed). Surfaced loudly per
    /// `projection.md` §4 ("if it does, that is a journal-impl bug, not a precedence
    /// question — surface it loudly").
    #[error("two Committed entries for MoteId {0} (journal dedupe-by-key bug)")]
    DuplicateCommitted(MoteId),

    /// Wraps an underlying [`kx_journal::JournalError`] surfaced while folding from
    /// a `Journal` instance.
    #[error(transparent)]
    Journal(#[from] kx_journal::JournalError),

    /// The topology materializer (D48 + D49 / P1.11) tried to fetch a shaper's
    /// `TopologyDecision` payload from the content store and the fetch failed.
    /// Surfaced loudly so the fold caller (executor / orchestrator) can react.
    #[error("content store fetch failed for shaper result_ref {result_ref:?}: {details}")]
    ContentStoreFetch {
        /// The result_ref whose payload could not be retrieved.
        result_ref: ContentRef,
        /// Underlying error formatted as a string (preserved here rather than
        /// typed because [`kx_content::ContentStore`]'s associated error type
        /// makes a generic transparent wrap impractical).
        details: String,
    },

    /// The topology materializer fetched a payload but bincode-canonical
    /// deserialization as `TopologyDecision` failed. Either the shaper
    /// committed something that wasn't a `TopologyDecision` (workflow-author
    /// bug — R-8 / R-14 should have caught this), or the journal/content-store
    /// is corrupt.
    #[error("failed to deserialize TopologyDecision from result_ref {result_ref:?}: {details}")]
    TopologyDecodeFailed {
        /// The result_ref whose bytes failed to decode.
        result_ref: ContentRef,
        /// Underlying bincode error formatted as a string.
        details: String,
    },
}
