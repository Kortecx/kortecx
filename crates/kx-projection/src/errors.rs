//! [`ProjectionError`] — errors raised by [`crate::Projection`] operations.

use kx_mote::MoteId;

/// Errors raised by [`Projection`] operations.
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
}
