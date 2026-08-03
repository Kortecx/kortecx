//! The MODEL-USAGE seam — who is holding a model right now.
//!
//! Offloading a model frees RAM by destroying it. Doing that under live work is a
//! disruption the operator cannot undo and, before this seam, could not even see:
//! [`crate::model_lifecycle::ModelLifecycleControl::offload`] evicts unconditionally, so
//! stopping a hosted App's model was one click with no warning and no record.
//!
//! # Why a separate seam rather than a method on the lifecycle control
//!
//! Residency and USE are owned by different things. The lifecycle control owns RAM (it
//! can answer "is this resident?"); only the host knows which Apps, Workflows and hosted
//! servers are bound to a model. Folding the question into the lifecycle trait would
//! force every implementation — including the deterministic test ones — to answer a
//! question about work it has no view of, and the honest answer they would all return is
//! "nothing", which is exactly the wrong default for a guard.
//!
//! # The absent case is REPORTED, never assumed safe
//!
//! A gateway with no usage view wired returns no holders. That is indistinguishable from
//! "nothing is using it" unless the caller is told which happened, so the RPC carries
//! `usage_checked` alongside the holder list. A guard whose unwired state reads as a
//! clean bill of health is not a guard.

use crate::error::GatewayError;

/// What kind of work holds a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelHolderKind {
    /// A hosted App whose server process is live.
    HostedApp,
    /// A saved App bound to this model.
    App,
    /// A saved Workflow bound to this model.
    Workflow,
    /// An in-flight run.
    Run,
}

/// One holder of a model — work an offload would disrupt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHolder {
    /// What kind of work this is.
    pub kind: ModelHolderKind,
    /// The App / Workflow handle holding the model.
    pub handle: String,
    /// Advisory display prose. Never authority.
    pub detail: String,
}

/// The host-side view of which work currently holds a model.
pub trait ModelUsageView: Send + Sync {
    /// Everything currently holding `model_id`.
    ///
    /// An EMPTY result means "nothing this view can see holds it" — the view reports
    /// what it observes and never claims completeness. The caller distinguishes an empty
    /// answer from an absent view; this method cannot.
    ///
    /// # Errors
    /// [`GatewayError::Internal`] when the host cannot determine usage. A failure is
    /// propagated rather than degraded to "no holders": degrading here would turn an
    /// unreadable supervisor into a silent green light for a disruptive offload.
    fn holders(&self, model_id: &str) -> Result<Vec<ModelHolder>, GatewayError>;
}
