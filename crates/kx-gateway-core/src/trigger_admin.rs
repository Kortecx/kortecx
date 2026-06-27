//! The trigger admin seam (D113 — `RegisterTrigger` / `ListTriggers` /
//! `DeregisterTrigger` / `SubmitTrigger` / `TestTrigger`).
//!
//! Spoken in gateway-core's OWN vocabulary (`String` / `[u8; N]` / `bool`) — no host
//! type crosses the seam, the [`crate::mcp_gateway_admin::McpGatewayAdmin`] pattern.
//! The host (`kx-gateway`) implements it over the `triggers.db` sidecar + the SAME
//! `RecipeBinder` + `RunSubmitter` the Invoke path uses, so an inbound event starts a
//! run through the exact propose-proxy (the coordinator stays the sole journal writer;
//! the frozen trio is untouched). The host webhook/cron LISTENERS call [`TriggerAdmin::submit`]
//! directly too — one event→run path, shared by the gRPC handler and the listeners.
//!
//! # Boundaries (SN-8 / D102.2 / D113)
//! - **Server-derived id + owner.** `trigger_id` is derived from the name; the run binds
//!   under the REGISTRANT's party (the trigger cannot escalate beyond what its registrant
//!   could Invoke).
//! - **Idempotency.** A replayed event (same `idempotency_key`) is a no-op: it returns the
//!   prior run and fires nothing (`deduped = true`).
//! - **Secret-less.** The webhook auth secret is referenced by NAME only (resolved from
//!   the keychain at verify time); the value never crosses this seam or the journal.
//! - **`None` seam ⇒ `unimplemented`.** The hosted multi-tenant trigger gateway is CLOUD.

/// A `RegisterTrigger` request, in gateway-core vocabulary. `kind`/`auth` are validated
/// strings (`"webhook"|"cron"|"grpc"`, `"none"|"hmac_sha256"|"bearer"`) so the seam stays
/// decoupled from the proto enums.
#[derive(Clone, Debug)]
pub struct TriggerRegistration {
    /// Unique operator handle (derives the server-side `trigger_id`).
    pub name: String,
    /// `"webhook"` | `"cron"` | `"grpc"`.
    pub kind: String,
    /// The `kx/recipes/…` handle the event Invokes.
    pub recipe_handle: String,
    /// `"none"` | `"hmac_sha256"` | `"bearer"` (the webhook auth posture).
    pub auth: String,
    /// SecretRef NAME of the webhook auth secret (never the value); `""` ⇒ none.
    pub auth_secret_ref: String,
    /// Cron: the interval in seconds (e.g. `"300"`); empty otherwise.
    pub schedule_spec: String,
    /// Whether the trigger is active (a disabled trigger refuses to fire).
    pub enabled: bool,
    /// The server-derived registrant party the fired run binds authority under (D102.2).
    pub owner_party: String,
}

/// One registered trigger, the `ListTriggers` governance row. Never a secret value.
#[derive(Clone, Debug)]
pub struct TriggerView {
    /// 16-byte server-derived trigger id.
    pub trigger_id: [u8; 16],
    /// The operator handle.
    pub name: String,
    /// `"webhook"` | `"cron"` | `"grpc"`.
    pub kind: String,
    /// The recipe handle the event Invokes.
    pub recipe_handle: String,
    /// `"none"` | `"hmac_sha256"` | `"bearer"`.
    pub auth: String,
    /// Whether an auth-secret ref NAME is attached (never the value, D81).
    pub auth_secret_present: bool,
    /// Cron interval seconds (empty for non-cron).
    pub schedule_spec: String,
    /// Whether the trigger is active.
    pub enabled: bool,
    /// Last-fired wall-clock (ms since epoch); 0 ⇒ never fired.
    pub last_fire_unix_ms: u64,
}

/// The outcome of an inbound event ([`TriggerAdmin::submit`]).
#[derive(Clone, Debug)]
pub struct TriggerFireOutcome {
    /// The registered run (the PRIOR run's id when `deduped`).
    pub instance_id: [u8; 16],
    /// `true` ⇒ a prior identical event already started this run (fired nothing).
    pub deduped: bool,
}

/// Why a [`TriggerAdmin`] operation was refused.
#[derive(Debug, thiserror::Error)]
pub enum TriggerAdminError {
    /// A malformed request field. Maps to `invalid_argument`.
    #[error("invalid trigger: {0}")]
    InvalidArgument(String),
    /// No trigger with the given name. Maps to `not_found`.
    #[error("no such trigger: {0}")]
    NotFound(String),
    /// The bound recipe refused authority (uniform — no existence oracle). Maps to
    /// `permission_denied`.
    #[error("not authorized")]
    NotAuthorized,
    /// The serve cannot run this trigger (e.g. a react recipe with no inference executor,
    /// or a disabled trigger). Maps to `failed_precondition`.
    #[error("trigger unsupported: {0}")]
    Unsupported(String),
    /// A durable-store / submission failure. Maps to `internal`.
    #[error("trigger store error: {0}")]
    Storage(String),
}

/// The trigger admin seam behind the 5 D113 RPCs. Async (consistent with
/// [`crate::RecipeBinder`] / [`crate::submit::RunSubmitter`]) because [`Self::submit`]
/// binds + submits a run. The host impl owns the `triggers.db` store + the binder +
/// submitter. A `None` seam ⇒ the RPCs return `unimplemented`.
#[tonic::async_trait]
pub trait TriggerAdmin: Send + Sync {
    /// Register (or replace) a trigger. Returns the 16-byte server-derived id.
    async fn register(&self, reg: TriggerRegistration) -> Result<[u8; 16], TriggerAdminError>;

    /// List triggers (deterministic `(name)` order), keyset-paged after `after_name`.
    async fn list(
        &self,
        limit: u32,
        after_name: &str,
    ) -> Result<(Vec<TriggerView>, bool), TriggerAdminError>;

    /// Deregister a trigger by name. Returns `true` iff removed.
    async fn deregister(&self, name: &str) -> Result<bool, TriggerAdminError>;

    /// Fire an inbound event: dedup on `idempotency_key` (empty ⇒ server-derived from
    /// the payload), then — for a fresh key — bind the recipe under the trigger's owner
    /// party with `payload_json` as the args (passthrough) and start a run via the Invoke
    /// propose-proxy. A duplicate fires nothing and returns the prior run.
    async fn submit(
        &self,
        name: &str,
        idempotency_key: &str,
        payload_json: &str,
    ) -> Result<TriggerFireOutcome, TriggerAdminError>;

    /// Dry-run: validate the binding (handle resolves, payload binds) WITHOUT firing.
    /// Returns `(ok, detail)`.
    async fn test(
        &self,
        name: &str,
        payload_json: &str,
    ) -> Result<(bool, String), TriggerAdminError>;
}
