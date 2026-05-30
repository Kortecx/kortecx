//! Opt-in capture scope + consent. The safe choice is the default; retaining
//! step-level reasoning/thinking is opt-in-with-consent (the safe-default
//! principle at every human-handoff seam).

/// How much of an atomic Mote's (agent's) step is retained in the capture
/// projection. The runtime always reuses the **action** (the committed Mote
/// result on the journal); this scope only governs the OPT-IN, OFF-TRUTH-PATH
/// retention of step-level exhaust for the user's own analysis/reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash, PartialOrd, Ord)]
pub enum CaptureScope {
    /// **Default.** Retain only the committed action's `result_ref` (a join key
    /// back to truth). Reasoning/thinking/input are NOT retained. The
    /// privacy-safe default — reuse the action, never the thinking.
    #[default]
    ActionsOnly,
    /// **Opt-in.** Retain step-level input / output / reasoning / thinking blobs
    /// (all content-addressed) at the atomic Mote (agent) level. Still a
    /// disposable projection — never journaled, never gates, never feeds identity.
    Full,
}

/// Per-capture-session consent. The dangerous (data-retaining) choice is
/// opt-in; the default retains only actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CaptureConsent {
    /// The retention scope this session consented to.
    pub scope: CaptureScope,
}

impl CaptureConsent {
    /// The safe default: retain only committed actions.
    #[must_use]
    pub const fn actions_only() -> Self {
        Self {
            scope: CaptureScope::ActionsOnly,
        }
    }

    /// Opt in to full step-level capture (input/output/reasoning/thinking).
    #[must_use]
    pub const fn full() -> Self {
        Self {
            scope: CaptureScope::Full,
        }
    }

    /// `true` iff this session consented to step-level (non-action) capture.
    #[must_use]
    pub const fn captures_steps(&self) -> bool {
        matches!(self.scope, CaptureScope::Full)
    }
}
