//! [`CompileError`] — the closed vocabulary of ways a [`crate::WorkflowDef`]
//! can fail to compile into a Mote DAG.
//!
//! Every variant names a *structural* defect detected by the pure
//! [`crate::compile`] pass — never an I/O or runtime failure (compilation does
//! neither). The variants are exhaustive: a `WorkflowDef` that triggers none of
//! them compiles to a well-formed, acyclic DAG.

use thiserror::Error;

/// A structural defect that prevents a [`crate::WorkflowDef`] from compiling
/// into a valid Mote DAG.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompileError {
    /// An edge (or a critic's producer reference) names a step index that does
    /// not exist in the workflow. Carries the offending index.
    #[error("step index {0} is out of range for this workflow")]
    StepIndexOutOfRange(usize),

    /// The declared edges form a cycle. A Mote's identity derives from its
    /// committed inputs, so the execution graph MUST be acyclic — loops are
    /// expressed at runtime via topology shapers, not as static cycles.
    /// Carries one step index that participates in the unresolved cycle.
    #[error("workflow DAG contains a cycle involving step {0}")]
    Cycle(usize),

    /// A critic step references a producer that is not a (transitive)
    /// predecessor — the producer's `MoteId` is not yet known when the critic
    /// is compiled. Add a dependency edge from the producer to the critic.
    #[error("critic step {critic} references producer step {producer}, which does not precede it")]
    InvalidCritic {
        /// The critic step's index.
        critic: usize,
        /// The referenced producer step's index.
        producer: usize,
    },

    /// The same `(parent, child)` edge was declared more than once.
    #[error("duplicate edge from step {parent} to step {child}")]
    DuplicateEdge {
        /// The parent step's index.
        parent: usize,
        /// The child step's index.
        child: usize,
    },

    /// A prompt template (see [`crate::PromptTemplate`]) is structurally
    /// malformed — an unbalanced, empty, nested, or invalid-character
    /// `{placeholder}` brace. Detected by the pure [`crate::PromptTemplate::parse`].
    #[error("malformed prompt template: {reason}")]
    MalformedTemplate {
        /// Human-readable description of the structural defect.
        reason: String,
    },

    /// A template placeholder has no bound parameter at render time. Fail-closed:
    /// every `{name}` MUST be supplied — a recipe never renders a half-bound prompt.
    #[error("prompt template placeholder '{name}' has no bound parameter")]
    MissingPlaceholder {
        /// The unfilled placeholder name.
        name: String,
    },

    /// A supplied parameter names no placeholder the template declares.
    /// Fail-closed: an unknown parameter is a caller error, never silently dropped
    /// (a typo'd param would otherwise leave the intended placeholder unfilled).
    #[error("prompt parameter '{name}' names no template placeholder")]
    UnknownParam {
        /// The unrecognized parameter name.
        name: String,
    },

    /// [`crate::render_prompts`] failed rendering the template carried by a
    /// specific step. Carries the step index and the underlying reason; the
    /// `WorkflowDef` is left byte-unchanged (the pass is atomic — clone-then-commit).
    #[error("prompt render failed at step {step}: {reason}")]
    RenderPromptStep {
        /// The index of the step whose template failed to render.
        step: usize,
        /// The underlying [`MalformedTemplate`](Self::MalformedTemplate) /
        /// [`MissingPlaceholder`](Self::MissingPlaceholder) /
        /// [`UnknownParam`](Self::UnknownParam) failure, rendered.
        reason: String,
    },

    /// A recipe builder was given an empty set of required step logics (zero
    /// mappers / workers / attempts / images). A recipe must declare ≥1 such step.
    #[error("recipe requires at least one step but none were provided")]
    EmptyRecipe,
}
