//! The `GetAppManifest` seam — a READ-ONLY capability manifest for a stored App
//! ("what this App needs vs. what you have").
//!
//! An App declares the tools / connections / model it wants; the runtime grants only
//! the intersection with the caller's own authority at run time (SN-8). The manifest
//! is the DERIVED preview of that intersection, computed by the host from the stored
//! envelope + the SAME live policy folds `RunApp` uses — so it can never report a
//! capability "in policy" that the run would drop. It is advisory: it gates nothing,
//! writes nothing, and is off-journal / off-digest (recomputed on demand).
//!
//! # Boundaries (load-bearing)
//! - **Server-authoritative + DERIVE-never-store.** The "have" side (which tools are
//!   fireable, which models are served, which connections are registered) is live
//!   host state a client cannot see; the host owns both the envelope parse AND the
//!   policy folds, so the diff is computed once, server-side. No envelope type crosses
//!   this seam (the dependency wall) — only the already-computed diff, in gateway-core's
//!   own vocabulary.
//! - **Caller-scoped.** Takes the SERVER-RESOLVED `principal`; uniform `Ok(None)` for an
//!   absent OR not-owned handle (no cross-party existence oracle — mirrors `AppCatalog`).
//! - **`None` seam ⇒ degrade.** A host without the seam leaves `GetAppManifest`
//!   `unimplemented`; clients then fall back to an envelope-only "needs" view.

use crate::error::GatewayError;

/// One capability line in an [`AppManifest`]. Opaque primitives only (the dependency
/// wall). For a tool, `id`/`version` are the registry id + version; for a connection,
/// `id` is the descriptor and `version` is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppCapability {
    /// Tool id (e.g. `mcp-echo/echo`) or connection descriptor.
    pub id: String,
    /// Tool version; empty for a connection.
    pub version: String,
    /// The App named this capability (via its references / steering wish).
    pub requested: bool,
    /// The capability is within the caller's resolvable policy (a fireable+registered
    /// tool, or a registered connection).
    pub in_policy: bool,
    /// The capability surfaced ONLY because the tool axis is `reach = InheritPrincipal`
    /// (inherited from the caller's ceiling, not explicitly requested).
    pub inherited: bool,
}

/// The server-computed, READ-ONLY manifest for one stored App. Gates nothing;
/// off-journal / off-digest; recomputed on demand from the stored envelope + the live
/// policy folds. A capability with `requested && !in_policy` is the "missing" set the
/// caller must satisfy (register the connection / serve the model) before a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppManifest {
    /// The effective tool reach — `true` when the App inherits the caller's whole
    /// tool ceiling (`reach = InheritPrincipal`) rather than an explicit wish.
    pub reach_inherit: bool,
    /// The tool capability lines (requested wish ∪ the inherited ceiling).
    pub tools: Vec<AppCapability>,
    /// The connection capability lines (`references.connections` vs. the registered set).
    pub connections: Vec<AppCapability>,
    /// The dataset capability lines (`references.datasets` ∪ steering dataset refs vs. the
    /// ingested corpora). A declared dataset that is neither self-contained nor ingested is
    /// the ONE dependency that HARD-FAILS `RunApp` (`AppRunError::InvalidArgs`), so it is the
    /// one preflight must surface: `requested && !in_policy` ⇒ the run would refuse.
    pub datasets: Vec<AppCapability>,
    /// The App's declared model route (empty ⇒ the served default is used).
    pub model_route: String,
    /// Whether `model_route` is offered by this serve (always `true` when empty). When
    /// `false`, a run would REFUSE — the manifest surfaces it before the run.
    pub model_route_served: bool,
}

/// The `GetAppManifest` seam: derive the READ-ONLY capability manifest for a
/// caller-owned App `handle`. The host reuses the SAME policy folds `RunApp` applies,
/// so the manifest and the run agree by construction. A `None` seam on the service ⇒
/// `GetAppManifest` returns `unimplemented`.
pub trait AppManifestView: Send + Sync {
    /// Compute the manifest for `(principal, handle)`, if the App exists + is owned by
    /// the caller (uniform `Ok(None)` for absent OR not-owned — no existence oracle).
    ///
    /// # Errors
    /// A host read / resolution failure ([`GatewayError::Internal`]), or
    /// [`GatewayError::NotAuthorized`] if the caller may not resolve its own policy.
    fn manifest(&self, principal: &str, handle: &str) -> Result<Option<AppManifest>, GatewayError>;
}
