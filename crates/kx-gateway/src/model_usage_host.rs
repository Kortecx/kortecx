//! `HostModelUsage` — the host side of the [`ModelUsageView`] seam.
//!
//! Answers "who is holding this model right now" from the things this serve can actually
//! observe: hosted Apps whose server process is LIVE, resolved against each App's
//! declared model route.
//!
//! # What it sees, and what it does not
//!
//! It reports hosted Apps in a live state. It does NOT yet see in-flight
//! `SubmitWorkflow` / `RunApp` runs — those live in the coordinator, behind a seam this
//! view has no handle on. That is a REAL limit and it is stated here rather than implied
//! away: an empty holder list from this view means "no live hosted App holds it", not
//! "nothing in the runtime does". The RPC's `usage_checked` flag tells a caller the
//! check ran; this doc tells whoever extends it where the next source belongs.
//!
//! # The empty-route case is a holder too
//!
//! An App with no declared `model_route` runs on the served DEFAULT. Offloading the
//! default therefore disrupts it just as surely as offloading a model it named, so an
//! empty route counts as a hold on the default model. Missing this would make the most
//! common App shape — one that names no model at all — invisible to the guard.

use std::sync::Arc;

use kx_gateway_core::{
    AppManifestView, GatewayError, HostedAppSupervisor, HostedState, ModelHolder, ModelHolderKind,
    ModelUsageView,
};

/// Host-side model-usage view over the hosted supervisor + the App manifest resolver.
pub(crate) struct HostModelUsage {
    supervisor: Arc<dyn HostedAppSupervisor>,
    manifests: Arc<dyn AppManifestView>,
    /// The serve's primary principal. Single-node serves have one; this mirrors the
    /// `parties.first()` namespacing the memory view already uses.
    principal: String,
    /// The served DEFAULT model id, if any — what an App with an empty `model_route`
    /// actually runs on.
    default_model: Option<String>,
}

impl HostModelUsage {
    /// Construct the view.
    #[must_use]
    pub(crate) fn new(
        supervisor: Arc<dyn HostedAppSupervisor>,
        manifests: Arc<dyn AppManifestView>,
        principal: String,
        default_model: Option<String>,
    ) -> Self {
        Self {
            supervisor,
            manifests,
            principal,
            default_model,
        }
    }

    /// Whether a hosted state means "work is live and would be disrupted".
    ///
    /// Everything except `Stopped` and `Failed` counts. A materializing/installing/
    /// building App is mid-lifecycle and killing its model mid-flight is exactly the
    /// disruption being guarded — waiting only for `Running` would leave the whole
    /// startup window unprotected.
    fn is_live(state: HostedState) -> bool {
        !matches!(state, HostedState::Stopped | HostedState::Failed)
    }
}

impl ModelUsageView for HostModelUsage {
    fn holders(&self, model_id: &str) -> Result<Vec<ModelHolder>, GatewayError> {
        let live = self
            .supervisor
            .list(&self.principal)?
            .into_iter()
            .filter(|s| Self::is_live(s.state));

        let mut out = Vec::new();
        for status in live {
            // A manifest that cannot be resolved is SKIPPED rather than failing the whole
            // query: one unreadable App must not make every offload impossible. The
            // trade-off is deliberate and is the conservative direction only because the
            // supervisor already told us this App is live — we lose a holder we could not
            // name, not the knowledge that something is running.
            let Ok(Some(manifest)) = self.manifests.manifest(&self.principal, &status.handle)
            else {
                continue;
            };
            let holds = if manifest.model_route.is_empty() {
                // No declared route ⇒ it runs on the served default.
                self.default_model.as_deref() == Some(model_id)
            } else {
                manifest.model_route == model_id
            };
            if holds {
                out.push(ModelHolder {
                    kind: ModelHolderKind::HostedApp,
                    handle: status.handle.clone(),
                    detail: match status.state {
                        HostedState::Running => "hosted server running".to_string(),
                        other => format!("hosted server {other:?}").to_lowercase(),
                    },
                });
            }
        }
        Ok(out)
    }
}
