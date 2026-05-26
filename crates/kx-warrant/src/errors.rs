//! Typed refusal vocabularies: [`NarrowingError`] (from [`crate::intersect`])
//! + [`ToolDenied`] (from [`crate::check_tool_requirement`]).

use kx_content::ContentRef;

use crate::fields::WarrantField;

/// Typed error returned by [`crate::intersect`] when the child's role attempts
/// to widen on a qualitative axis. Quantitative axes never produce this error;
/// they narrow silently via `min()`.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum NarrowingError {
    /// Child's role proposed a value wider than the parent's on a qualitative
    /// axis. Always refused; the model NEVER authorizes a widen.
    #[error("child role attempted to widen warrant on field {field:?}: parent={parent} proposed={proposed}")]
    AttemptedWiden {
        /// The axis that was widened.
        field: WarrantField,
        /// Debug rendering of parent's value.
        parent: String,
        /// Debug rendering of child's proposed value.
        proposed: String,
    },
    /// Intersection on this axis is empty after narrowing.
    #[error("intersection on field {field:?} is empty")]
    EmptyIntersect {
        /// The axis with the empty intersection.
        field: WarrantField,
    },
    /// Syscall profile is not a subset of the parent's profile (per the
    /// seccomp compiler; treated opaquely here).
    #[error("syscall profile {profile_ref:?} is not a subset of parent")]
    SyscallProfileNotASubset {
        /// The non-subset profile reference.
        profile_ref: ContentRef,
    },
    /// Model route is structurally invalid (e.g., zero token ceiling).
    #[error("invalid model route: {reason}")]
    InvalidModelRoute {
        /// Description of why the route is invalid.
        reason: String,
    },
}

/// Typed error returned by [`crate::check_tool_requirement`] when the tool's
/// `ToolRequirement` exceeds the Mote's warrant on a specific axis.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("tool requirement exceeds warrant on field {field:?}")]
pub struct ToolDenied {
    /// The axis that was exceeded.
    pub field: WarrantField,
}
