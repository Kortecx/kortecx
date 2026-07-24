// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! G2 — the App-pointer → run resolution seam (the `RunApp` path).
//!
//! Apps run **client-orchestrated** today: the client `GetApp`s the envelope,
//! extracts only the `blueprint`, and calls `SubmitWorkflow` — so the envelope's
//! `references.connections` + `guards.secret_scope` are dropped before the gateway
//! sees them, and the run's warrant keeps the fail-closed `SecretScope::None`, which
//! makes a credentialed connector (Gmail/Discord) impossible to dial inside the
//! agentic loop. G2 adds a SERVER-SIDE app-run: the host reads the validated stored
//! envelope from its off-journal `apps.db`, lowers the blueprint through the SAME
//! canonical `kx-blueprint` path the client uses, resolves `references.connections`
//! against the caller's OWN registered connections by name, and sets the run
//! warrant's `SecretScope::AllowList` to the App's declared `guards.secret_scope`
//! (bounded by the referenced connections) so the granted connector's credential can
//! be resolved at dial time.
//!
//! The envelope is server-owned + validated, so the client cannot forge references or
//! run arbitrary steps under an App's credentials (SN-8). gateway-core owns the RPC +
//! the fireable-grant admission + the register/submit tail via the returned
//! [`BoundRecipe`]; the envelope parse (`kx-app`) + connection/secret resolution live
//! in the host (no envelope type crosses this seam — the dependency wall), mirroring
//! [`crate::RecipeBinder`] / [`crate::WorkflowAuthor`].

use crate::service::BoundRecipe;

/// A failure from [`AppAuthor::author_app`]. The gateway collapses `NotAuthorized` to
/// a UNIFORM `permission_denied` (no existence oracle on the execution surface —
/// mirrors [`crate::BinderError`]); maps `MissingIntegration` to
/// `failed_precondition("missing integration: <name>")` (an actionable,
/// non-oracle error — the App is owned), `InvalidArgs` to `invalid_argument`, and
/// `Internal` to `internal`.
#[derive(Debug)]
pub enum AppRunError {
    /// Unauthorized OR the App is absent / not-owned — collapsed so an unauthorized
    /// caller learns nothing about what exists.
    NotAuthorized,
    /// The stored envelope / entry `args` are malformed or uncompilable (a
    /// client-side authoring error), or `guards.secret_scope` names a credential no
    /// referenced connection provides (the loud mis-authoring guard).
    InvalidArgs(String),
    /// The App references a connection the caller has NOT registered. Carries the
    /// connection/credential name so the gateway can surface an actionable
    /// "register it with `kx connections add`" hint.
    MissingIntegration(String),
    /// The App composes another App (`references.apps`) that the caller does not own, or
    /// that cannot be composed (a hosted App has no blueprint to lower). Carries the
    /// handle plus the reason.
    ///
    /// Actionable rather than a collapsed `NotAuthorized` for the SAME reason as
    /// `MissingIntegration`: the App doing the composing is the caller's own, so naming
    /// the dependency it cannot resolve leaks nothing the caller did not already write.
    UncomposableApp {
        /// The callee handle the composing App named.
        handle: String,
        /// Why it cannot be composed.
        reason: String,
    },
    /// The App's `steering_config.model.model_route` (or a per-step model id) names a
    /// model this serve does not offer (empty intersection with the served catalog).
    /// Carries the requested route so the gateway can surface which model is missing.
    /// Fail-closed REFUSE at submit — the runtime never silently substitutes a model
    /// the App did not name (SN-8: the user names the model; no auto-select).
    UnservedModelRoute(String),
    /// An internal failure (storage / lowering).
    Internal(String),
}

/// The G2 App-run seam (the `RunApp` path). The host resolves a caller-owned App
/// `handle` (+ optional entry `args`) for the SERVER-DERIVED `party` into a runnable,
/// least-privilege [`BoundRecipe`] whose warrants already carry the App's resolved
/// secret scope. It does NO journal write (that is the `RunSubmitter`'s job). A `None`
/// seam on the service ⇒ `RunApp` returns `unimplemented` (clients then fall back to
/// the legacy `GetApp` → `SubmitWorkflow` path — no regression).
#[tonic::async_trait]
pub trait AppAuthor: Send + Sync {
    /// Resolve the caller-owned App `handle` (+ optional entry `args`) into a runnable
    /// [`BoundRecipe`].
    ///
    /// `require_approval` (T-APP-TRIGGER-TARGET / D114): when `true`, the entry agentic
    /// step is authored with the HITL posture so the coordinator withholds irreversible
    /// (world-mutating) tool actions until an operator grant. `false` ⇒ byte-identical to
    /// today (the coordinator falls back to the serve-wide `KX_SERVE_REQUIRE_APPROVAL`
    /// default). Injected at author time (into the launch Mote's config, BEFORE the DAG is
    /// lowered) — never post-hoc, which would change the MoteId and orphan its edges.
    ///
    /// # Errors
    /// [`AppRunError`] — see the variants.
    async fn author_app(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
        require_approval: bool,
    ) -> Result<BoundRecipe, AppRunError>;
}
