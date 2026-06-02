//! [`PlanError`] — the closed vocabulary for every way a plan is refused, from
//! fail-closed decode (IMP-5) through role resolution (D75) to the structural
//! gate ([`kx_workflow::compile`]). Derives `PartialEq + Eq` (every embedded
//! error does too) so property tests can assert the exact refusal.

use kx_mote::RoleId;
use kx_warrant::NarrowingError;
use kx_workflow::CompileError;
use thiserror::Error;

/// Why a model-proposed plan was refused. A refusal NEVER produces a partial DAG
/// — the plan is rejected whole, fail-closed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    // ----- decode (IMP-5): the untrusted-bytes surface -----
    /// The plan bytes exceed the per-plan cap (checked BEFORE parsing, so a
    /// hostile model cannot force a large allocation).
    #[error("plan bytes {got} exceed the cap {max}")]
    Oversize {
        /// Observed plan size in bytes.
        got: usize,
        /// The cap.
        max: usize,
    },
    /// The envelope was malformed: non-JSON, non-object, truncated, trailing
    /// garbage, or an unexpected key (`deny_unknown_fields`). Fail-closed.
    #[error("plan envelope malformed: {diagnostic}")]
    Malformed {
        /// A short structural diagnostic (never the raw payload).
        diagnostic: String,
    },
    /// The envelope declared a schema `version` this binary does not understand.
    #[error("unknown plan schema version {version}")]
    UnknownVersion {
        /// The unrecognized version.
        version: u32,
    },
    /// The plan declared zero steps (there is nothing to register).
    #[error("plan declared zero steps")]
    EmptyPlan,
    /// The plan declared more steps than [`crate::MAX_PLAN_STEPS`] (`DoS` bound).
    #[error("plan declared {got} steps, exceeding the cap {max}")]
    TooManySteps {
        /// Observed step count.
        got: usize,
        /// The cap.
        max: usize,
    },
    /// The plan declared more edges than [`crate::MAX_PLAN_EDGES`] (`DoS` bound).
    #[error("plan declared {got} edges, exceeding the cap {max}")]
    TooManyEdges {
        /// Observed edge count.
        got: usize,
        /// The cap.
        max: usize,
    },

    // ----- role resolution / lowering (D75): authority + identity -----
    /// A critic/deterministic-critic step omitted its `producer`, or named a
    /// `producer` index that does not precede it (`producer >= this step`) or is
    /// out of range. Producers MUST precede critics.
    #[error("step {step} has an invalid producer reference {producer:?}")]
    InvalidProducer {
        /// The critic step index.
        step: usize,
        /// The (missing / out-of-range / non-preceding) producer index.
        producer: Option<usize>,
    },
    /// The plan named a role that is not in the warrant `RoleRegistry` (no fuzzy
    /// fallback — exact `RoleId` equality, D70).
    #[error("plan named role {0:?}, which is not registered in the role registry")]
    UnknownRole(RoleId),
    /// The plan named a role for which no [`crate::RoleRecipe`] is registered
    /// (the heavy `MoteDef` axes are unknown — refuse rather than guess).
    #[error("plan named role {0:?}, for which no recipe is registered")]
    UnknownRecipe(RoleId),
    /// A [`PlanStepKind::DeterministicCritic`](crate::PlanStepKind::DeterministicCritic)
    /// step's recipe carries no `CheckSpec` (a deterministic critic with no
    /// check is meaningless).
    #[error("role {0:?} is used as a deterministic critic but its recipe declares no check")]
    MissingCheck(RoleId),
    /// The role's recipe declares a tool the role's warrant does not grant — a
    /// step could never legally call it, so the plan is refused (IMP-5: refuse
    /// ungrantable tools up front).
    #[error("role {role:?} recipe requires tool {tool} which the role warrant does not grant")]
    UngrantableTool {
        /// The offending role.
        role: RoleId,
        /// The ungranted tool (`name@version`).
        tool: String,
    },
    /// Computing the step's warrant via `intersect(parent, role)` failed — the
    /// role proposed authority wider than the parent (D75: never widens).
    #[error("role {role:?} cannot be granted under the parent warrant: {source}")]
    Ungrantable {
        /// The offending role.
        role: RoleId,
        /// The underlying narrowing error.
        #[source]
        source: NarrowingError,
    },

    // ----- the structural gate (delegated to kx_workflow::compile) -----
    /// The lowered plan is not a valid Mote DAG. `kx_workflow::compile` is the
    /// sole structural gate: a cycle (an agentic loop authored as a back-edge
    /// instead of a shaper), a critic that does not follow its producer, a
    /// duplicate edge, or an out-of-range index all surface here.
    #[error("compiled plan is not a valid Mote DAG: {0}")]
    Compile(#[from] CompileError),
}
