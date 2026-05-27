//! [`ProjectionError`] — errors raised by [`crate::Projection`] operations.

use kx_content::ContentRef;
use kx_mote::{MoteId, RoleId};

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

    /// **PR 11.5 / KG-1-close.** The topology materializer tried to fetch a
    /// shaper's [`kx_warrant::WarrantSpec`] from the content store (so it
    /// could compute child warrants via D30's `intersect`) and the fetch
    /// failed. The shaper's warrant_ref MUST resolve to its WarrantSpec
    /// at fold time — workflow author / executor MUST `put` the spec
    /// before the shaper commits.
    #[error("warrant store fetch failed for shaper warrant_ref {warrant_ref:?}: {details}")]
    WarrantStoreFetch {
        /// The warrant_ref whose payload could not be retrieved.
        warrant_ref: ContentRef,
        /// Underlying error formatted as a string.
        details: String,
    },

    /// **PR 11.5 / KG-1-close.** The materializer fetched a payload but
    /// bincode-canonical deserialization as `WarrantSpec` failed. The
    /// content store is corrupt or the wrong bytes were written under
    /// the warrant_ref.
    #[error("failed to deserialize WarrantSpec from warrant_ref {warrant_ref:?}: {details}")]
    WarrantDecodeFailed {
        /// The warrant_ref whose bytes failed to decode.
        warrant_ref: ContentRef,
        /// Underlying bincode error formatted as a string.
        details: String,
    },

    /// **PR 11.5 / KG-1-close.** A child descriptor named a `RoleId` that
    /// the [`kx_warrant::RoleRegistry`] does not know. The materializer
    /// refuses to silently widen — the workflow author MUST register
    /// every role referenced by any descriptor before submitting the
    /// shaper.
    #[error("role {role_id:?} (descriptor index {descriptor_index}) is not registered in the role registry")]
    RoleNotRegistered {
        /// The unresolved descriptor-side handle.
        role_id: RoleId,
        /// Index of the descriptor in `TopologyDecision.children`.
        descriptor_index: usize,
    },

    /// **PR 11.5 / KG-1-close.** D30 `intersect(shaper.warrant,
    /// role.spec)` returned a typed [`kx_warrant::NarrowingError`] —
    /// the proposed role attempts to widen the shaper's warrant on
    /// some axis. Surfaced as a fold error so the workflow author /
    /// operator sees the offending descriptor + axis.
    #[error("warrant narrowing failed for descriptor index {descriptor_index}: {details}")]
    NarrowingFailed {
        /// Index of the descriptor in `TopologyDecision.children`.
        descriptor_index: usize,
        /// The underlying [`kx_warrant::NarrowingError`] formatted as a string.
        details: String,
    },
}
