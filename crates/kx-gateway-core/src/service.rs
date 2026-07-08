//! [`GatewayService`] — the [`KxGateway`] tonic implementation. Read RPCs fold
//! through the read-only seam; `SubmitRun` and `Invoke` proxy through the
//! [`RunSubmitter`]; the signature RPCs and `Invoke` dispatch to the optional
//! [`SignatureCatalog`] / [`RecipeBinder`] seams the host injects (each returns
//! `unimplemented` when its seam is absent — backward-compatible).

use std::pin::Pin;
use std::sync::Arc;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_server::KxGateway;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::capture_view::CaptureView;
use crate::datasets::DatasetView;
use crate::error::{hash_32, instance_id_16, GatewayError};
use crate::fuzzy_discovery::FuzzyDiscoveryView;
use crate::identity::CallerParty;
use crate::memory::MemoryView;
use crate::reader::{ContentReader, JournalReader};
use crate::submit::{RunSubmitter, SubmitterError};
use crate::{events, view};

/// The id a `RegisterSignature` server-derived from the manifest bytes (SN-8:
/// the client never supplies the id; the host derives it from the decoded entry).
#[derive(Clone, Copy, Debug)]
pub struct RegisteredSignature {
    /// The 32-byte content-addressed signature id.
    pub signature_id: [u8; 32],
}

/// One entry in a `ListSignatures` enumeration: the content-addressed id plus a
/// host-derived human label.
#[derive(Clone, Debug)]
pub struct SignatureSummaryEntry {
    /// The 32-byte content-addressed signature id.
    pub signature_id: [u8; 32],
    /// A short, stable, human-distinguishable label (the catalog stores no name
    /// of its own; a richer name belongs in advisory metadata later).
    pub name: String,
}

/// A failure from the [`SignatureCatalog`] seam.
///
/// The catalog is a PUBLIC discovery surface (authoritative for *what recipes
/// exist*), so — unlike the `Invoke` execution surface, which collapses to a
/// uniform "not authorized" with no existence oracle — these stay honest,
/// distinct codes.
#[derive(Debug)]
pub enum CatalogSeamError {
    /// A DIFFERENT entry already exists at this content-addressed id.
    ImmutabilityConflict,
    /// The `manifest` bytes could not be decoded into a signature entry.
    Malformed(String),
    /// A backend storage failure (durable-backend I/O, a corrupt row).
    Internal(String),
}

/// The signature-catalog seam (the M7 catalog RPCs frozen at D120).
///
/// Spoken in the gateway's WIRE vocabulary — opaque `manifest` bytes + a 32-byte
/// server-derived id — so gateway-core stays off `kx-catalog` (the
/// dependency wall). The host implements it over a `kx_catalog::CatalogRegistry`,
/// decoding/encoding with the catalog's canonical codec and server-deriving the
/// id from the decoded entry. A `None` seam on the service means the host wired
/// no catalog, so the three signature RPCs return `unimplemented`.
pub trait SignatureCatalog: Send + Sync {
    /// Decode `manifest`, server-derive its id, register it (idempotent +
    /// immutable), and return the id.
    ///
    /// # Errors
    /// [`CatalogSeamError`] on a malformed manifest, an immutability conflict, or
    /// a storage failure.
    fn register(&self, manifest: &[u8]) -> Result<RegisteredSignature, CatalogSeamError>;
    /// The encoded manifest for `signature_id`, or `None` if absent.
    fn get(&self, signature_id: &[u8; 32]) -> Option<Vec<u8>>;
    /// Every registered signature as an `(id, name)` summary, in deterministic
    /// (hash) order.
    fn list(&self) -> Vec<SignatureSummaryEntry>;
}

/// The value domain of a recipe free-param, in gateway-core's wire vocabulary
/// (mirrors `kx_tool_registry::ParamType` without depending on it). `Unspecified`
/// is an untyped slot (no schema). There is deliberately no float / json.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeParamKind {
    /// An untyped free-param (the slot declared no schema).
    Unspecified,
    /// A UTF-8 string, bounded by `max_len`.
    Str,
    /// A signed integer.
    Int,
    /// A boolean.
    Bool,
    /// Opaque bytes, bounded by `max_len`.
    Bytes,
    /// A closed set of permitted string values (`allowed`).
    Enum,
}

/// One variable free-param a recipe requires, ready to render as a form field.
/// Constant slots (fixed by the recipe body) are never surfaced. Spoken in
/// gateway-core's own vocabulary so the seam stays off `kx-catalog` (the host
/// maps `kx_catalog::free_params_to_input_schema` output into this).
#[derive(Clone, Debug)]
pub struct RecipeFormFieldEntry {
    /// The JSON arg key (the slot name).
    pub name: String,
    /// The value domain.
    pub kind: RecipeParamKind,
    /// Whether the caller must supply it (variable slots are required).
    pub required: bool,
    /// The max length for [`RecipeParamKind::Str`] / [`RecipeParamKind::Bytes`].
    pub max_len: Option<u64>,
    /// The permitted values for [`RecipeParamKind::Enum`] (else empty).
    pub allowed: Vec<String>,
}

/// The recipe-discovery seam (the UI-2 `ListRecipes` / `GetRecipeForm` path) —
/// the PUBLIC catalog of INVOCABLE recipe handles + each handle's free-param
/// FORM. DISTINCT from [`SignatureCatalog`] (the TaskSignature/verdict registry):
/// these enumerate the handles `Invoke` runs and describe their inputs. Like the
/// signature catalog this is a public discovery surface (honest `not_found`, no
/// existence-oracle collapse — `Invoke` remains the authorization gate). The host
/// implements it over its provisioned recipe library; a `None` seam ⇒ the two
/// recipe RPCs return `unimplemented`.
pub trait RecipeCatalog: Send + Sync {
    /// Every invocable recipe handle (`"namespace/collection/name"`), in a
    /// deterministic order.
    fn list_recipes(&self) -> Vec<String>;
    /// The variable free-param fields for `handle`, or `None` if no such recipe
    /// is provisioned.
    fn get_recipe_form(&self, handle: &str) -> Option<Vec<RecipeFormFieldEntry>>;
    /// The published workflow fingerprint a bound run of `handle` registers
    /// under (`RunSummary.recipe_fingerprint` joins on this — PR-2.1 run
    /// naming). Display/join only, NEVER identity. Defaults to `None` so
    /// existing impls keep compiling (the wire then carries an empty field).
    fn recipe_fingerprint(&self, _handle: &str) -> Option<[u8; 32]> {
        None
    }
    /// The ADVISORY metadata (description / tags / version) for `handle` —
    /// PR-4 Batch D, from the host's `kx_catalog` AdvisoryMetadataStore +
    /// VersionLedger. Display/discovery ONLY, NEVER identity. Defaults to `None`
    /// so existing impls keep compiling (the wire then carries empty fields).
    fn recipe_metadata(&self, _handle: &str) -> Option<RecipeMetadataEntry> {
        None
    }
    /// ADVISORY recipe discovery — rank the provisioned recipes against `intent`
    /// (+ optional `keywords`), best-first, capped at `limit`. PR-4 Batch D, over
    /// the host's `kx_catalog` DiscoveryIndex (fuzzy-in/exact-out). `score_bp` is
    /// DISPLAY-ONLY (integer basis points; a search SURFACES a recipe, never
    /// invokes one — `Invoke` stays the authorization gate). Defaults to `None`
    /// so existing impls keep compiling; `None` ⇒ `SearchRecipes` returns
    /// `unimplemented`.
    fn search_recipes(
        &self,
        _intent: &str,
        _keywords: &[String],
        _limit: usize,
    ) -> Option<Vec<ScoredRecipeEntry>> {
        None
    }
}

/// The advisory metadata for a recipe handle (PR-4 Batch D), in gateway-core's
/// own vocabulary so the seam stays off `kx-catalog` (the dependency wall). The
/// host folds its `kx_catalog` AdvisoryMetadataStore + VersionLedger into these.
/// Display/discovery ONLY — never identity, never an authorization.
#[derive(Clone, Debug, Default)]
pub struct RecipeMetadataEntry {
    /// Free-form human description (never parsed for enforcement).
    pub description: String,
    /// Advisory discovery tags (sorted, deduplicated).
    pub tags: Vec<String>,
    /// Advisory published version label (empty when unversioned).
    pub version: String,
}

/// One recipe's advisory rank against a search intent (PR-4 Batch D). `score_bp`
/// is integer basis points (0..=10000) — DISPLAY-ONLY (the SN-8
/// no-persisted-confidence rule; never a float, never an authorization).
#[derive(Clone, Debug)]
pub struct ScoredRecipeEntry {
    /// The matched recipe handle.
    pub handle: String,
    /// The advisory metadata (description / tags / version).
    pub metadata: RecipeMetadataEntry,
    /// The advisory rank, integer basis points (0..=10000).
    pub score_bp: u32,
}

/// One team in a `ListTeams` enumeration, in gateway-core's wire vocabulary
/// (strings/u32 — no `kx-fleet` type, so the seam stays off the membership crate,
/// the dependency wall). The host folds its `kx_fleet::MembershipLedger` into these.
#[derive(Clone, Debug)]
pub struct TeamSummaryEntry {
    /// The team's group principal id (the party grants are issued to).
    pub team_id: String,
    /// The advisory human handle from the founding fact (never parsed for enforcement).
    pub display_name: String,
    /// The founding owner principal id.
    pub owner: String,
    /// The count of effective (authority-checked) active members.
    pub member_count: u32,
}

/// A compact, human-readable warrant projection — NEVER the warrant body or any
/// secret material; the load-bearing ceilings + scopes a member's resolved warrant
/// conveys, as display strings/scalars (mirrors the `kx` CLI warrant render). The
/// host renders it once from a `kx_warrant::WarrantSpec`; the UI never reconstructs
/// kx-warrant formatting, and a future kx-warrant axis bump never forces a proto change.
#[derive(Clone, Debug)]
pub struct WarrantProjection {
    /// The executor class (e.g. "Bwrap" / "MacOsSandbox").
    pub executor_class: String,
    /// A one-line model route ("model_id ×max_calls (in/out tok)").
    pub model_route: String,
    /// The egress scope summary ("None" / "EgressAllowlist(host:port,…)").
    pub net_scope: String,
    /// The filesystem scope summary ("/path:ro, …").
    pub fs_scope: String,
    /// The headline narrowing axis: `model_route.max_calls`.
    pub max_calls: u64,
    /// The CPU ceiling (`resource_ceiling.cpu_milli`).
    pub cpu_milli: u64,
    /// The wall-clock ceiling (`resource_ceiling.wall_clock_ms`).
    pub wall_clock_ms: u64,
}

/// One member of a team, with the optional resolved-warrant projection.
#[derive(Clone, Debug)]
pub struct TeamMemberEntry {
    /// The member principal id.
    pub party: String,
    /// The merged runtime-scope role name (advisory display).
    pub role: String,
    /// The merged catalog action cap, e.g. `["Read", "Use", "Delegate"]`.
    pub action_caps: Vec<String>,
    /// Present iff the caller passed an `asset_ref` AND a membership path resolves a
    /// warrant for this member on it (the "what would this member actually get" view).
    pub resolved_warrant: Option<WarrantProjection>,
}

/// The members of one team (with the team owner echoed so the UI can mark the owner row).
#[derive(Clone, Debug)]
pub struct TeamMembersView {
    /// The team owner principal id.
    pub owner: String,
    /// The active members, by member principal id (deterministic).
    pub members: Vec<TeamMemberEntry>,
}

/// The membership read seam (the UI-3 `ListTeams` / `ListTeamMembers` path). The host
/// implements it over a `kx_fleet::MembershipLedger` (+ a `GovernedFleet` for the
/// optional resolve), folding `list_facts()` for the team list and `effective_members`
/// for the member view — spoken in gateway-core's wire vocabulary so the seam stays off
/// `kx-fleet` / `kx-catalog`. A `None` seam ⇒ the two team RPCs return `unimplemented`.
pub trait MembershipView: Send + Sync {
    /// Every founded team, in founding order.
    fn list_teams(&self) -> Vec<TeamSummaryEntry>;
    /// The active members of `team_id`, or `None` if no such team is founded. When
    /// `asset_ref` is `Some`, each member's `resolved_warrant` is populated (the
    /// membership ∩ grant fold via the frozen narrowing seam); `None` leaves it unset.
    fn list_members(&self, team_id: &str, asset_ref: Option<&str>) -> Option<TeamMembersView>;
}

/// One grant on an asset, fold-classified (root vs delegated, active vs revoked).
#[derive(Clone, Debug)]
pub struct GrantEntry {
    /// The grantor principal id.
    pub grantor: String,
    /// The grantee principal id.
    pub grantee: String,
    /// The catalog actions the grant conveys, e.g. `["Read", "Use"]`.
    pub actions: Vec<String>,
    /// The grant's runtime-scope role name (advisory display).
    pub runtime_scope: String,
    /// `true` iff this is a root grant (`grant.prior` is `None`) from the asset owner.
    pub is_root: bool,
    /// `true` iff an AUTHORIZED revocation makes the grant inert in the fold.
    pub revoked: bool,
}

/// Every grant on one asset, with the bound owner echoed.
#[derive(Clone, Debug)]
pub struct AssetGrantsView {
    /// The asset's bound owner principal id ("" if unbound).
    pub owner: String,
    /// Every grant fact on the asset (root + delegated), fold-classified.
    pub grants: Vec<GrantEntry>,
}

/// The grant read seam (the UI-3 `ListAssetGrants` path). The host implements it over
/// the SAME `kx_catalog::GrantLedger` the demo recipes seed, classifying each grant
/// fact root/delegated + active/revoked via the fold. A `None` seam ⇒ `ListAssetGrants`
/// returns `unimplemented`; an unknown asset ⇒ `None` (the handler maps to `not_found`).
pub trait GrantView: Send + Sync {
    /// Every grant on `asset_ref`, or `None` if the asset handle is unparseable /
    /// unknown.
    fn list_asset_grants(&self, asset_ref: &str) -> Option<AssetGrantsView>;
}

/// A recipe resolved + bound to concrete args, ready to submit. Mirrors
/// `kx_invoke::BoundRun`, but in gateway-core's own vocabulary (`kx_mote` +
/// `kx_warrant` types it already depends on) so the binding seam stays off
/// kx-invoke / kx-catalog (the dependency wall).
pub struct BoundRecipe {
    /// The recipe identity → the `recipe_fingerprint` passed to `RegisterRun`.
    pub recipe_fingerprint: [u8; 32],
    /// The runnable Motes in submission order, each paired with its narrowed
    /// warrant (⊆ the caller's Use authority AND the recipe's step warrant).
    pub motes: Vec<(kx_mote::Mote, kx_warrant::WarrantSpec)>,
    /// The terminal (sink) Mote whose committed result is the invocation output.
    pub terminal_mote_id: kx_mote::MoteId,
    /// PR-2d-2 (react-tools-live): `true` iff this recipe seeds a live ReAct
    /// chain (`kx/recipes/react`) — the Invoke arm then submits the bound Mote
    /// with `react_seed = true`, triggering the coordinator's seed-swap (the
    /// run-salted turn 0 + the durable anchor). Set ONLY by the host binder for
    /// the react handle (a single-step recipe); every other recipe is `false`
    /// and submits exactly as before.
    pub react_seed: bool,
}

/// A bind failure the host's [`RecipeBinder`] surfaces. The gateway collapses
/// [`BinderError::NotAuthorized`] to a UNIFORM `permission_denied` (no existence
/// oracle on the execution surface); [`BinderError::InvalidArgs`] is the only
/// distinct, caller-actionable code.
#[derive(Debug)]
pub enum BinderError {
    /// Unauthorized OR not-found OR not-a-workflow OR body-unavailable — collapsed
    /// by the host so an unauthorized caller learns nothing about what exists.
    NotAuthorized,
    /// Argument validation / parse / unbound slot / uncompilable / empty recipe.
    InvalidArgs(String),
    /// An internal binder failure (storage, etc.).
    Internal(String),
}

/// The recipe-binding seam (the `Invoke` path). The host implements it with
/// `kx_invoke::bind_snapshot` over its provisioned ledgers + the per-handle
/// free-param contract, resolving the caller's Use authority from the
/// authoritative grant ledger (never a caller-supplied warrant — SN-8). It does
/// NO journal write (that is the [`RunSubmitter`]'s job). A `None` seam on the
/// service ⇒ `Invoke` returns `unimplemented`.
#[tonic::async_trait]
pub trait RecipeBinder: Send + Sync {
    /// Resolve `handle` + `args` for the SERVER-DERIVED `party` into a runnable,
    /// least-privilege [`BoundRecipe`].
    ///
    /// # Errors
    /// [`BinderError`] — `NotAuthorized` (uniform, no oracle) or `InvalidArgs`.
    async fn bind(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
        context_bundles: &[String],
        context_refs: &[String],
    ) -> Result<BoundRecipe, BinderError>;
}

/// The VETTED step palette for `SubmitWorkflow` (Tier-1 authoring). gateway-core's
/// own vocabulary — the host translates each to a `kx_workflow::StepDef`, assigning
/// the `logic_ref` SERVER-SIDE (the client never supplies executable bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorStepKind {
    /// Deterministic transform; the host derives a content sentinel `logic_ref`.
    Pure,
    /// Greedy model step routed by `model_id` + `prompt` (must equal the served model).
    Model,
    /// References a REGISTERED body by its signature id; the host maps it to that
    /// body's content `logic_ref`. The client cannot inject bytes (Tier-1 invariant).
    Exec,
    /// Fires a single REGISTERED tool as a standalone DAG node (PR-6b-2). The host
    /// looks the tool up in the live tool registry, builds its warrant SERVER-SIDE
    /// from the tool's declared `required_capability` (SN-8 — never a client
    /// warrant), and carries the authored args (canonical-JSON) into the step's
    /// `config_subset[TOOL_ARGS_KEY]`. The client supplies only the
    /// `(tool_id, tool_version)` + args; the warrant + identity are server-derived.
    Tool,
}

/// One authored step in gateway-core's vocabulary (no `kx_workflow` dep here).
#[derive(Debug, Clone)]
pub struct AuthorStep {
    /// The palette kind (PURE / MODEL / EXEC / TOOL).
    pub kind: AuthorStepKind,
    /// MODEL: the model id (must equal the served model); ignored otherwise.
    pub model_id: String,
    /// MODEL: the prompt text (bound into the step's config); empty otherwise.
    pub prompt: String,
    /// EXEC: the registered body's content/signature id.
    pub body_signature_id: Option<[u8; 32]>,
    /// The per-step tool contract (`tool_id → tool_version`); authority is the warrant's.
    pub tool_contract: std::collections::BTreeMap<String, String>,
    /// Free config entries that land in the step's `config_subset` (identity-bearing).
    pub params: std::collections::BTreeMap<String, Vec<u8>>,
}

/// One authored edge (parent/child are indices into the authored `steps`).
#[derive(Debug, Clone, Copy)]
pub struct AuthorEdge {
    /// The parent step's index in `steps`.
    pub parent: u32,
    /// The child step's index in `steps`.
    pub child: u32,
    /// `true` = a DATA edge (parent result feeds the child); `false` = a CONTROL edge.
    pub data: bool,
    /// CONTROL-only cascade opt-out (`EdgeMeta::non_cascade`).
    pub non_cascade: bool,
}

/// The blueprint execution mode (`blueprint-execution-modes.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorExecutionMode {
    /// Memoize/reuse the content-addressed DAG (today's behavior).
    Frozen,
    /// Re-plan-fresh each run (reserved — PR-1 refuses it fail-closed).
    Dynamic,
}

/// The DAG-authoring seam (the `SubmitWorkflow` path). The host compiles an
/// authored Tier-1 DAG via `kx_workflow::compile`, assigns each step's `logic_ref`
/// from its [`AuthorStepKind`], and resolves + INTERSECTS every warrant SERVER-SIDE
/// from the party's grants (never a client warrant — the SubmitRun BLOCKER #5
/// lesson). It does NO journal write (that is the [`RunSubmitter`]'s job). A `None`
/// seam ⇒ `SubmitWorkflow` returns `unimplemented`. Reuses [`BoundRecipe`] (the
/// `react_seed` field is always `false`) + [`BinderError`].
#[tonic::async_trait]
pub trait WorkflowAuthor: Send + Sync {
    /// Compile + warrant-resolve an authored DAG for the SERVER-DERIVED `party`.
    ///
    /// # Errors
    /// [`BinderError`] — `NotAuthorized` (uniform, no oracle), `InvalidArgs`
    /// (malformed shape / over-cap / unknown body / wrong model / dynamic mode).
    async fn author(
        &self,
        party: &str,
        seed: u32,
        steps: &[AuthorStep],
        edges: &[AuthorEdge],
        mode: AuthorExecutionMode,
        context_bundles: &[String],
    ) -> Result<BoundRecipe, BinderError>;
}

/// The LIVE broker-fireable tool set seam (PR-6b-2 — the authoring backstop's
/// truth source). The host impl wraps the serve broker so a runtime-dialed
/// external MCP tool becomes authorable (a `tool()` step / an auto-grant) the
/// MOMENT its firing capability registers, never gated by a startup snapshot.
///
/// `None` on the service ⇒ the backstop falls back to the static
/// `GatewayService::registered_tools` set (the PR-2d-2 behaviour — bundled
/// tools only). The VIEW never authorizes (SN-8): it is a belt-and-braces
/// fail-closed gate that refuses authoring a warrant granting a tool the broker
/// cannot fire; the broker's own 6-gate `precheck` re-verifies at dispatch.
pub trait RegisteredToolsView: Send + Sync {
    /// The `(tool_id, tool_version)` of every tool whose firing capability is
    /// CURRENTLY registered on the serve broker.
    fn registered_grants(&self) -> std::collections::BTreeSet<(String, String)>;
}

/// The boxed server-streaming type the `StreamEvents` RPC returns.
pub type EventStream =
    Pin<Box<dyn Stream<Item = Result<proto::EventFrame, Status>> + Send + 'static>>;

/// The event-tailing seam behind `StreamEvents`. The default [`SnapshotTailer`]
/// emits the deltas in `(since_seq, head]` once and ends (snapshot-to-head); the
/// host can inject a LIVE tailer (R5 — `kx-gateway`'s `LiveTailer`) that keeps the
/// stream open and emits frames as the journal advances. Spoken in gateway-core's
/// own vocabulary (a [`JournalReader`] + the frozen [`EventFrame`](proto::EventFrame))
/// so the live tailer lives in the binary WITHOUT putting a runtime/timer dep on
/// the read-fold crate (the dep wall).
pub trait EventTailer: Send + Sync {
    /// Open the event stream for `(instance_id, since_seq)`. `reader` is owned
    /// (`Arc`) so a tailer that spawns a poller can outlive the handler call. The
    /// ownership check is the tailer's first action.
    ///
    /// # Errors
    /// A uniform `permission_denied` if the caller does not own the run (no
    /// existence oracle); `internal` on a read/fold failure.
    // The Ok variant is a thin boxed stream while `tonic::Status` is large, which
    // trips `result_large_err`; boxing the Status would force every caller to
    // unbox to satisfy the tonic handler's own `Result<_, Status>`. A clean
    // pre-stream ownership error (vs. an in-band error frame) is the right
    // semantics, so allow the lint on this seam.
    #[allow(clippy::result_large_err)]
    fn stream(
        &self,
        reader: Arc<dyn JournalReader>,
        instance_id: [u8; 16],
        since_seq: u64,
    ) -> Result<EventStream, Status>;
}

/// The default, dependency-free tailer: emit `(since_seq, head]` once, then END
/// (snapshot-to-head). This was gateway-core's behavior before R5; it is kept as
/// the default so the crate stays self-contained and its round-trip tests need no
/// async runtime. A live tail is opt-in via [`GatewayService::with_event_tailer`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SnapshotTailer;

impl EventTailer for SnapshotTailer {
    #[allow(clippy::result_large_err)] // see the trait method.
    fn stream(
        &self,
        reader: Arc<dyn JournalReader>,
        instance_id: [u8; 16],
        since_seq: u64,
    ) -> Result<EventStream, Status> {
        let frames = events::build_frames(reader.as_ref(), instance_id, since_seq)?;
        Ok(Box::pin(tokio_stream::iter(frames.into_iter().map(Ok))))
    }
}

/// The boxed server-streaming type the `StreamModelTokens` RPC returns (PR-4.2).
pub type TokenStream =
    Pin<Box<dyn Stream<Item = Result<proto::TokenChunk, Status>> + Send + 'static>>;

/// The token-tailing seam behind `StreamModelTokens` (PR-4.2 / T-STREAM1). The
/// default [`NoTokenTailer`] returns an immediately-ending EMPTY stream — the
/// FFI-free / broker-unwired serve has no live tokens, which is honest (never an
/// error). The host injects a broker-backed tailer (`kx-gateway`'s
/// `LiveTokenTailer`) that subscribes the per-mote ADVISORY stream. Spoken in
/// gateway-core's vocabulary (a [`JournalReader`] for the ownership fold + the
/// frozen [`TokenChunk`](proto::TokenChunk)) so the live tailer lives in the
/// binary WITHOUT a runtime dep on the read-fold crate (the dep wall — the
/// [`EventTailer`] posture).
pub trait TokenTailer: Send + Sync {
    /// Open the token stream for `(instance_id, mote_id, since_seq)`. The
    /// ownership gate — the caller owns `instance_id` (the `StreamEvents`
    /// run-ownership precedent) — is the tailer's first action (uniform
    /// `permission_denied`, no existence oracle). `mote_id` is the broker key (a
    /// server-derived, unguessable id), NOT a second journal gate: a
    /// freshly-submitted terminal mote is not journaled when a client subscribes
    /// for time-to-first-token. `since_seq` is an advisory replay cursor into the
    /// broker's per-mote history.
    ///
    /// # Errors
    /// A uniform `permission_denied` if the caller does not own the run;
    /// `internal` on a read/fold failure.
    // Same `result_large_err` rationale as `EventTailer::stream`.
    #[allow(clippy::result_large_err)]
    fn stream(
        &self,
        reader: Arc<dyn JournalReader>,
        instance_id: [u8; 16],
        mote_id: [u8; 32],
        since_seq: u64,
    ) -> Result<TokenStream, Status>;
}

/// The default, dependency-free token tailer: an immediately-ending EMPTY stream.
/// The FFI-free / broker-unwired serve surfaces no live tokens — honest, never an
/// error (a client reconciles to the committed `result_ref` it polls regardless).
/// A live tail is opt-in via [`GatewayService::with_token_tailer`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTokenTailer;

impl TokenTailer for NoTokenTailer {
    #[allow(clippy::result_large_err)] // see the trait method.
    fn stream(
        &self,
        _reader: Arc<dyn JournalReader>,
        _instance_id: [u8; 16],
        _mote_id: [u8; 32],
        _since_seq: u64,
    ) -> Result<TokenStream, Status> {
        Ok(Box::pin(tokio_stream::iter(std::iter::empty())))
    }
}

/// The boxed server-streaming type the `StreamAllEvents` RPC returns (Batch C).
pub type GlobalEventStream =
    Pin<Box<dyn Stream<Item = Result<proto::GlobalEventFrame, Status>> + Send + 'static>>;

/// The GLOBAL event-tailing seam behind `StreamAllEvents` (Batch C — the
/// [`EventTailer`] twin, run-unscoped). The default [`SnapshotGlobalTailer`]
/// emits `(since_seq, head]` once and ends; the host injects a live tailer
/// (`kx-gateway`'s `GlobalLiveTailer`) that keeps the stream open. NO ownership
/// gate by design: the surface is operator-global on single-node OSS (the host
/// auth interceptor is the gate; CLOUD must party-scope or deny — the proto
/// flag on `StreamAllEventsRequest`).
pub trait GlobalEventTailer: Send + Sync {
    /// Open the global event stream from `since_seq`. `reader` is owned (`Arc`)
    /// so a tailer that spawns a poller can outlive the handler call.
    ///
    /// # Errors
    /// `internal` on a read/fold failure.
    // Same `result_large_err` rationale as `EventTailer::stream`.
    #[allow(clippy::result_large_err)]
    fn stream_all(
        &self,
        reader: Arc<dyn JournalReader>,
        since_seq: u64,
    ) -> Result<GlobalEventStream, Status>;
}

/// The default, dependency-free global tailer: emit `(since_seq, head]` once,
/// then END (snapshot-to-head — the [`SnapshotTailer`] twin). A live tail is
/// opt-in via [`GatewayService::with_global_event_tailer`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SnapshotGlobalTailer;

impl GlobalEventTailer for SnapshotGlobalTailer {
    #[allow(clippy::result_large_err)] // see the trait method.
    fn stream_all(
        &self,
        reader: Arc<dyn JournalReader>,
        since_seq: u64,
    ) -> Result<GlobalEventStream, Status> {
        let frames = events::build_global_frames(reader.as_ref(), since_seq)?;
        Ok(Box::pin(tokio_stream::iter(frames.into_iter().map(Ok))))
    }
}

/// The backend behind the external `KxGateway` service: a read-only journal +
/// content reader (the read-fold) and a [`RunSubmitter`] (the propose-proxy).
/// Holds no writer; auth/ownership stay cloud-side (the host wraps this with
/// middleware). Construct with [`GatewayService::new`]; wire the optional
/// catalog seam with [`GatewayService::with_signature_catalog`].
#[derive(Clone)]
pub struct GatewayService {
    reader: Arc<dyn JournalReader>,
    submitter: Arc<dyn RunSubmitter>,
    content: Arc<dyn ContentReader>,
    /// The optional signature-catalog seam (the host injects a concrete catalog).
    /// `None` ⇒ the three signature RPCs return `unimplemented`.
    catalog: Option<Arc<dyn SignatureCatalog>>,
    /// The optional recipe-binding seam (the host injects a kx-invoke-backed
    /// binder). `None` ⇒ `Invoke` returns `unimplemented`.
    binder: Option<Arc<dyn RecipeBinder>>,
    /// The optional recipe-discovery seam (the host injects a library-backed
    /// catalog). `None` ⇒ `ListRecipes` / `GetRecipeForm` return `unimplemented`.
    catalog_recipes: Option<Arc<dyn RecipeCatalog>>,
    /// The optional membership-view seam (the host injects a `kx-fleet`-backed view).
    /// `None` ⇒ `ListTeams` / `ListTeamMembers` return `unimplemented`.
    membership: Option<Arc<dyn MembershipView>>,
    /// The optional grant-view seam (the host injects a `kx-catalog`-backed view).
    /// `None` ⇒ `ListAssetGrants` returns `unimplemented`.
    grants_view: Option<Arc<dyn GrantView>>,
    /// The optional dataset-view seam (the host injects a `kx-dataset-hnsw`-backed
    /// view behind the `hnsw` feature). `None` ⇒ `ListDatasets` / `IngestDocuments`
    /// / `QueryDataset` return `unimplemented`.
    datasets: Option<Arc<dyn DatasetView>>,
    /// The optional fuzzy-discovery seam (Slice-B). The host injects the SAME
    /// `kx-dataset-hnsw`-backed view as `datasets` (one `Arc`, two seams). `None`
    /// ⇒ `FuzzyDiscovery` returns `unimplemented`. Advisory, off the journal/digest.
    fuzzy: Option<Arc<dyn FuzzyDiscoveryView>>,
    /// The optional durable-MEMORY seam (RC5a — the host injects a `kx-memory`-backed
    /// view behind the `hnsw` feature). `None` ⇒ `StoreMemory` / `ListMemories` /
    /// `RecallMemory` / `ForgetMemory` return `unimplemented`. Off the journal/digest
    /// (memory.db is a rebuildable sidecar); namespace-scoped per caller principal.
    memory: Option<Arc<dyn MemoryView>>,
    /// The optional capture-view seam (the Morphic Data Engine — the host injects
    /// a `capture.db`-backed view folded from the journal). `None` ⇒
    /// `ListCaptureRecords` returns `unimplemented`. Read-only, off-truth-path.
    capture: Option<Arc<dyn CaptureView>>,
    /// The `StreamEvents` tailer. Defaults to [`SnapshotTailer`]; the host injects
    /// a live tailer via [`GatewayService::with_event_tailer`].
    tailer: Arc<dyn EventTailer>,
    /// The `StreamAllEvents` GLOBAL tailer (Batch C). Defaults to
    /// [`SnapshotGlobalTailer`]; the host injects a live tailer via
    /// [`GatewayService::with_global_event_tailer`].
    global_tailer: Arc<dyn GlobalEventTailer>,
    /// The `StreamModelTokens` tailer (PR-4.2 / T-STREAM1). Defaults to the
    /// [`NoTokenTailer`] empty stream; the host injects a broker-backed
    /// `LiveTokenTailer` via [`GatewayService::with_token_tailer`] on the
    /// inference build. ADVISORY + out-of-band — never journal / digest.
    token_tailer: Arc<dyn TokenTailer>,
    /// The optional telemetry-view seam (Batch C — the host injects a
    /// `telemetry.db`-backed view of execution exhaust). `None` ⇒
    /// `ListMoteTelemetry` returns `unimplemented`. Read-only, off-truth-path,
    /// rebuildable-to-empty.
    telemetry: Option<Arc<dyn crate::telemetry_view::TelemetryView>>,
    /// Whether this serve build can EVALUATE a native deterministic critic
    /// (PR-2c-3 critic-live, H5). The verdict arm lives in the inference-build
    /// executor; on a serve that lacks it, a critic Mote would commit echo bytes and
    /// the P4.2-3 exit gate would withhold the producer's consumers FOREVER. So when
    /// this is `false`, `SubmitRun` REFUSES a critic-bearing workflow fail-closed
    /// (rather than admitting a guaranteed deadlock). The host sets it `true` via
    /// [`GatewayService::with_critics_supported`] only when it wires the critic-capable
    /// executor. Defaults to `false` (conservative).
    critics_supported: bool,
    /// Whether this serve build can DRIVE a live ReAct chain (PR-2d-2 — the
    /// `critics_supported` twin, the B3/H5 mirror). The react decode/fence arm
    /// lives in the inference-build executor; on a serve that lacks it, a
    /// `react_seed` submit would echo-commit fake turns and the chain would
    /// settle a meaningless Answer. `false` ⇒ `SubmitRun` REFUSES react seeds
    /// fail-closed. Set via [`GatewayService::with_react_supported`]; defaults
    /// to `false` (conservative).
    react_supported: bool,
    /// The `(tool_id, tool_version)` pairs whose capabilities the host has
    /// ACTUALLY registered on the serve broker (PR-2d-2). The Invoke admission
    /// refuses a bound warrant granting a tool outside this set — a grant the
    /// broker cannot honour would dead-letter every observation (belt-and-braces
    /// over the provisioning invariant; the react recipe is only seeded when its
    /// tool registered). Empty by default (no tools — every grant refused).
    registered_tools: std::collections::BTreeSet<(String, String)>,
    /// PR-6b-2: the optional LIVE broker-fireable set seam. When wired (the serve
    /// path), the authoring backstop reads this instead of the static
    /// `registered_tools` snapshot — so a runtime-DIALED external MCP tool becomes
    /// authorable the moment its capability registers. `None` ⇒ the static set.
    registered_tools_view: Option<Arc<dyn RegisteredToolsView>>,
    /// The optional advisory toolscout seam (W1.A5 — the host injects a
    /// registry-backed manifest index). `None` ⇒ `ListToolManifests` /
    /// `ScoreTaskBundle` return `unimplemented`. Read-only, display-only.
    toolscout: Option<Arc<dyn crate::toolscout_view::ToolScoutView>>,
    /// The optional DAG-authoring seam (the Blueprint builder — the host injects a
    /// `kx-workflow`-backed author). `None` ⇒ `SubmitWorkflow` returns `unimplemented`.
    author: Option<Arc<dyn WorkflowAuthor>>,
    /// The optional content-write seam (Batch A `PutContent` — a content-store
    /// write, NEVER a journal write). `None` ⇒ `PutContent` returns
    /// `unimplemented`.
    content_writer: Option<Arc<dyn crate::writer::ContentWriter>>,
    /// The optional uploads ledger (the Batch A advisory audit sidecar + the
    /// EMPTY-`instance_id` uploads-scope authorized set). `None` ⇒ `PutContent`
    /// returns `unimplemented` and uploads-scope reads uniformly deny.
    uploads: Option<Arc<dyn crate::uploads::UploadsLedger>>,
    /// The fail-closed `PutContent` payload cap in bytes (checked BEFORE the
    /// store is touched). Defaults to [`DEFAULT_PUT_CAP_BYTES`]; the host wires
    /// `kx serve --content-max-bytes`.
    put_cap_bytes: u64,
    /// The optional model-discovery seam (Batch A `ListModels` — display only,
    /// the toolscout advisory precedent). `None` ⇒ `ListModels` returns
    /// `unimplemented`; an EMPTY catalog is the honest FFI-free answer.
    models: Option<Arc<dyn crate::models_view::ModelCatalogView>>,
    /// POC-3 (Models "Local lifecycle"): the optional model-lifecycle CONTROL seam
    /// behind `LoadModel`/`OffloadModel` (warm/evict the registered set's RAM
    /// residency). `None` ⇒ both RPCs return `unimplemented` (the `GetServerInfo`
    /// precedent). Off-journal, off-digest — pure ephemeral RAM state.
    model_lifecycle: Option<Arc<dyn crate::model_lifecycle::ModelLifecycleControl>>,
    /// POC-1 (Settings "Workspace"): the NON-SECRET server-configuration facts the
    /// host projects via `GetServerInfo`. `None` ⇒ `GetServerInfo` returns
    /// `unimplemented`. A plain value (not a live view) — fixed at serve startup;
    /// it carries no secret by construction (no token / TLS-key field).
    server_info: Option<crate::server_info::ServerInfoFacts>,
    /// The optional def-resolution seam (Batch B `GetMoteDetail` — display
    /// only). `None` ⇒ `GetMoteDetail` returns `unimplemented`; an absent def
    /// blob through a WIRED seam is the honest `def_found = false`.
    mote_defs: Option<Arc<dyn crate::mote_def_view::MoteDefView>>,
    /// The optional user-feedback store seam (PR-4.1 — the host injects a
    /// `feedback.db`-backed 👍/👎 sidecar). `None` ⇒ `SubmitFeedback` /
    /// `ListFeedback` return `unimplemented`. Client-origin product signal:
    /// off-journal, off-digest, off-identity, rebuildable-to-empty.
    feedback: Option<Arc<dyn crate::feedback_view::FeedbackStore>>,
    /// The optional run-inputs store seam (PR-D "Re-run with changes" — the host
    /// injects a `run_inputs.db`-backed args-capture sidecar). `None` ⇒
    /// `GetRunInputs` returns `unimplemented` and the `Invoke` capture is skipped.
    /// Off-journal, off-digest, off-identity, rebuildable-to-empty.
    run_inputs: Option<Arc<dyn crate::run_inputs_view::RunInputsStore>>,
    /// The optional alerts-inbox view seam (W1a-2 — the host injects an
    /// `alerts.db`-backed read-cache folded from the journal's terminal `Failed`
    /// facts). `None` ⇒ `ListAlerts` returns `unimplemented`. Read-only,
    /// off-truth-path, rebuildable. The triage LIFECYCLE (acknowledge/resolve),
    /// the rule engine, and notifications are a CLOUD capability (D156/D129) — so
    /// this seam carries NO mutate method (GR19).
    alerts: Option<Arc<dyn crate::alerts_view::AlertView>>,
    /// The optional declarative-tools registry admin seam (PR-6a — the host
    /// injects an `Arc<SqliteToolRegistry>` wrapper over the durable `tools.db`).
    /// `None` ⇒ `RegisterTool`/`DeregisterTool`/`DiscoverTools` return
    /// `unimplemented`. Off-journal, off-digest, off-identity. DIALING the
    /// external MCP server + Connections + parallel fan-out are PR-6b/Cloud
    /// (D159/GR19) — this seam stores a vetted `server_host`, never dials it.
    tool_admin: Option<Arc<dyn crate::tool_registry_admin::ToolRegistryAdmin>>,
    /// The optional EXTERNAL MCP gateway admin seam (PR-6b-1 — the host injects an
    /// `McpGateway` wrapper that DIALS external MCP servers + registers their
    /// tools into the same `tools.db`). `None` ⇒ `RegisterMcpServer`/
    /// `ListMcpServers`/`DiscoverServerTools`/`TestMcpServer`/`DeregisterMcpServer`
    /// return `unimplemented`. The live untrusted-egress surface (GR8). OAuth/
    /// device-flow + a credential marketplace are CLOUD (D159/GR19).
    mcp_admin: Option<Arc<dyn crate::mcp_gateway_admin::McpGatewayAdmin>>,
    /// The optional LOCAL secret-store admin seam (MM-3 — the host injects a
    /// keychain-backed impl). `None` ⇒ `PutSecret`/`ListSecretNames`/`DeleteSecret`
    /// return `unimplemented`. Off-journal (OS keychain + an off-digest NAME index);
    /// the secret VALUE never crosses any return type, the wire, or the journal (D81).
    secret_admin: Option<Arc<dyn crate::secret_admin::SecretAdmin>>,
    /// Whether secret WRITES (`PutSecret`/`DeleteSecret`) are permitted — set true by
    /// the host ONLY when the gateway is bound to a loopback address (the local-first
    /// default). A network-exposed bind ⇒ `false` ⇒ writes are refused (`permission_denied`)
    /// since a remote peer cannot be distinguished from the local operator behind the
    /// gRPC-web/CORS layers; the operator manages secrets over a loopback bind or via the
    /// environment. Reads (`ListSecretNames`) need only an authenticated caller. Default
    /// false (fail-closed).
    secret_writes_loopback_ok: bool,
    /// The optional trigger admin seam (D113 — the host injects a `triggers.db`-backed
    /// impl over the SAME binder + submitter the Invoke path uses). `None` ⇒ the 5
    /// trigger RPCs return `unimplemented`. An inbound event starts a run through the
    /// existing propose-proxy (no journal-writer dep added; frozen trio untouched).
    trigger_admin: Option<Arc<dyn crate::trigger_admin::TriggerAdmin>>,
    /// D114/M11: the optional autonomy-safety admin seam (the host wires a coordinator-
    /// backed impl). `None` ⇒ the four approval/cost RPCs return `unimplemented`.
    approval_admin: Option<Arc<dyn crate::approval_admin::ApprovalAdmin>>,
    /// The optional context-bundle store seam (PR-7 — the host injects a
    /// `bundles.db`-backed sidecar). `None` ⇒ the 4 context-bundle RPCs return
    /// `unimplemented` and `context_bundles` cannot be resolved at bind. Caller-
    /// scoped, off-journal, off-digest, rebuildable-to-empty.
    bundles: Option<Arc<dyn crate::bundles_view::BundleStore>>,
    /// The optional branch store seam (D155 Phase-A — the host injects a
    /// `branches.db`-backed sidecar + content store + `KX_SERVE_FS_ROOT` mount).
    /// `None` ⇒ the 5 branch RPCs return `unimplemented`. Caller-scoped,
    /// off-journal, off-digest, rebuildable-to-empty. `SnapshotInto` READS host
    /// files into CAS (default-OFF); it never writes the host (Phase-B).
    branches: Option<Arc<dyn crate::branches_view::BranchStore>>,
    /// The optional App-catalog store seam (POC-4 — the host injects an
    /// `apps.db`-backed sidecar). `None` ⇒ the 3 App RPCs return `unimplemented`.
    /// Caller-scoped, off-journal, off-digest, rebuildable-to-empty. The envelope
    /// carries NO authority (the host validates it); `app run` re-resolves warrants.
    apps: Option<Arc<dyn crate::apps_view::AppCatalog>>,
    /// The optional skill-catalog store seam (the host injects a
    /// `skills.db`-backed sidecar). `None` ⇒ the 4 skill RPCs return
    /// `unimplemented`. Caller-scoped, off-journal, off-digest, rebuildable-to-
    /// empty. A skill is a WISH bundle (the host refuses authority deny-keys);
    /// the bind grants only `wish ∩ caller grants ∩ fireable`.
    skills: Option<Arc<dyn crate::skills_view::SkillCatalog>>,
    /// G2: the optional App-RUN seam (the host injects an `apps.db` + connection-store
    /// backed resolver). `None` ⇒ `RunApp` returns `unimplemented` and clients fall
    /// back to the legacy `GetApp` → `SubmitWorkflow` path. Server-minted warrants
    /// (SN-8); connection/secret resolution is caller-scoped, off-journal, off-digest.
    app_runner: Option<Arc<dyn crate::apps_run::AppAuthor>>,
    /// The optional per-App lock store seam (POC-5b — the host injects a `locks.db`
    /// sidecar). `None` ⇒ `LockApp` / `UnlockApp` are `unimplemented` AND the
    /// `AdvanceBranch` chokepoint degrades OPEN (an additive feature never tightens
    /// an existing serve). Caller-scoped, off-journal, off-digest, rebuildable-to-
    /// empty (fails OPEN on loss — a lock is an availability gate, not integrity).
    locks: Option<Arc<dyn crate::locks_view::LockStore>>,
    /// The optional POC-5a App-scaffold orchestrator seam (the host injects a
    /// server-side driver that creates the branch + drives the write-recipe loop +
    /// advances the manifest, holding its own runtime/tracker). `None` ⇒ `ScaffoldApp`
    /// / `GetScaffoldStatus` return `unimplemented` (no served model / branch store).
    /// Off-journal, off-digest.
    scaffolder: Option<Arc<dyn crate::scaffold::AppScaffolder>>,
    /// Model Control v2: the optional model-acquisition orchestrator seam behind
    /// `PullModel` / `GetPullStatus` (download + runtime-register a model). `None` ⇒
    /// both RPCs return `unimplemented`. The host impl owns the deny-by-default
    /// opt-in/allowlist/SHA gate; HOST INFRASTRUCTURE, not a client Mote (SN-8).
    /// Off-journal, off-digest.
    model_puller: Option<Arc<dyn crate::model_pull::ModelPuller>>,
    /// Model Control v2: the optional active-default-model CONTROL seam behind
    /// `SetActiveModel` (+ projected on `ModelSummary.active` /
    /// `GetServerInfo.active_model_id`). `None` ⇒ `SetActiveModel` is `unimplemented`
    /// and `active` is always false. An off-journal advisory hint (SN-8).
    active_model: Option<Arc<dyn crate::active_model::ActiveModelControl>>,
}

/// The default fail-closed `PutContent` payload cap (32 MiB).
pub const DEFAULT_PUT_CAP_BYTES: u64 = 32 * 1024 * 1024;

/// The fail-closed `SubmitFeedback` comment cap (4 KiB). A longer note ⇒
/// `invalid_argument` (checked BEFORE the write — never unbounded sidecar rows).
pub const MAX_FEEDBACK_COMMENT_BYTES: usize = 4 * 1024;

/// The `GetContentBatch` ref-count cap: more refs ⇒ `invalid_argument`
/// (fail-closed, never silent truncation).
pub const MAX_BATCH_REFS: usize = 64;

/// The server-side per-item payload clamp on `GetContentBatch` (512 KiB). The
/// effective clamp is `min(client max_bytes_per_item, this)` — a full batch can
/// never exceed `64 × 512 KiB = 32 MiB` on the wire, so the response always
/// fits the transport budget; `truncated` + `full_size` stay honest and the
/// full blob remains fetchable via single `GetContent`.
pub const BATCH_ITEM_CLAMP_BYTES: u64 = 512 * 1024;

/// The default `SearchRecipes` result cap when the request omits `limit`.
pub const SEARCH_RECIPES_DEFAULT_LIMIT: usize = 20;
/// The hard ceiling on `SearchRecipes` results — a request `limit` is clamped to
/// this so a huge value can never widen the host ranker's own bound.
pub const SEARCH_RECIPES_MAX_LIMIT: usize = 100;

impl GatewayService {
    /// Wire a gateway over a read-only journal seam, a propose-proxy, and a
    /// read-only content seam. No catalog seam (the signature RPCs stay
    /// `unimplemented` until [`GatewayService::with_signature_catalog`] wires one).
    pub fn new(
        reader: Arc<dyn JournalReader>,
        submitter: Arc<dyn RunSubmitter>,
        content: Arc<dyn ContentReader>,
    ) -> Self {
        Self {
            reader,
            submitter,
            content,
            catalog: None,
            binder: None,
            catalog_recipes: None,
            membership: None,
            grants_view: None,
            datasets: None,
            fuzzy: None,
            memory: None,
            capture: None,
            tailer: Arc::new(SnapshotTailer),
            global_tailer: Arc::new(SnapshotGlobalTailer),
            token_tailer: Arc::new(NoTokenTailer),
            telemetry: None,
            critics_supported: false,
            react_supported: false,
            registered_tools: std::collections::BTreeSet::new(),
            registered_tools_view: None,
            toolscout: None,
            author: None,
            content_writer: None,
            uploads: None,
            put_cap_bytes: DEFAULT_PUT_CAP_BYTES,
            models: None,
            model_lifecycle: None,
            server_info: None,
            mote_defs: None,
            feedback: None,
            run_inputs: None,
            alerts: None,
            tool_admin: None,
            mcp_admin: None,
            secret_admin: None,
            secret_writes_loopback_ok: false,
            trigger_admin: None,
            approval_admin: None,
            bundles: None,
            branches: None,
            apps: None,
            skills: None,
            app_runner: None,
            locks: None,
            scaffolder: None,
            model_puller: None,
            active_model: None,
        }
    }

    /// Declare that this serve can EVALUATE native deterministic critics (PR-2c-3
    /// critic-live, H5) — the host has wired a critic-capable executor (the
    /// inference build's `ModelRouterExecutor`). Until set, `SubmitRun` refuses a
    /// critic-bearing workflow fail-closed (a critic with no verdict arm deadlocks
    /// the exit gate).
    #[must_use]
    pub fn with_critics_supported(mut self, supported: bool) -> Self {
        self.critics_supported = supported;
        self
    }

    /// Declare that this serve can DRIVE live ReAct chains (PR-2d-2) — the host
    /// has wired the inference-build executor whose react arm decodes/fences a
    /// turn's output. Until set, `SubmitRun` refuses `react_seed` submissions
    /// fail-closed (a chain whose turns echo-commit settles a meaningless
    /// Answer — the critic-admission B3/H5 mirror).
    #[must_use]
    pub fn with_react_supported(mut self, supported: bool) -> Self {
        self.react_supported = supported;
        self
    }

    /// Declare the `(tool_id, tool_version)` capabilities the host ACTUALLY
    /// registered on the serve broker (PR-2d-2). The Invoke admission refuses a
    /// bound warrant granting anything outside this set — a grant the broker
    /// cannot honour would dead-letter every observation it fires.
    #[must_use]
    pub fn with_registered_tools(
        mut self,
        tools: std::collections::BTreeSet<(String, String)>,
    ) -> Self {
        self.registered_tools = tools;
        self
    }

    /// Wire the LIVE broker-fireable tool set seam (PR-6b-2). When set, the
    /// authoring backstop reads the broker's CURRENT capabilities instead of the
    /// static [`with_registered_tools`](Self::with_registered_tools) snapshot — so
    /// a runtime-DIALED external MCP tool (or any tool registered after startup)
    /// becomes authorable the moment its firing capability registers.
    #[must_use]
    pub fn with_registered_tools_view(mut self, view: Arc<dyn RegisteredToolsView>) -> Self {
        self.registered_tools_view = Some(view);
        self
    }

    /// PR-6b-2: the LIVE broker-fireable `(tool_id, tool_version)` set the
    /// authoring/invoke backstop checks against — the wired [`RegisteredToolsView`]
    /// when present (so a runtime-DIALED external MCP tool is visible the moment it
    /// registers), else the static `registered_tools` snapshot (the PR-2d-2
    /// behaviour). Never authorizes (SN-8); a fail-closed drift backstop.
    fn fireable_grants(&self) -> std::collections::BTreeSet<(String, String)> {
        match &self.registered_tools_view {
            Some(view) => view.registered_grants(),
            None => self.registered_tools.clone(),
        }
    }

    /// Wire the signature-catalog seam (the host's concrete `kx-catalog`-backed
    /// impl). Enables `ListSignatures` / `GetSignature` / `RegisterSignature`.
    #[must_use]
    pub fn with_signature_catalog(mut self, catalog: Arc<dyn SignatureCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Wire the recipe-binding seam (the host's `kx-invoke`-backed binder).
    /// Enables `Invoke` (recipe-by-handle execution).
    #[must_use]
    pub fn with_recipe_binder(mut self, binder: Arc<dyn RecipeBinder>) -> Self {
        self.binder = Some(binder);
        self
    }

    /// Wire the DAG-authoring seam (the host's `kx-workflow`-backed author).
    /// Enables `SubmitWorkflow` (Tier-1 Blueprint authoring + run).
    #[must_use]
    pub fn with_workflow_author(mut self, author: Arc<dyn WorkflowAuthor>) -> Self {
        self.author = Some(author);
        self
    }

    /// Wire the recipe-discovery seam (the host's recipe-library-backed catalog).
    /// Enables `ListRecipes` / `GetRecipeForm` (the UI-2 recipe forms). Read-only
    /// — never a journal write or a digest change.
    #[must_use]
    pub fn with_recipe_catalog(mut self, catalog_recipes: Arc<dyn RecipeCatalog>) -> Self {
        self.catalog_recipes = Some(catalog_recipes);
        self
    }

    /// Wire the membership-view seam (the host's `kx-fleet`-backed view). Enables
    /// `ListTeams` / `ListTeamMembers` (the UI-3 teams viewer). Read-only — never a
    /// journal write or a digest change.
    #[must_use]
    pub fn with_membership_view(mut self, membership: Arc<dyn MembershipView>) -> Self {
        self.membership = Some(membership);
        self
    }

    /// Wire the grant-view seam (the host's `kx-catalog`-backed view). Enables
    /// `ListAssetGrants` (the UI-3 sharing/grants inspector). Read-only — never a
    /// journal write or a digest change.
    #[must_use]
    pub fn with_grant_view(mut self, grants_view: Arc<dyn GrantView>) -> Self {
        self.grants_view = Some(grants_view);
        self
    }

    /// Wire the dataset-view seam (the host's `kx-dataset-hnsw`-backed view, behind
    /// the `hnsw` feature). Enables `ListDatasets` / `IngestDocuments` /
    /// `QueryDataset` (the T3.7 Datasets data-plane). Off the journal/digest —
    /// datasets are a separate durable store (D40 rebuildable cache).
    #[must_use]
    pub fn with_dataset_view(mut self, datasets: Arc<dyn DatasetView>) -> Self {
        self.datasets = Some(datasets);
        self
    }

    /// Wire the fuzzy-discovery seam (Slice-B). The host injects the SAME
    /// `kx-dataset-hnsw`-backed view as [`with_dataset_view`](Self::with_dataset_view).
    /// Enables `FuzzyDiscovery` (advisory fuzzy-in/exact-out). Off the journal/digest.
    #[must_use]
    pub fn with_fuzzy_discovery(mut self, fuzzy: Arc<dyn FuzzyDiscoveryView>) -> Self {
        self.fuzzy = Some(fuzzy);
        self
    }

    /// Wire the durable-MEMORY seam (RC5a — the host's `kx-memory`-backed view,
    /// behind the `hnsw` feature). Enables `StoreMemory` / `ListMemories` /
    /// `RecallMemory` / `ForgetMemory`. Off the journal/digest — memory.db is a
    /// rebuildable sidecar; each memory is namespaced to the caller principal.
    #[must_use]
    pub fn with_memory_view(mut self, memory: Arc<dyn MemoryView>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Wire the capture-view seam (the Morphic Data Engine — the host's
    /// `capture.db`-backed action projection, folded from the journal). Enables
    /// `ListCaptureRecords`. Read-only, off-truth-path: capture is a rebuildable
    /// cache, never journaled, never identity (D40).
    #[must_use]
    pub fn with_capture_view(mut self, capture: Arc<dyn CaptureView>) -> Self {
        self.capture = Some(capture);
        self
    }

    /// Wire a live `StreamEvents` tailer (R5 — `kx-gateway`'s `LiveTailer`),
    /// replacing the default snapshot-to-head [`SnapshotTailer`]. Read-side only;
    /// it never changes the journal or the digest.
    #[must_use]
    pub fn with_event_tailer(mut self, tailer: Arc<dyn EventTailer>) -> Self {
        self.tailer = tailer;
        self
    }

    /// Wire a live `StreamAllEvents` GLOBAL tailer (Batch C — `kx-gateway`'s
    /// `GlobalLiveTailer`), replacing the default snapshot-to-head
    /// [`SnapshotGlobalTailer`]. Read-side only; it never changes the journal or
    /// the digest. Operator-global: cloud must party-scope or deny (the proto
    /// flag).
    #[must_use]
    pub fn with_global_event_tailer(mut self, global_tailer: Arc<dyn GlobalEventTailer>) -> Self {
        self.global_tailer = global_tailer;
        self
    }

    /// Wire a live `StreamModelTokens` tailer (PR-4.2 / T-STREAM1 —
    /// `kx-gateway`'s broker-backed `LiveTokenTailer`), replacing the default
    /// empty [`NoTokenTailer`]. ADVISORY + out-of-band: it taps the in-flight
    /// generation loop and never changes the journal, the digest, or identity.
    #[must_use]
    pub fn with_token_tailer(mut self, token_tailer: Arc<dyn TokenTailer>) -> Self {
        self.token_tailer = token_tailer;
        self
    }

    /// Wire the telemetry-view seam (Batch C — the host's `telemetry.db`-backed
    /// execution-exhaust view). Enables `ListMoteTelemetry`. Read-only,
    /// off-truth-path, rebuildable-to-empty: telemetry is host-measured exhaust,
    /// never journaled, never identity, never a digest input.
    #[must_use]
    pub fn with_telemetry_view(
        mut self,
        telemetry: Arc<dyn crate::telemetry_view::TelemetryView>,
    ) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Wire the advisory toolscout seam (W1.A5 — the host's registry-backed
    /// manifest index). Enables `ListToolManifests` / `ScoreTaskBundle`.
    /// Read-only, display-only — never a journal write, a digest change, or an
    /// authorization (the SN-8 advisory boundary).
    #[must_use]
    pub fn with_toolscout_view(
        mut self,
        toolscout: Arc<dyn crate::toolscout_view::ToolScoutView>,
    ) -> Self {
        self.toolscout = Some(toolscout);
        self
    }

    /// Wire the content-write seam (Batch A — the host exposes its content
    /// store for client uploads). Enables `PutContent` together with
    /// [`GatewayService::with_uploads_ledger`] — BOTH are required (an upload
    /// that cannot be recorded would be unreachable through the uploads scope).
    /// A content-store write only: the journal seam still has no write surface.
    #[must_use]
    pub fn with_content_writer(mut self, writer: Arc<dyn crate::writer::ContentWriter>) -> Self {
        self.content_writer = Some(writer);
        self
    }

    /// Wire the uploads ledger (Batch A — the host's `uploads.db` sidecar:
    /// advisory audit rows + the uploads-scope authorized set). Rebuildable-
    /// to-empty audit state, off-journal, off-digest.
    #[must_use]
    pub fn with_uploads_ledger(mut self, uploads: Arc<dyn crate::uploads::UploadsLedger>) -> Self {
        self.uploads = Some(uploads);
        self
    }

    /// Set the fail-closed `PutContent` payload cap (the host wires
    /// `kx serve --content-max-bytes`; defaults to [`DEFAULT_PUT_CAP_BYTES`]).
    #[must_use]
    pub fn with_put_content_cap(mut self, cap_bytes: u64) -> Self {
        self.put_cap_bytes = cap_bytes;
        self
    }

    /// Wire the model-discovery seam (Batch A — the host's provisioned model
    /// catalog). Enables `ListModels`. Display/discovery only: model selection
    /// stays a recipe ENUM free-param validated server-side (SN-8).
    #[must_use]
    pub fn with_model_catalog_view(
        mut self,
        models: Arc<dyn crate::models_view::ModelCatalogView>,
    ) -> Self {
        self.models = Some(models);
        self
    }

    /// POC-3: wire the model-lifecycle CONTROL seam (warm/evict the registered
    /// set's RAM residency). Enables `LoadModel`/`OffloadModel`; without it they
    /// return `unimplemented`. Off-journal, off-digest — ephemeral RAM state.
    #[must_use]
    pub fn with_model_lifecycle(
        mut self,
        control: Arc<dyn crate::model_lifecycle::ModelLifecycleControl>,
    ) -> Self {
        self.model_lifecycle = Some(control);
        self
    }

    /// Model Control v2: wire the model-acquisition orchestrator seam (download +
    /// runtime-register a model). Enables `PullModel`/`GetPullStatus`; without it they
    /// return `unimplemented`. The host impl owns the deny-by-default opt-in/allowlist/
    /// SHA gate — HOST INFRASTRUCTURE, not a client Mote (SN-8). Off-journal.
    #[must_use]
    pub fn with_model_puller(mut self, puller: Arc<dyn crate::model_pull::ModelPuller>) -> Self {
        self.model_puller = Some(puller);
        self
    }

    /// Model Control v2: wire the active-default-model CONTROL seam. Enables
    /// `SetActiveModel` + the `ModelSummary.active` / `GetServerInfo.active_model_id`
    /// projection. An off-journal advisory hint (the server never re-routes chat).
    #[must_use]
    pub fn with_active_model_control(
        mut self,
        control: Arc<dyn crate::active_model::ActiveModelControl>,
    ) -> Self {
        self.active_model = Some(control);
        self
    }

    /// POC-1: wire the NON-SECRET server-configuration facts the host projects via
    /// `GetServerInfo` (the Settings "Workspace" view). Without this seam
    /// `GetServerInfo` returns `unimplemented`. The facts carry no secret by
    /// construction (no token / TLS-key field — POC-2 token-never-leaks negative).
    #[must_use]
    pub fn with_server_info(mut self, facts: crate::server_info::ServerInfoFacts) -> Self {
        self.server_info = Some(facts);
        self
    }

    /// Wire the def-resolution seam (Batch B — the host's content-store-backed
    /// def reader). Enables `GetMoteDetail`. Display only (SN-8).
    #[must_use]
    pub fn with_mote_def_view(
        mut self,
        mote_defs: Arc<dyn crate::mote_def_view::MoteDefView>,
    ) -> Self {
        self.mote_defs = Some(mote_defs);
        self
    }

    /// Wire the user-feedback store (PR-4.1 — the host's `feedback.db` sidecar:
    /// 👍/👎 product signal). Enables `SubmitFeedback` + `ListFeedback`.
    /// Client-origin, rebuildable-to-empty, off-journal, off-digest, off-identity.
    #[must_use]
    pub fn with_feedback_store(
        mut self,
        feedback: Arc<dyn crate::feedback_view::FeedbackStore>,
    ) -> Self {
        self.feedback = Some(feedback);
        self
    }

    /// Wire the run-inputs store (PR-D — the host's `run_inputs.db` sidecar that
    /// captures `Invoke` args). Enables `GetRunInputs` + the best-effort capture
    /// on the `Invoke` path. Rebuildable-to-empty, off-journal, off-digest,
    /// off-identity (the args never become committed facts).
    #[must_use]
    pub fn with_run_inputs_store(
        mut self,
        run_inputs: Arc<dyn crate::run_inputs_view::RunInputsStore>,
    ) -> Self {
        self.run_inputs = Some(run_inputs);
        self
    }

    /// Wire the alerts-inbox view (W1a-2 — the host's `alerts.db` read-cache
    /// folded from the journal's terminal `Failed` facts). Enables `ListAlerts`.
    /// Read-only, off-truth-path, rebuildable. The triage LIFECYCLE
    /// (acknowledge/resolve), the rule engine, and notifications are a CLOUD
    /// capability (D156/D129) — not exposed by this OSS seam (GR19).
    #[must_use]
    pub fn with_alerts_view(mut self, alerts: Arc<dyn crate::alerts_view::AlertView>) -> Self {
        self.alerts = Some(alerts);
        self
    }

    /// Wire the declarative-tools registry admin seam (PR-6a — the host's durable
    /// `tools.db` + admission-time SSRF vetting). Enables `RegisterTool` /
    /// `DeregisterTool` / `DiscoverTools`. Off-journal, off-digest. DIALING the
    /// external MCP server + Connections + parallel fan-out are PR-6b/Cloud
    /// (D159/GR19) — not exposed by this OSS seam.
    #[must_use]
    pub fn with_tool_admin(
        mut self,
        tool_admin: Arc<dyn crate::tool_registry_admin::ToolRegistryAdmin>,
    ) -> Self {
        self.tool_admin = Some(tool_admin);
        self
    }

    /// Inject the EXTERNAL MCP gateway admin seam (PR-6b-1). `None` (the default)
    /// ⇒ the 5 MCP-server RPCs return `unimplemented`.
    #[must_use]
    pub fn with_mcp_admin(
        mut self,
        mcp_admin: Arc<dyn crate::mcp_gateway_admin::McpGatewayAdmin>,
    ) -> Self {
        self.mcp_admin = Some(mcp_admin);
        self
    }

    /// Inject the LOCAL secret-store admin seam (MM-3). `None` (the default) ⇒ the 3
    /// secret RPCs return `unimplemented`. `writes_loopback_ok` gates secret WRITES
    /// (`PutSecret`/`DeleteSecret`): pass `true` ONLY when the gateway is loopback-bound
    /// (so no remote peer can plant/remove credential material); reads need only an
    /// authenticated caller regardless.
    #[must_use]
    pub fn with_secret_admin(
        mut self,
        secret_admin: Arc<dyn crate::secret_admin::SecretAdmin>,
        writes_loopback_ok: bool,
    ) -> Self {
        self.secret_admin = Some(secret_admin);
        self.secret_writes_loopback_ok = writes_loopback_ok;
        self
    }

    /// Inject the trigger admin seam (D113). `None` (the default) ⇒ the 5 trigger
    /// RPCs return `unimplemented`.
    #[must_use]
    pub fn with_trigger_admin(
        mut self,
        trigger_admin: Arc<dyn crate::trigger_admin::TriggerAdmin>,
    ) -> Self {
        self.trigger_admin = Some(trigger_admin);
        self
    }

    /// Inject the autonomy-safety admin seam (D114/M11). `None` (the default) ⇒ the
    /// four approval/cost RPCs return `unimplemented`.
    #[must_use]
    pub fn with_approval_admin(
        mut self,
        approval_admin: Arc<dyn crate::approval_admin::ApprovalAdmin>,
    ) -> Self {
        self.approval_admin = Some(approval_admin);
        self
    }

    /// Inject the context-bundle store seam (PR-7). `None` (the default) ⇒ the 4
    /// context-bundle RPCs return `unimplemented` and `context_bundles` resolves
    /// empty (a clear bind error).
    #[must_use]
    pub fn with_bundles_store(
        mut self,
        bundles: Arc<dyn crate::bundles_view::BundleStore>,
    ) -> Self {
        self.bundles = Some(bundles);
        self
    }

    /// Wire the D155 Phase-A branch store (the host's `branches.db` sidecar +
    /// content store + `KX_SERVE_FS_ROOT` mount). Without it the five branch RPCs
    /// return `unimplemented`.
    #[must_use]
    pub fn with_branches_store(
        mut self,
        branches: Arc<dyn crate::branches_view::BranchStore>,
    ) -> Self {
        self.branches = Some(branches);
        self
    }

    /// Wire the POC-4 App-catalog store (the host's `apps.db` sidecar). Without it
    /// the three App RPCs (`SaveApp`/`ListApps`/`GetApp`) return `unimplemented`.
    #[must_use]
    pub fn with_apps_catalog(mut self, apps: Arc<dyn crate::apps_view::AppCatalog>) -> Self {
        self.apps = Some(apps);
        self
    }

    /// Wire the skill-catalog store (the host's `skills.db` sidecar).
    /// Without it the four skill RPCs (`ListSkills`/`GetSkillForm`/`AddSkill`/
    /// `RemoveSkill`) return `unimplemented`.
    #[must_use]
    pub fn with_skill_catalog(mut self, skills: Arc<dyn crate::skills_view::SkillCatalog>) -> Self {
        self.skills = Some(skills);
        self
    }

    /// G2: wire the App-RUN seam (the `RunApp` path). Without it `RunApp` returns
    /// `unimplemented` and clients fall back to the legacy `GetApp` → `SubmitWorkflow`
    /// path. The host impl (`kx-gateway`) reads the validated envelope, lowers its
    /// blueprint, resolves `references.connections` against the caller's own registry,
    /// and narrows the run warrant's secret scope to the App's declared secrets;
    /// warrants are server-minted (SN-8).
    #[must_use]
    pub fn with_app_runner(mut self, runner: Arc<dyn crate::apps_run::AppAuthor>) -> Self {
        self.app_runner = Some(runner);
        self
    }

    /// Wire the POC-5b per-App lock store (the host's `locks.db` sidecar). Without it
    /// `LockApp`/`UnlockApp` return `unimplemented` AND the `AdvanceBranch` chokepoint
    /// degrades OPEN (an additive feature never tightens an existing serve).
    #[must_use]
    pub fn with_lock_store(mut self, locks: Arc<dyn crate::locks_view::LockStore>) -> Self {
        self.locks = Some(locks);
        self
    }

    /// Wire the POC-5a App-scaffold orchestrator (the host's server-side driver,
    /// seeded only when a served model is present). Without it `ScaffoldApp` /
    /// `GetScaffoldStatus` return `unimplemented` fail-closed.
    #[must_use]
    pub fn with_app_scaffolder(
        mut self,
        scaffolder: Arc<dyn crate::scaffold::AppScaffolder>,
    ) -> Self {
        self.scaffolder = Some(scaffolder);
        self
    }
}

/// The structured-refusal metadata key (PR-2). The value is
/// `SubmissionRefusal::code()` — static ASCII by construction.
pub const REFUSAL_CODE_METADATA_KEY: &str = "kx-refusal-code";

/// Attach the structured refusal code to a refusal `Status` (PR-2). A code
/// that fails ASCII metadata parsing (theoretical — codes are static ASCII)
/// degrades to the bare status rather than failing the error path.
fn with_refusal_code(mut status: Status, code: &str) -> Status {
    if let Ok(value) = code.parse() {
        status
            .metadata_mut()
            .insert(REFUSAL_CODE_METADATA_KEY, value);
    }
    status
}

fn submit_status(err: SubmitterError) -> Status {
    match err {
        SubmitterError::Rejected(detail) => Status::failed_precondition(detail),
        SubmitterError::Refused { code, detail } => {
            with_refusal_code(Status::failed_precondition(detail), &code)
        }
        SubmitterError::Transport(detail) => Status::unavailable(detail),
    }
}

/// Derive the per-run agentic chain key returned on [`proto::RunHandle`]. A
/// tool-granted MODEL Mote (its server-built warrant carries non-empty `tool_grants`)
/// launches an agentic ReAct chain whose turns the coordinator salts by that step's
/// `MoteId` (`step_salt = *launch_id.as_bytes()`, `kx-coordinator` `settle_agentic_launches`).
/// Reporting it lets a client on serve's SHARED journal (a single `instance_id` across
/// every run) scope `ListReactTurns` / the answer poll to THIS submission's chain — the
/// `SubmitWorkflow`/`RunApp` analogue of `InvokeResponse.react_chain_salt`.
///
/// Returns that step's id iff EXACTLY ONE agentic step is present; otherwise EMPTY (no
/// agentic step, or more than one — ambiguous, so the client falls back to
/// instance_id-only scoping, exactly as it does against an old server). Server-derived
/// from the bound motes, never client-supplied (SN-8).
fn agentic_chain_salt(motes: &[(kx_mote::Mote, kx_warrant::WarrantSpec)]) -> Vec<u8> {
    let mut agentic = motes.iter().filter(|(_, w)| !w.tool_grants.is_empty());
    match (agentic.next(), agentic.next()) {
        (Some((mote, _)), None) => mote.id.as_bytes().to_vec(),
        _ => Vec::new(),
    }
}

/// Map the wire palette (`WorkflowStep` / `WorkflowEdge` + execution mode) into
/// gateway-core's authoring vocabulary. The SINGLE shared parse that both
/// `SubmitWorkflow` and the G2 `RunApp` path (via the host, after lowering a stored
/// App's blueprint through `kx-blueprint::to_request`) funnel through, so a blueprint
/// lowers to identical `AuthorStep`s regardless of caller. UNSPECIFIED step/edge kinds
/// are refused (`invalid_argument`); UNSPECIFIED + DYNAMIC execution modes collapse to
/// `Frozen` / `Dynamic` (the host refuses `Dynamic` fail-closed downstream).
///
/// # Errors
/// [`Status::invalid_argument`] on an UNSPECIFIED step kind, an UNSPECIFIED edge kind,
/// or a `body_signature_id` that is present but not 32 bytes.
// `Status` is large; boxing it would force every caller (the RPC handlers) to unbox —
// the same crate-wide rationale as the streaming seams above.
#[allow(clippy::result_large_err)]
pub fn author_steps_from_proto(
    steps: Vec<proto::WorkflowStep>,
    edges: Vec<proto::WorkflowEdge>,
    execution_mode: i32,
) -> Result<(Vec<AuthorStep>, Vec<AuthorEdge>, AuthorExecutionMode), Status> {
    let mut out_steps: Vec<AuthorStep> = Vec::with_capacity(steps.len());
    for s in steps {
        let kind = match proto::WorkflowStepKind::try_from(s.kind) {
            Ok(proto::WorkflowStepKind::Pure) => AuthorStepKind::Pure,
            Ok(proto::WorkflowStepKind::Model) => AuthorStepKind::Model,
            Ok(proto::WorkflowStepKind::Exec) => AuthorStepKind::Exec,
            Ok(proto::WorkflowStepKind::Tool) => AuthorStepKind::Tool,
            _ => {
                return Err(Status::invalid_argument(
                    "WorkflowStep.kind must be PURE, MODEL, EXEC, or TOOL",
                ));
            }
        };
        let body_signature_id = if s.body_signature_id.is_empty() {
            None
        } else {
            Some(hash_32(
                &s.body_signature_id,
                "WorkflowStep.body_signature_id must be 32 bytes",
            )?)
        };
        out_steps.push(AuthorStep {
            kind,
            model_id: s.model_id,
            prompt: s.prompt,
            body_signature_id,
            tool_contract: s.tool_contract.into_iter().collect(),
            params: s.params.into_iter().collect(),
        });
    }
    let mut out_edges: Vec<AuthorEdge> = Vec::with_capacity(edges.len());
    for e in edges {
        let data = match proto::EdgeKind::try_from(e.edge_kind) {
            Ok(proto::EdgeKind::Data) => true,
            Ok(proto::EdgeKind::Control) => false,
            _ => {
                return Err(Status::invalid_argument(
                    "WorkflowEdge.edge_kind must be DATA or CONTROL",
                ));
            }
        };
        out_edges.push(AuthorEdge {
            parent: e.parent,
            child: e.child,
            data,
            non_cascade: e.non_cascade,
        });
    }
    let mode = match proto::WorkflowExecutionMode::try_from(execution_mode) {
        Ok(proto::WorkflowExecutionMode::Dynamic) => AuthorExecutionMode::Dynamic,
        _ => AuthorExecutionMode::Frozen,
    };
    Ok((out_steps, out_edges, mode))
}

/// Map an optional wire `bytes` field to a fixed `[u8; N]` — all-zero when
/// absent, `invalid_argument` when present but the wrong length (the
/// fail-closed posture of the telemetry/uploads id validators). The `Status`
/// Err is the gRPC return type the calling handler already uses (the trait-impl
/// handlers are exempt from `result_large_err`; this free helper shares the same
/// return type by design — boxing it would only churn the call site).
#[allow(clippy::result_large_err)]
fn opt_fixed<const N: usize>(raw: Option<Vec<u8>>, what: &str) -> Result<[u8; N], Status> {
    match raw {
        None => Ok([0u8; N]),
        Some(v) => <[u8; N]>::try_from(v.as_slice())
            .map_err(|_| Status::invalid_argument(format!("{what} must be {N} bytes"))),
    }
}

/// The fail-closed `RegisterTool` description cap (4 KiB). A longer description
/// ⇒ `invalid_argument` (checked BEFORE the durable write).
const MAX_TOOL_DESCRIPTION_BYTES: usize = 4 * 1024;

/// The fail-closed `RegisterTool` param-count cap. A larger schema ⇒
/// `invalid_argument`.
const MAX_TOOL_PARAMS: usize = 64;

/// Map a `ToolInputSchema` wire message into the gateway-core seam vocabulary.
fn tool_schema_from_proto(s: proto::ToolInputSchema) -> crate::ToolSchemaWire {
    crate::ToolSchemaWire {
        params: s
            .params
            .into_iter()
            .map(|p| crate::ToolParamWire {
                name: p.name,
                ty: p.ty,
                max_len: p.max_len,
                required: p.required,
                allowed: p.allowed,
            })
            .collect(),
        deny_unknown: s.deny_unknown,
    }
}

/// Map a tool-registry admin refusal onto the fail-closed gRPC status. A rejected
/// `server_host` is `permission_denied` (not a permitted egress target); a bad
/// field is `invalid_argument`; a durable-store failure is `internal`.
#[allow(clippy::result_large_err)]
fn tool_admin_status(err: crate::ToolAdminError) -> Status {
    match err {
        crate::ToolAdminError::HostRejected(detail) => Status::permission_denied(detail),
        crate::ToolAdminError::InvalidArgument(detail) => Status::invalid_argument(detail),
        crate::ToolAdminError::Storage(detail) => Status::internal(detail),
    }
}

/// Map an MCP gateway admin refusal onto the fail-closed gRPC status. A rejected
/// host is `permission_denied`; an invalid spec is `invalid_argument`; an
/// unreachable server is `failed_precondition`; over-budget is `resource_exhausted`;
/// an unknown server is `not_found`; a storage failure is `internal`.
#[allow(clippy::result_large_err)]
fn mcp_admin_status(err: crate::McpAdminError) -> Status {
    match err {
        crate::McpAdminError::HostRejected(detail) => Status::permission_denied(detail),
        crate::McpAdminError::InvalidArgument(detail) => Status::invalid_argument(detail),
        crate::McpAdminError::Dial(detail) => Status::failed_precondition(detail),
        crate::McpAdminError::RateLimited(detail) => Status::resource_exhausted(detail),
        crate::McpAdminError::NotFound(detail) => Status::not_found(detail),
        crate::McpAdminError::Storage(detail) => Status::internal(detail),
    }
}

/// Map a [`crate::SecretAdminError`] (MM-3) to a tonic `Status`.
fn secret_admin_status(err: crate::SecretAdminError) -> Status {
    match err {
        crate::SecretAdminError::InvalidArgument(detail) => Status::invalid_argument(detail),
        crate::SecretAdminError::Unavailable(detail) => Status::failed_precondition(detail),
        crate::SecretAdminError::Storage(detail) => Status::internal(detail),
    }
}

/// Map a [`crate::TriggerAdminError`] (D113) to a tonic `Status`.
fn trigger_admin_status(err: crate::TriggerAdminError) -> Status {
    match err {
        crate::TriggerAdminError::InvalidArgument(detail) => Status::invalid_argument(detail),
        crate::TriggerAdminError::NotFound(detail) => Status::not_found(detail),
        crate::TriggerAdminError::NotAuthorized => Status::permission_denied("not authorized"),
        crate::TriggerAdminError::Unsupported(detail) => Status::failed_precondition(detail),
        crate::TriggerAdminError::Storage(detail) => Status::internal(detail),
    }
}

/// D114/M11: map an [`crate::ApprovalAdminError`] to a gRPC status.
fn approval_admin_status(err: crate::ApprovalAdminError) -> Status {
    match err {
        crate::ApprovalAdminError::InvalidArgument(detail) => Status::invalid_argument(detail),
        crate::ApprovalAdminError::Internal(detail) => Status::internal(detail),
    }
}

/// D114: validate a 16-byte approval `request_id` argument (SN-8 — the server-derived
/// handshake handle; a client never computes it, only echoes the bytes it was shown).
#[allow(clippy::result_large_err)] // a `Status` Err mirrors the handler convention.
fn approval_request_id_arg(raw: &[u8]) -> Result<[u8; 16], Status> {
    raw.try_into()
        .map_err(|_| Status::invalid_argument("request_id must be 16 bytes"))
}

/// Proto `TriggerKind` (i32) → the seam's string vocabulary.
fn trigger_kind_str(kind: i32) -> &'static str {
    match proto::TriggerKind::try_from(kind) {
        Ok(proto::TriggerKind::Webhook) => "webhook",
        Ok(proto::TriggerKind::Cron) => "cron",
        Ok(proto::TriggerKind::Grpc) => "grpc",
        _ => "",
    }
}

/// The seam's string vocabulary → proto `TriggerKind` (i32).
fn trigger_kind_proto(kind: &str) -> i32 {
    match kind {
        "webhook" => proto::TriggerKind::Webhook as i32,
        "cron" => proto::TriggerKind::Cron as i32,
        "grpc" => proto::TriggerKind::Grpc as i32,
        _ => proto::TriggerKind::Unspecified as i32,
    }
}

/// Proto `TriggerAuth` (i32) → the seam's string vocabulary.
fn trigger_auth_str(auth: i32) -> &'static str {
    match proto::TriggerAuth::try_from(auth) {
        Ok(proto::TriggerAuth::None) => "none",
        Ok(proto::TriggerAuth::HmacSha256) => "hmac_sha256",
        Ok(proto::TriggerAuth::Bearer) => "bearer",
        _ => "",
    }
}

/// The seam's string vocabulary → proto `TriggerAuth` (i32).
fn trigger_auth_proto(auth: &str) -> i32 {
    match auth {
        "none" => proto::TriggerAuth::None as i32,
        "hmac_sha256" => proto::TriggerAuth::HmacSha256 as i32,
        "bearer" => proto::TriggerAuth::Bearer as i32,
        _ => proto::TriggerAuth::Unspecified as i32,
    }
}

/// MM-3 secret-name validation. A secret NAME is referenced as a connection's
/// `credential_ref` AND used as an OS-keychain entry key AND as the chained-env
/// fallback var name, so it must be a portable identifier: non-empty, ≤255 bytes,
/// and `[A-Za-z0-9_.-]` only (env-var-name-ish + dots/dashes). Rejecting anything
/// else keeps the keychain key + the env fallback unambiguous and log-safe.
fn valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// PR-7: lightweight `namespace/collection/name` handle validation for context
/// bundles (gateway-core stays off `kx-catalog`; mirrors `AssetPath`'s segment
/// rule — lowercase `[a-z0-9._-]`, no leading/trailing `.`/`-`, ≤128B, 3 segments).
fn valid_bundle_handle(h: &str) -> bool {
    let mut segs = 0usize;
    for seg in h.split('/') {
        segs += 1;
        if seg.is_empty() || seg.len() > 128 {
            return false;
        }
        if !seg.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        }) {
            return false;
        }
        let bytes = seg.as_bytes();
        if matches!(bytes[0], b'.' | b'-') || matches!(bytes[bytes.len() - 1], b'.' | b'-') {
            return false;
        }
    }
    segs == 3
}

/// Extract the SERVER-RESOLVED caller principal from the auth interceptor
/// extension (SN-8 — never wire-trusted). Shared by the caller-scoped sidecar
/// RPCs (D155 branches).
// `tonic::Status` exists only at the handler; boxing it would churn every caller.
#[allow(clippy::result_large_err)]
fn caller_principal<T>(request: &Request<T>) -> Result<String, Status> {
    request
        .extensions()
        .get::<CallerParty>()
        .map(|p| p.0.clone())
        .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))
}

/// Validate an optional CoW parent handle (D155): empty ⇒ `None`; non-empty must
/// be a valid `namespace/collection/name` AssetPath handle.
#[allow(clippy::result_large_err)] // see `caller_principal`.
fn optional_handle(h: &str) -> Result<Option<&str>, Status> {
    if h.is_empty() {
        return Ok(None);
    }
    if !valid_bundle_handle(h) {
        return Err(Status::invalid_argument(
            "parent_handle must be a 'namespace/collection/name' AssetPath",
        ));
    }
    Ok(Some(h))
}

/// PR-7: map a host `BundleManifest` to the wire view.
fn manifest_to_proto(m: crate::BundleManifest) -> proto::ContextBundle {
    let item_count = u32::try_from(m.items.len()).unwrap_or(u32::MAX);
    proto::ContextBundle {
        bundle_ref: m.bundle_ref.to_vec(),
        handle: m.handle,
        description: m.description,
        items: m
            .items
            .into_iter()
            .map(|it| proto::ContextItem {
                name: it.name,
                content_ref: it.content_ref.to_vec(),
                media_type: it.media_type,
            })
            .collect(),
        item_count,
    }
}

/// POC-4: map a host `AppRecord` (envelope-derived summary) to the wire view.
fn app_record_to_proto(r: crate::AppRecord) -> proto::AppSummary {
    proto::AppSummary {
        handle: r.handle,
        app_ref: r.app_ref.to_vec(),
        name: r.name,
        version: r.version,
        description: r.description,
        tags: r.tags,
        step_count: r.step_count,
        // POC-5b: the lock lives at the App's project branch (same handle as the
        // App — the one-App-one-branch model). The list/get handlers OVERRIDE this
        // from the wired LockStore; `false` here = unlocked / no branch / unwired.
        locked: false,
    }
}

/// Map a host [`crate::SkillRecord`] (manifest-derived) to the wire view.
fn skill_record_to_proto(r: crate::skills_view::SkillRecord) -> proto::SkillSummary {
    proto::SkillSummary {
        skill_ref: r.skill_ref.to_vec(),
        name: r.name,
        version: r.version,
        description: r.description,
        instructions_ref: r.instructions_ref,
        tools: r.tools.into_iter().collect(),
        tags: r.tags,
    }
}

/// The display excerpt stored beside a skill row at `AddSkill` time
/// (UTF-8 lossy, cut at a char boundary within [`crate::SKILL_PREVIEW_CAP_BYTES`]).
fn skill_preview(body: &[u8]) -> (String, bool) {
    let text = String::from_utf8_lossy(body);
    let cap = crate::skills_view::SKILL_PREVIEW_CAP_BYTES;
    if text.len() <= cap {
        return (text.into_owned(), false);
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (text[..cut].to_string(), true)
}

/// Map the host scaffold phase to the wire enum (POC-5a).
fn scaffold_phase_to_proto(
    p: crate::scaffold::ScaffoldPhase,
) -> proto::get_scaffold_status_response::Phase {
    use crate::scaffold::ScaffoldPhase as P;
    use proto::get_scaffold_status_response::Phase as W;
    match p {
        P::Planning => W::Planning,
        P::Writing => W::Writing,
        P::Done => W::Done,
        P::Failed => W::Failed,
    }
}

/// D155: map a host `BranchManifest` to the wire view (`{path -> ref}` entries).
fn branch_to_proto(m: crate::BranchManifest) -> proto::Branch {
    let item_count = u32::try_from(m.items.len()).unwrap_or(u32::MAX);
    proto::Branch {
        branch_ref: m.branch_ref.to_vec(),
        handle: m.handle,
        parent_handle: m.parent_handle,
        description: m.description,
        items: m
            .items
            .into_iter()
            .map(|it| proto::BranchItem {
                path: it.path,
                content_ref: it.content_ref.to_vec(),
            })
            .collect(),
        item_count,
    }
}

#[tonic::async_trait]
impl KxGateway for GatewayService {
    async fn submit_run(
        &self,
        request: Request<proto::SubmitRunRequest>,
    ) -> Result<Response<proto::RunHandle>, Status> {
        let req = request.into_inner();
        let recipe_fp = hash_32(
            &req.recipe_fingerprint,
            "recipe_fingerprint must be 32 bytes",
        )?;

        // Convert ALL Motes up front (PR-2c-3 critic-live): the cross-Mote critic
        // admission below needs every Mote of the run together, and converting before
        // `register_run` means a malformed / refused submission never leaves an orphan
        // registered run behind.
        let mut collected: Vec<(kx_mote::Mote, kx_warrant::WarrantSpec, bool, bool)> =
            Vec::with_capacity(req.motes.len());
        for spec in req.motes {
            let mote_proto = spec
                .mote
                .ok_or_else(|| Status::invalid_argument("SubmitMoteSpec.mote is required"))?;
            // IDENTITY INVARIANT: TryFrom re-derives the MoteId Rust-side; the
            // wire mote_id is advisory only (D53).
            let mote: kx_mote::Mote = mote_proto
                .try_into()
                .map_err(|e: kx_proto::ConvertError| Status::invalid_argument(e.to_string()))?;
            let warrant_proto = spec
                .warrant
                .ok_or_else(|| Status::invalid_argument("SubmitMoteSpec.warrant is required"))?;
            let warrant: kx_warrant::WarrantSpec = warrant_proto
                .try_into()
                .map_err(|e: kx_proto::ConvertError| Status::invalid_argument(e.to_string()))?;
            collected.push((mote, warrant, spec.accept_at_least_once, spec.react_seed));
        }

        // PR-2d-2 — the SubmitRun TOOL-AUTHORITY gate (red-team BLOCKER #5 + the
        // standing Morphic finding): SubmitRun accepts the client warrant VERBATIM
        // (unlike Invoke, whose warrants are server-derived via bind → intersect),
        // so a client-supplied `tool_grants` would mint tool authority the server
        // never issued. Refused fail-closed BEFORE `register_run` (no orphan run).
        // Tool authority enters serve ONLY via the server-constructed react
        // warrant on the Invoke path.
        if collected
            .iter()
            .any(|(_, w, _, _)| !w.tool_grants.is_empty())
        {
            return Err(Status::failed_precondition(
                "SubmitRun refuses client warrants with tool_grants: tool authority \
                 is server-issued only (use Invoke with a tool-granting recipe)",
            ));
        }

        // PR-2d-2 — react ADMISSION (the critics_supported twin, B3/H5): a react
        // seed on a serve without the inference executor's react arm would
        // echo-commit fake turns and settle a meaningless Answer. Refuse loudly.
        if collected.iter().any(|(_, _, _, react)| *react) && !self.react_supported {
            return Err(Status::failed_precondition(
                "this serve cannot drive a live ReAct chain (no inference executor \
                 wired); a react_seed submission is refused",
            ));
        }

        // PR-2c-3 critic-live — cross-Mote critic ADMISSION (only when the run carries a
        // critic, so a critic-free workflow is byte-for-byte unaffected).
        if collected
            .iter()
            .any(|(m, _, _, _)| m.def.critic_check.is_some())
        {
            // H5: a native critic's verdict is computed ONLY by the inference-build
            // executor. On a serve that cannot, a critic would commit echo bytes and the
            // P4.2-3 exit gate would withhold its producer's consumers forever — so we
            // refuse fail-closed rather than admit a guaranteed deadlock.
            if !self.critics_supported {
                return Err(Status::failed_precondition(
                    "this serve cannot evaluate native deterministic critics (no inference \
                     executor wired); a critic-bearing workflow is refused",
                ));
            }
            // Enforce the CROSS-Mote critic refusals (R-2/R-4/R-5/R-6) the per-Mote
            // submit path cannot — `critic_for` must reference an existing WORLD-MUTATING
            // producer, no producer may carry two critics, a critic may not be itself
            // WORLD-MUTATING, etc. `master_warrant`/`run_id` are not consulted by these
            // checks, so placeholder values are sound.
            let motes: std::collections::BTreeMap<kx_mote::MoteId, kx_mote::Mote> = collected
                .iter()
                .map(|(m, _, _, _)| (m.id, m.clone()))
                .collect();
            let accept_at_least_once = collected.iter().map(|(m, _, a, _)| (m.id, *a)).collect();
            let submission = kx_refusal::WorkflowSubmission {
                run_id: [0u8; 32],
                master_warrant: kx_warrant::WarrantSpec::default(),
                motes,
                accept_at_least_once,
            };
            kx_refusal::validate_submission(&submission).map_err(|e| {
                with_refusal_code(
                    Status::failed_precondition(format!("critic admission refused: {e}")),
                    e.code(),
                )
            })?;
        }

        // Register: returns only after the journaled instance_id (never acks ahead of
        // the journal). Then submit each Mote in order.
        let instance_id = self
            .submitter
            .register_run(recipe_fp)
            .await
            .map_err(submit_status)?;
        for (mote, warrant, accept, react_seed) in collected {
            self.submitter
                .submit_mote(mote, warrant, accept, react_seed)
                .await
                .map_err(submit_status)?;
        }

        Ok(Response::new(proto::RunHandle {
            instance_id: instance_id.to_vec(),
            recipe_fingerprint: recipe_fp.to_vec(),
            // SubmitRun refuses client `tool_grants` + `react_seed` (above), so it can
            // carry no agentic step ⇒ never a chain key.
            react_chain_salt: Vec::new(),
        }))
    }

    async fn invoke(
        &self,
        request: Request<proto::InvokeRequest>,
    ) -> Result<Response<proto::InvokeResponse>, Status> {
        let binder = self.binder.as_ref().ok_or_else(|| {
            Status::unimplemented("Invoke: no recipe binder wired (host provisioned no recipes)")
        })?;
        // SERVER-DERIVED identity (SN-8): the party the auth interceptor resolved
        // and stashed. Absent ⇒ no caller was resolved ⇒ deny. The wire request
        // carries no party field, so a caller cannot assert who it is.
        let party = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();

        let bound = binder
            .bind(
                &party,
                &req.handle,
                &req.args,
                &req.context_bundles,
                &req.context_refs,
            )
            .await
            .map_err(|e| match e {
                // Uniform "not authorized" — no existence oracle on the execution
                // surface (unauthorized / unknown handle are indistinguishable).
                BinderError::NotAuthorized => Status::permission_denied("not authorized"),
                BinderError::InvalidArgs(detail) => Status::invalid_argument(detail),
                BinderError::Internal(detail) => Status::internal(detail),
            })?;

        // PR-2d-2 — Invoke tool-grant admission: every bound warrant's grants must
        // name capabilities the host ACTUALLY registered on the serve broker (a
        // grant the broker cannot honour dead-letters every observation it fires).
        // Server-derived warrants make this a provisioning invariant; the check is
        // the fail-closed backstop against drift. PR-6b-2: read the LIVE broker set
        // so a runtime-dialed tool a recipe grants is honoured.
        let fireable = self.fireable_grants();
        for (_, warrant) in &bound.motes {
            if let Some(grant) = warrant
                .tool_grants
                .iter()
                .find(|g| !fireable.contains(&(g.tool_id.0.clone(), g.tool_version.0.clone())))
            {
                return Err(Status::failed_precondition(format!(
                    "recipe grants tool {}@{} but this serve registered no such \
                     capability",
                    grant.tool_id.0, grant.tool_version.0
                )));
            }
        }
        // PR-2d-2 — the react recipe needs the inference executor's react arm
        // (the SubmitRun react admission, mirrored; unreachable when provisioning
        // seeds the recipe only on a react-capable serve — the fail-closed backstop).
        if bound.react_seed && !self.react_supported {
            return Err(Status::failed_precondition(
                "this serve cannot drive a live ReAct chain (no inference executor \
                 wired); the react recipe is refused",
            ));
        }

        // The SAME propose-proxy as SubmitRun: register first (returns only after
        // the journaled instance_id), then submit each bound Mote. No new write
        // path; the coordinator stays the sole journal writer.
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(submit_status)?;

        // PR-D — best-effort capture of the Invoke args for "Re-run with changes"
        // (an off-journal, off-digest sidecar keyed by `instance_id`). A capture
        // failure NEVER fails the Invoke: the args are pre-fill convenience, not
        // part of run admission (the run is already registered + about to dispatch).
        // `handle` is captured here because a durable run otherwise carries only
        // the fingerprint, not the handle `GetRecipeForm` needs.
        if let Some(store) = self.run_inputs.as_ref() {
            let captured_unix_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            if let Err(e) = store.record(crate::run_inputs_view::RunInputsRecord {
                instance_id,
                recipe_fingerprint: bound.recipe_fingerprint,
                handle: req.handle.clone(),
                args: req.args.clone(),
                principal: party.clone(),
                captured_unix_ms,
            }) {
                tracing::warn!(error = %e, "run-inputs capture failed (best-effort; Invoke unaffected)");
            }
        }

        let react_seed = bound.react_seed;
        // PR-R1: the per-invocation ReAct chain key = the bound react seed Mote's id.
        // The coordinator salts the run-level chain by this SAME id at the seed-swap
        // (`chain_salt = seed.id`), so the value the client gets back scopes
        // ListReactTurns / the answer poll to THIS invocation's chain on serve's
        // shared journal — they agree by construction (SN-8: server-derived). Empty
        // for a non-react Invoke.
        let react_chain_salt: Vec<u8> = if react_seed {
            bound
                .motes
                .first()
                .map(|(m, _)| m.id.as_bytes().to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, react_seed)
                .await
                .map_err(submit_status)?;
        }

        Ok(Response::new(proto::InvokeResponse {
            instance_id: instance_id.to_vec(),
            recipe_fingerprint: bound.recipe_fingerprint.to_vec(),
            // SERVER-DERIVED (from bind → compile, never client-supplied — SN-8).
            terminal_mote_id: bound.terminal_mote_id.as_bytes().to_vec(),
            react_chain_salt,
        }))
    }

    async fn submit_workflow(
        &self,
        request: Request<proto::SubmitWorkflowRequest>,
    ) -> Result<Response<proto::RunHandle>, Status> {
        let author = self.author.as_ref().ok_or_else(|| {
            Status::unimplemented("SubmitWorkflow: no workflow author wired on this gateway")
        })?;
        // SERVER-DERIVED identity (SN-8): the party the auth interceptor resolved.
        // The wire carries no party field, so a caller cannot assert who it is.
        let party = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();

        // Map the wire palette → gateway-core's vocabulary via the SHARED parse the G2
        // `RunApp` path reuses (through the host). The client supplies TOPOLOGY +
        // PARAMS only; UNSPECIFIED kinds are refused; DYNAMIC is reserved (the host
        // refuses it fail-closed, PR-1 frozen-only).
        let (steps, edges, mode) =
            author_steps_from_proto(req.steps, req.edges, req.execution_mode)?;

        // Compile + warrant-resolve SERVER-SIDE (identity + warrants derived from the
        // party's grants, never the client — the BLOCKER #5 fix). `BoundRecipe`
        // (react_seed = false) is reused; every Mote then flows the SAME admission as
        // Invoke/SubmitRun.
        let bound = author
            .author(&party, req.seed, &steps, &edges, mode, &req.context_bundles)
            .await
            .map_err(|e| match e {
                BinderError::NotAuthorized => Status::permission_denied("not authorized"),
                BinderError::InvalidArgs(detail) => Status::invalid_argument(detail),
                BinderError::Internal(detail) => Status::internal(detail),
            })?;

        // The Invoke tool-grant backstop: every server-built warrant's grants must
        // name a capability the host ACTUALLY registered on the broker (fail-closed
        // against provisioning drift). PR-6b-2: a `tool()` step grants a registered
        // tool; the LIVE broker set makes a runtime-dialed tool authorable.
        let fireable = self.fireable_grants();
        for (_, warrant) in &bound.motes {
            if let Some(grant) = warrant
                .tool_grants
                .iter()
                .find(|g| !fireable.contains(&(g.tool_id.0.clone(), g.tool_version.0.clone())))
            {
                return Err(Status::failed_precondition(format!(
                    "authored step grants tool {}@{} but this serve registered no such \
                     capability",
                    grant.tool_id.0, grant.tool_version.0
                )));
            }
        }

        // The SAME propose-proxy as Invoke/SubmitRun: register first, then submit each
        // compiled Mote (react_seed always false — no react kind in the Tier-1 palette).
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(submit_status)?;
        // The agentic-step chain key, derived BEFORE the consuming submit loop.
        let react_chain_salt = agentic_chain_salt(&bound.motes);
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, false)
                .await
                .map_err(submit_status)?;
        }

        Ok(Response::new(proto::RunHandle {
            instance_id: instance_id.to_vec(),
            recipe_fingerprint: bound.recipe_fingerprint.to_vec(),
            react_chain_salt,
        }))
    }

    async fn run_app(
        &self,
        request: Request<proto::RunAppRequest>,
    ) -> Result<Response<proto::RunHandle>, Status> {
        // G2: run a caller-owned App SERVER-SIDE so its `references.connections` +
        // `guards.secret_scope` are honored (the client-orchestrated `GetApp` →
        // `SubmitWorkflow` path drops them). `None` seam ⇒ `unimplemented` (clients
        // fall back to that legacy path — no regression).
        let runner = self.app_runner.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "RunApp: no app-run seam wired on this gateway \
                 (falls back to GetApp -> SubmitWorkflow)",
            )
        })?;
        // SERVER-DERIVED identity (SN-8): the party the auth interceptor resolved.
        let party = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();

        // The host reads the validated stored envelope, lowers its blueprint through
        // the canonical `kx-blueprint` path, resolves `references.connections` against
        // the caller's OWN registry, and sets the run warrant's secret scope from the
        // App's declared `guards.secret_scope`. Server-minted warrants (SN-8): the
        // envelope carries NO authority.
        let bound = runner
            // Interactive RunApp keeps today's posture (require_approval = false ⇒ the
            // serve-wide KX_SERVE_REQUIRE_APPROVAL default applies). A per-request
            // override field is a later additive; the per-TRIGGER posture is threaded on
            // the App-target trigger path (T-APP-TRIGGER-TARGET).
            .author_app(&party, &req.handle, &req.args, false)
            .await
            .map_err(|e| match e {
                crate::apps_run::AppRunError::NotAuthorized => {
                    Status::permission_denied("not authorized")
                }
                crate::apps_run::AppRunError::InvalidArgs(detail) => {
                    Status::invalid_argument(detail)
                }
                crate::apps_run::AppRunError::MissingIntegration(name) => {
                    Status::failed_precondition(format!(
                        "missing integration: {name} (register it with `kx connections add`)"
                    ))
                }
                crate::apps_run::AppRunError::Internal(detail) => Status::internal(detail),
            })?;

        // The SAME fireable-grant backstop as Invoke/SubmitWorkflow: every server-built
        // warrant's grants must name a capability the host ACTUALLY registered on the
        // broker (fail-closed against provisioning drift).
        let fireable = self.fireable_grants();
        for (_, warrant) in &bound.motes {
            if let Some(grant) = warrant
                .tool_grants
                .iter()
                .find(|g| !fireable.contains(&(g.tool_id.0.clone(), g.tool_version.0.clone())))
            {
                return Err(Status::failed_precondition(format!(
                    "app step grants tool {}@{} but this serve registered no such capability",
                    grant.tool_id.0, grant.tool_version.0
                )));
            }
        }

        // The SAME propose-proxy as Invoke/SubmitWorkflow: register first, then submit
        // each compiled Mote (react_seed always false — an agentic MODEL step drives
        // its own bounded loop via its Mote def, exactly as SubmitWorkflow authors it).
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(submit_status)?;
        // The agentic-step chain key, derived BEFORE the consuming submit loop (an
        // App whose blueprint carries one tool-granted MODEL step is agentic).
        let react_chain_salt = agentic_chain_salt(&bound.motes);
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, false)
                .await
                .map_err(submit_status)?;
        }

        Ok(Response::new(proto::RunHandle {
            instance_id: instance_id.to_vec(),
            recipe_fingerprint: bound.recipe_fingerprint.to_vec(),
            react_chain_salt,
        }))
    }

    async fn get_projection(
        &self,
        request: Request<proto::GetProjectionRequest>,
    ) -> Result<Response<proto::ProjectionView>, Status> {
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let view = view::build_view(self.reader.as_ref(), instance_id, req.at_seq)?;
        Ok(Response::new(view))
    }

    async fn get_content(
        &self,
        request: Request<proto::GetContentRequest>,
    ) -> Result<Response<proto::ContentBlob>, Status> {
        let req = request.into_inner();
        let content_ref = hash_32(&req.content_ref, "content_ref must be 32 bytes")?;
        // Batch A: an EMPTY instance_id selects the UPLOADS scope (previously a
        // hard invalid_argument — additive-safe). A 16-byte ticket takes the
        // original run-scope path byte-identically.
        let payload = if req.instance_id.is_empty() {
            view::get_uploaded_content(self.content.as_ref(), self.uploads.as_deref(), content_ref)?
        } else {
            let instance_id = instance_id_16(&req.instance_id)?;
            view::get_owned_content(
                self.reader.as_ref(),
                self.content.as_ref(),
                instance_id,
                content_ref,
            )?
        };
        Ok(Response::new(proto::ContentBlob { payload }))
    }

    async fn put_content(
        &self,
        request: Request<proto::PutContentRequest>,
    ) -> Result<Response<proto::PutContentResponse>, Status> {
        // BOTH seams are required: an upload the ledger cannot record would be
        // unreachable through the uploads scope (a silent blob leak).
        let (Some(writer), Some(uploads)) = (self.content_writer.as_ref(), self.uploads.as_ref())
        else {
            return Err(Status::unimplemented(
                "PutContent: no content writer / uploads ledger wired on this gateway",
            ));
        };
        // SERVER-DERIVED identity (SN-8): the party the auth interceptor stashed.
        let principal = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();

        // Fail-closed cap BEFORE hashing or touching the store (Rule 8c — the
        // first client write path never does unbounded work on oversized input).
        if req.payload.len() as u64 > self.put_cap_bytes {
            return Err(GatewayError::ResourceExhausted(
                "payload exceeds the server content cap (--content-max-bytes)",
            )
            .into());
        }

        let (content_ref, deduplicated) = writer.put(&req.payload)?;
        // Advisory audit row + the uploads-scope authorization. Wall-clock is
        // audit-only (off-digest, off-identity). Recorded AFTER the store write
        // so the ledger never names a ref the store does not hold.
        let uploaded_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        uploads.record(crate::uploads::UploadRecord {
            content_ref,
            media_type: req.media_type,
            filename: req.filename,
            principal,
            uploaded_ms,
        })?;

        Ok(Response::new(proto::PutContentResponse {
            content_ref: content_ref.to_vec(),
            size: req.payload.len() as u64,
            deduplicated,
        }))
    }

    async fn get_content_batch(
        &self,
        request: Request<proto::GetContentBatchRequest>,
    ) -> Result<Response<proto::GetContentBatchResponse>, Status> {
        let req = request.into_inner();
        // Fail-closed ref-count cap — refuse, never silently truncate.
        if req.content_refs.len() > MAX_BATCH_REFS {
            return Err(Status::invalid_argument(
                "GetContentBatch accepts at most 64 content_refs",
            ));
        }
        // EMPTY = uploads scope; 16 bytes = run scope; anything else is malformed.
        let instance_id = if req.instance_id.is_empty() {
            None
        } else {
            Some(instance_id_16(&req.instance_id)?)
        };
        // The effective per-item clamp: the client may only LOWER the server's
        // bound, so a full 64-item batch always fits the transport budget.
        let item_clamp = req
            .max_bytes_per_item
            .map_or(BATCH_ITEM_CLAMP_BYTES, |m| m.min(BATCH_ITEM_CLAMP_BYTES));
        let items = view::get_content_batch(
            self.reader.as_ref(),
            self.content.as_ref(),
            self.uploads.as_deref(),
            instance_id,
            &req.content_refs,
            item_clamp,
        )?;
        Ok(Response::new(proto::GetContentBatchResponse { items }))
    }

    async fn list_models(
        &self,
        _request: Request<proto::ListModelsRequest>,
    ) -> Result<Response<proto::ListModelsResponse>, Status> {
        // Display/discovery ONLY (SN-8): selection stays a recipe ENUM
        // free-param. An EMPTY catalog is the honest FFI-free answer; only a
        // gateway with no seam at all degrades to `unimplemented`.
        let models = self
            .models
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListModels: no model catalog wired"))?;
        // Model Control v2: recompute `active` live from the active-model selection
        // (the `loaded`-from-residency precedent) — an advisory display bit, SN-8.
        let active_id = self.active_model.as_ref().and_then(|a| a.get());
        let models = models
            .list()?
            .into_iter()
            .map(|m| {
                let active = active_id.as_deref() == Some(m.model_id.as_str());
                proto::ModelSummary {
                    model_id: m.model_id,
                    modalities: m.modalities,
                    description: m.description,
                    serving: m.serving,
                    context_len: m.context_len,
                    loaded: m.loaded,
                    chat_handle: m.chat_handle,
                    engine: m.engine,
                    can_embed: m.can_embed,
                    source: m.source,
                    active,
                    chat_rag_handle: m.chat_rag_handle,
                    embed_is_decoder: m.embed_is_decoder,
                }
            })
            .collect();
        Ok(Response::new(proto::ListModelsResponse { models }))
    }

    async fn get_server_info(
        &self,
        request: Request<proto::GetServerInfoRequest>,
    ) -> Result<Response<proto::GetServerInfoResponse>, Status> {
        // SERVER-DERIVED identity (SN-8): config facts go ONLY to an authenticated
        // caller (the interceptor-resolved party). The wire carries no party field,
        // so a caller cannot assert who it is; an unresolved caller is refused.
        let _party = request
            .extensions()
            .get::<CallerParty>()
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let facts = self
            .server_info
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GetServerInfo: no server info wired"))?;
        // A pure projection of the NON-SECRET facts — there is no token / TLS-key
        // field on `ServerInfoFacts` to leak (POC-2 token-never-leaks, type-level).
        Ok(Response::new(proto::GetServerInfoResponse {
            model_id: facts.model_id.clone(),
            model_path: facts.model_path.clone(),
            listen_addr: facts.listen_addr.clone(),
            ws_addr: facts.ws_addr.clone(),
            console_addr: facts.console_addr.clone(),
            metrics_addr: facts.metrics_addr.clone(),
            content_root: facts.content_root.clone(),
            journal_path: facts.journal_path.clone(),
            catalog_dir: facts.catalog_dir.clone(),
            max_lease: facts.max_lease,
            content_max_bytes: facts.content_max_bytes,
            cors_origins: facts.cors_origins.clone(),
            tls_enabled: facts.tls_enabled,
            auth_mode: facts.auth_mode.clone(),
            feature_hnsw: facts.feature_hnsw,
            feature_inference: facts.feature_inference,
            feature_console: facts.feature_console,
            feature_vision: facts.feature_vision,
            audit_log_enabled: facts.audit_log_enabled,
            react_max_turns: facts.react_max_turns,
            react_max_tool_calls: facts.react_max_tool_calls,
            embed_model_id: facts.embed_model_id.clone(),
            // Model Control v2: the active default (advisory) + the download posture.
            active_model_id: self
                .active_model
                .as_ref()
                .and_then(|a| a.get())
                .unwrap_or_default(),
            allow_model_pull: facts.allow_model_pull,
            // RC4a (T-RAG-EMBED-QUALITY): honest decoder-as-embedder advisory flag.
            embed_model_is_decoder: facts.embed_model_is_decoder,
            // The resolved embedded-worker pool size (1 = single worker).
            worker_pool: facts.worker_pool,
        }))
    }

    async fn load_model(
        &self,
        request: Request<proto::LoadModelRequest>,
    ) -> Result<Response<proto::LoadModelResponse>, Status> {
        // SERVER-DERIVED identity (SN-8): a mutating control op needs an
        // authenticated caller (the interceptor-resolved party); the wire carries
        // no party field. Off-journal, off-digest — pure RAM residency.
        let _party = request
            .extensions()
            .get::<CallerParty>()
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let control = self
            .model_lifecycle
            .as_ref()
            .ok_or_else(|| Status::unimplemented("LoadModel: no model lifecycle wired"))?;
        // An unregistered id ⇒ NotFound → not_found (fail-closed; never warms an
        // arbitrary path).
        let out = control.load(&request.into_inner().model_id)?;
        Ok(Response::new(proto::LoadModelResponse {
            model_id: out.model_id,
            loaded: out.loaded,
            was_resident: out.was_resident,
        }))
    }

    async fn offload_model(
        &self,
        request: Request<proto::OffloadModelRequest>,
    ) -> Result<Response<proto::OffloadModelResponse>, Status> {
        let _party = request
            .extensions()
            .get::<CallerParty>()
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let control = self
            .model_lifecycle
            .as_ref()
            .ok_or_else(|| Status::unimplemented("OffloadModel: no model lifecycle wired"))?;
        let out = control.offload(&request.into_inner().model_id)?;
        Ok(Response::new(proto::OffloadModelResponse {
            model_id: out.model_id,
            loaded: out.loaded,
            was_resident: out.was_resident,
        }))
    }

    // ----- Model Control v2 — acquire (pull + runtime-register) + switch -----

    async fn pull_model(
        &self,
        request: Request<proto::PullModelRequest>,
    ) -> Result<Response<proto::PullModelResponse>, Status> {
        // SERVER-DERIVED identity (SN-8): an authenticated caller may REQUEST a pull;
        // the operator's env opt-in (enforced in the host puller) AUTHORIZES the
        // egress. The wire carries no party field.
        let _party = request
            .extensions()
            .get::<CallerParty>()
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let puller = self
            .model_puller
            .as_ref()
            .ok_or_else(|| Status::unimplemented("PullModel: no model puller wired"))?;
        let req = request.into_inner();
        // Discriminate the proto `oneof` into the host vocabulary; a missing/empty
        // source is an invalid request (never an egress).
        let source = match req.source {
            Some(proto::pull_model_request::Source::OllamaTag(tag)) if !tag.trim().is_empty() => {
                crate::model_pull::PullSource::OllamaTag(tag)
            }
            Some(proto::pull_model_request::Source::Url(url)) if !url.trim().is_empty() => {
                // SHA-256 is REQUIRED for a direct-URL pull (the bytes are otherwise
                // untrusted) — refuse at the boundary before any egress.
                if req.sha256.trim().is_empty() {
                    return Ok(Response::new(proto::PullModelResponse {
                        model_id: String::new(),
                        accepted: false,
                        detail: "a sha256 is required for a direct --url pull (the download \
                                 is verified before it is registered)"
                            .to_string(),
                    }));
                }
                crate::model_pull::PullSource::Url {
                    url,
                    sha256: req.sha256,
                }
            }
            _ => {
                return Err(Status::invalid_argument(
                    "PullModel requires a non-empty ollama_tag or url source",
                ));
            }
        };
        let resp = match puller.start(source, req.model_id.trim()) {
            crate::model_pull::PullAdmission::Accepted { model_id } => proto::PullModelResponse {
                model_id,
                accepted: true,
                detail: String::new(),
            },
            crate::model_pull::PullAdmission::Refused { detail } => proto::PullModelResponse {
                model_id: String::new(),
                accepted: false,
                detail,
            },
        };
        Ok(Response::new(resp))
    }

    async fn get_pull_status(
        &self,
        request: Request<proto::GetPullStatusRequest>,
    ) -> Result<Response<proto::GetPullStatusResponse>, Status> {
        let _party = request
            .extensions()
            .get::<CallerParty>()
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let puller = self
            .model_puller
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GetPullStatus: no model puller wired"))?;
        let model_id = request.into_inner().model_id;
        // An unknown id ⇒ NOT_FOUND (never a fabricated progress).
        let progress = puller
            .status(&model_id)
            .ok_or_else(|| Status::not_found("no pull is tracked for that model id"))?;
        let phase = match progress.phase {
            crate::model_pull::PullPhase::Resolving => {
                proto::get_pull_status_response::Phase::Resolving
            }
            crate::model_pull::PullPhase::Downloading => {
                proto::get_pull_status_response::Phase::Downloading
            }
            crate::model_pull::PullPhase::Verifying => {
                proto::get_pull_status_response::Phase::Verifying
            }
            crate::model_pull::PullPhase::Registering => {
                proto::get_pull_status_response::Phase::Registering
            }
            crate::model_pull::PullPhase::Done => proto::get_pull_status_response::Phase::Done,
            crate::model_pull::PullPhase::Failed => proto::get_pull_status_response::Phase::Failed,
        };
        Ok(Response::new(proto::GetPullStatusResponse {
            phase: phase as i32,
            bytes_downloaded: progress.bytes_downloaded,
            bytes_total: progress.bytes_total,
            detail: progress.detail,
        }))
    }

    async fn set_active_model(
        &self,
        request: Request<proto::SetActiveModelRequest>,
    ) -> Result<Response<proto::SetActiveModelResponse>, Status> {
        let _party = request
            .extensions()
            .get::<CallerParty>()
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let control = self.active_model.as_ref().ok_or_else(|| {
            Status::unimplemented("SetActiveModel: no active-model control wired")
        })?;
        // An unknown id ⇒ NotFound → not_found (fail-closed; never an unrouteable active model).
        let active = control.set(request.into_inner().model_id.trim())?;
        Ok(Response::new(proto::SetActiveModelResponse {
            active_model_id: active.unwrap_or_default(),
        }))
    }

    // ----- POC-4 — App catalog (save / list / get; off-journal apps.db) -----

    async fn save_app(
        &self,
        request: Request<proto::SaveAppRequest>,
    ) -> Result<Response<proto::SaveAppResponse>, Status> {
        let apps = self.apps.as_ref().ok_or_else(|| {
            Status::unimplemented("SaveApp: no App catalog wired (apps.db absent)")
        })?;
        // SERVER-DERIVED identity (SN-8): apps are scoped to the auth-resolved party.
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.handle) {
            return Err(Status::invalid_argument(
                "handle must be a 'namespace/collection/name' AssetPath ([a-z0-9._-] segments)",
            ));
        }
        if req.envelope_json.is_empty() {
            return Err(Status::invalid_argument("envelope_json must not be empty"));
        }
        if req.envelope_json.len() > crate::MAX_APP_ENVELOPE_BYTES {
            return Err(Status::invalid_argument(
                "app envelope exceeds the server cap (1 MiB)",
            ));
        }
        // POC-5d — a LOCKED App is fully frozen: the lock refuses an agentic in-CAS
        // FILE edit (AdvanceBranch, POC-5b) AND a STRUCTURE edit (a re-save of the
        // App envelope/blueprint from the lineage editor). One-App-one-branch ⇒ the
        // lock is keyed by the App's own handle. A real lock-store error fails closed;
        // an absent lock seam degrades open (additive feature). This is an off-journal
        // availability gate (the digest is unaffected); the run path still re-resolves
        // every warrant from the caller's grants (SN-8).
        if let Some(locks) = self.locks.as_ref() {
            if locks.is_locked(&principal, &req.handle)? {
                return Err(with_refusal_code(
                    Status::failed_precondition(
                        "app is locked; structure edits are refused (unlock the App to save)",
                    ),
                    crate::locks_view::LOCKED_BRANCH_REFUSAL_CODE,
                ));
            }
        }
        // The host validates + canonicalizes the envelope and derives app_ref +
        // the summary (it carries NO authority — a bad envelope ⇒ InvalidArgument).
        let (record, deduplicated) = apps.save(&principal, &req.handle, &req.envelope_json)?;
        Ok(Response::new(proto::SaveAppResponse {
            app_ref: record.app_ref.to_vec(),
            handle: record.handle,
            deduplicated,
        }))
    }

    async fn list_apps(
        &self,
        request: Request<proto::ListAppsRequest>,
    ) -> Result<Response<proto::ListAppsResponse>, Status> {
        let apps = self.apps.as_ref().ok_or_else(|| {
            Status::unimplemented("ListApps: no App catalog wired (apps.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            100
        } else {
            (req.limit as usize).min(256)
        };
        let after = if req.after_handle.is_empty() {
            None
        } else {
            Some(req.after_handle.as_str())
        };
        let (records, has_more) = apps.list(&principal, limit, after)?;
        // POC-5b: enrich `locked` from the lock store (best-effort display — the
        // authoritative refusal is at the AdvanceBranch chokepoint). One-App-one-
        // branch ⇒ the lock is keyed by the App's own handle.
        let locks = self.locks.as_ref();
        let apps_out = records
            .into_iter()
            .map(|r| {
                let mut s = app_record_to_proto(r);
                if let Some(l) = locks {
                    s.locked = l.is_locked(&principal, &s.handle).unwrap_or(false);
                }
                s
            })
            .collect();
        Ok(Response::new(proto::ListAppsResponse {
            apps: apps_out,
            has_more,
        }))
    }

    async fn get_app(
        &self,
        request: Request<proto::GetAppRequest>,
    ) -> Result<Response<proto::GetAppResponse>, Status> {
        let apps = self.apps.as_ref().ok_or_else(|| {
            Status::unimplemented("GetApp: no App catalog wired (apps.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        // Uniform not-found for absent OR not-owned (no cross-party existence oracle).
        match apps.get(&principal, &req.handle)? {
            Some((record, envelope_json)) => {
                let mut summary = app_record_to_proto(record);
                if let Some(l) = self.locks.as_ref() {
                    summary.locked = l.is_locked(&principal, &summary.handle).unwrap_or(false);
                }
                Ok(Response::new(proto::GetAppResponse {
                    found: true,
                    envelope_json,
                    summary: Some(summary),
                }))
            }
            None => Ok(Response::new(proto::GetAppResponse {
                found: false,
                envelope_json: Vec::new(),
                summary: None,
            })),
        }
    }

    // ----- skill catalog (add / list / form / remove; off-journal skills.db) -----

    async fn add_skill(
        &self,
        request: Request<proto::AddSkillRequest>,
    ) -> Result<Response<proto::AddSkillResponse>, Status> {
        let skills = self.skills.as_ref().ok_or_else(|| {
            Status::unimplemented("AddSkill: no skill catalog wired (skills.db absent)")
        })?;
        // SERVER-DERIVED identity (SN-8): skills are scoped to the auth-resolved party.
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if req.manifest_json.is_empty() {
            return Err(Status::invalid_argument("manifest_json must not be empty"));
        }
        // Fail-closed caps BEFORE any store touch (mirrors kx-skill's own parse caps;
        // the host pins the equality).
        if req.manifest_json.len() > crate::skills_view::MAX_SKILL_MANIFEST_BYTES {
            return Err(Status::invalid_argument(
                "skill manifest exceeds the server cap (64 KiB)",
            ));
        }
        if req.instructions_body.len() > crate::skills_view::MAX_SKILL_INSTRUCTIONS_BODY_BYTES {
            return Err(Status::invalid_argument(
                "skill instructions exceed the server cap (256 KiB)",
            ));
        }
        // PACK form: a body rides the request — store it via the ONE content-write
        // seam (SN-8: the ref is server-derived; no uploads-ledger coupling — a
        // server-authored skill body is not a client upload). STORED form: an empty
        // body means the manifest must already name instructions_ref (host-enforced).
        let instructions = if req.instructions_body.is_empty() {
            None
        } else {
            let writer = self.content_writer.as_ref().ok_or_else(|| {
                Status::unimplemented("AddSkill: no content-write seam wired on this gateway")
            })?;
            let (content_ref, _dedup) = writer.put(&req.instructions_body)?;
            let (preview, truncated) = skill_preview(&req.instructions_body);
            Some(crate::skills_view::AddedInstructions {
                content_ref,
                preview,
                truncated,
            })
        };
        // The host validates + canonicalizes the manifest (authority deny-keys fail
        // closed) and derives skill_ref + the record.
        let (record, deduplicated) = skills.add(&principal, &req.manifest_json, instructions)?;
        Ok(Response::new(proto::AddSkillResponse {
            skill_ref: record.skill_ref.to_vec(),
            name: record.name,
            instructions_ref: record.instructions_ref,
            deduplicated,
        }))
    }

    async fn list_skills(
        &self,
        request: Request<proto::ListSkillsRequest>,
    ) -> Result<Response<proto::ListSkillsResponse>, Status> {
        let skills = self.skills.as_ref().ok_or_else(|| {
            Status::unimplemented("ListSkills: no skill catalog wired (skills.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            100
        } else {
            (req.limit as usize).min(256)
        };
        let after = if req.after_name.is_empty() {
            None
        } else {
            Some(req.after_name.as_str())
        };
        let (records, has_more) = skills.list(&principal, limit, after)?;
        Ok(Response::new(proto::ListSkillsResponse {
            skills: records.into_iter().map(skill_record_to_proto).collect(),
            has_more,
        }))
    }

    async fn get_skill_form(
        &self,
        request: Request<proto::GetSkillFormRequest>,
    ) -> Result<Response<proto::GetSkillFormResponse>, Status> {
        let skills = self.skills.as_ref().ok_or_else(|| {
            Status::unimplemented("GetSkillForm: no skill catalog wired (skills.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        // Uniform not-found for absent OR not-owned (no cross-party existence oracle).
        match skills.get(&principal, &req.name)? {
            Some(record) => {
                // ADVISORY display enrichment (never a grant): a wish is "registered"
                // iff the serve could currently fire it — the same live-broker /
                // static-snapshot truth the admission backstops intersect against.
                let fireable = self.fireable_grants();
                let wishes = record
                    .tools
                    .iter()
                    .map(|(id, version)| proto::SkillWish {
                        tool_id: id.clone(),
                        tool_version: version.clone(),
                        registered: fireable.contains(&(id.clone(), version.clone())),
                    })
                    .collect();
                let preview = record.instructions_preview.clone();
                let truncated = record.preview_truncated;
                Ok(Response::new(proto::GetSkillFormResponse {
                    found: true,
                    summary: Some(skill_record_to_proto(record)),
                    wishes,
                    instructions_preview: preview,
                    preview_truncated: truncated,
                }))
            }
            None => Ok(Response::new(proto::GetSkillFormResponse {
                found: false,
                summary: None,
                wishes: Vec::new(),
                instructions_preview: String::new(),
                preview_truncated: false,
            })),
        }
    }

    async fn remove_skill(
        &self,
        request: Request<proto::RemoveSkillRequest>,
    ) -> Result<Response<proto::RemoveSkillResponse>, Status> {
        let skills = self.skills.as_ref().ok_or_else(|| {
            Status::unimplemented("RemoveSkill: no skill catalog wired (skills.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let removed = skills.remove(&principal, &req.name)?;
        Ok(Response::new(proto::RemoveSkillResponse { removed }))
    }

    async fn get_mote_detail(
        &self,
        request: Request<proto::GetMoteDetailRequest>,
    ) -> Result<Response<proto::MoteDetail>, Status> {
        // Display ONLY (SN-8): the def is read back for inspection — nothing
        // here authorizes anything. Only a gateway with no seam at all
        // degrades to `unimplemented` (the ListModels pattern).
        let defs = self.mote_defs.as_ref().ok_or_else(|| {
            Status::unimplemented("GetMoteDetail: no mote-def view wired on this gateway")
        })?;
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let mote_id = hash_32(&req.mote_id, "mote_id must be 32 bytes")?;
        let detail = crate::mote_detail::mote_detail(
            self.reader.as_ref(),
            defs.as_ref(),
            instance_id,
            mote_id,
        )?;
        Ok(Response::new(detail))
    }

    type StreamEventsStream = EventStream;

    async fn stream_events(
        &self,
        request: Request<proto::StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        // Delegate to the injected tailer (default snapshot-to-head; the host
        // wires a live tailer via `with_event_tailer`). Ownership is the tailer's
        // first action → uniform `permission_denied`.
        let stream = self
            .tailer
            .stream(self.reader.clone(), instance_id, req.since_seq)?;
        Ok(Response::new(stream))
    }

    type StreamModelTokensStream = TokenStream;

    async fn stream_model_tokens(
        &self,
        request: Request<proto::StreamModelTokensRequest>,
    ) -> Result<Response<Self::StreamModelTokensStream>, Status> {
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let mote_id = hash_32(&req.mote_id, "mote_id must be 32 bytes")?;
        // Delegate to the injected tailer (default EMPTY stream; the inference
        // build wires a broker-backed live tailer via `with_token_tailer`). The
        // ownership gate — caller owns `instance_id` AND `mote_id` belongs to that
        // run — is the tailer's first action → uniform `permission_denied`. The
        // stream is ADVISORY; the committed `result_ref` stays the authority.
        let stream =
            self.token_tailer
                .stream(self.reader.clone(), instance_id, mote_id, req.since_seq)?;
        Ok(Response::new(stream))
    }

    type StreamAllEventsStream = GlobalEventStream;

    async fn stream_all_events(
        &self,
        request: Request<proto::StreamAllEventsRequest>,
    ) -> Result<Response<Self::StreamAllEventsStream>, Status> {
        let req = request.into_inner();
        // Batch C: the GLOBAL cross-run tail. No ownership gate by design —
        // operator-global on single-node OSS, gated solely by the host auth
        // interceptor; CLOUD must party-scope or deny (the proto flag).
        let stream = self
            .global_tailer
            .stream_all(self.reader.clone(), req.since_seq)?;
        Ok(Response::new(stream))
    }

    async fn list_signatures(
        &self,
        _request: Request<proto::ListSignaturesRequest>,
    ) -> Result<Response<proto::ListSignaturesResponse>, Status> {
        let catalog = self
            .catalog
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListSignatures: no signature catalog wired"))?;
        let signatures = catalog
            .list()
            .into_iter()
            .map(|e| proto::SignatureSummary {
                signature_id: e.signature_id.to_vec(),
                name: e.name,
            })
            .collect();
        Ok(Response::new(proto::ListSignaturesResponse { signatures }))
    }

    async fn get_signature(
        &self,
        request: Request<proto::GetSignatureRequest>,
    ) -> Result<Response<proto::GetSignatureResponse>, Status> {
        let catalog = self
            .catalog
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GetSignature: no signature catalog wired"))?;
        let id = hash_32(
            &request.into_inner().signature_id,
            "signature_id must be 32 bytes",
        )?;
        // A public discovery surface: `not_found` here is intended (the catalog is
        // authoritative for WHAT recipes exist), NOT collapsed like the Invoke
        // execution surface.
        let manifest = catalog
            .get(&id)
            .ok_or_else(|| Status::not_found("signature not found"))?;
        Ok(Response::new(proto::GetSignatureResponse {
            signature_id: id.to_vec(),
            manifest,
        }))
    }

    async fn register_signature(
        &self,
        request: Request<proto::RegisterSignatureRequest>,
    ) -> Result<Response<proto::RegisterSignatureResponse>, Status> {
        let catalog = self.catalog.as_ref().ok_or_else(|| {
            Status::unimplemented("RegisterSignature: no signature catalog wired")
        })?;
        // The host server-derives the id from the decoded manifest (SN-8) and the
        // registry enforces idempotency + immutability.
        let registered = catalog
            .register(&request.into_inner().manifest)
            .map_err(|e| match e {
                CatalogSeamError::ImmutabilityConflict => {
                    Status::failed_precondition("immutable catalog conflict")
                }
                CatalogSeamError::Malformed(detail) => Status::invalid_argument(detail),
                CatalogSeamError::Internal(detail) => Status::internal(detail),
            })?;
        Ok(Response::new(proto::RegisterSignatureResponse {
            signature_id: registered.signature_id.to_vec(),
        }))
    }

    async fn list_runs(
        &self,
        request: Request<proto::ListRunsRequest>,
    ) -> Result<Response<proto::ListRunsResponse>, Status> {
        let req = request.into_inner();
        // A read-only fold over the run-registration facts (off-digest). Always
        // available (no seam) — it needs only the journal reader the service holds.
        let resp = crate::runs::list_runs(self.reader.as_ref(), req.limit, req.before_seq)?;
        Ok(Response::new(resp))
    }

    async fn get_run_inputs(
        &self,
        request: Request<proto::GetRunInputsRequest>,
    ) -> Result<Response<proto::GetRunInputsResponse>, Status> {
        // PR-D ("Re-run with changes"): return the args captured at `Invoke` so a
        // run recovered from `ListRuns` (no client-side localStorage) can pre-fill
        // its recipe form and re-invoke. A serve without the sidecar wired degrades
        // forward-compatibly to `unimplemented`; a run with nothing captured (pre-
        // PR-D, or a rebuilt-to-empty sidecar) is an honest `not_found`. No
        // read-time party filter (single-tenant; the kx-cloud SN-8 wall is above).
        let store = self.run_inputs.as_ref().ok_or_else(|| {
            Status::unimplemented("GetRunInputs: no run-inputs store wired (run_inputs.db absent)")
        })?;
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let entry = store
            .get(&instance_id)?
            .ok_or_else(|| Status::not_found("run inputs not captured for this run"))?;
        Ok(Response::new(proto::GetRunInputsResponse {
            instance_id: entry.instance_id.to_vec(),
            recipe_fingerprint: entry.recipe_fingerprint.to_vec(),
            handle: entry.handle,
            args: entry.args,
        }))
    }

    async fn list_replan_rounds(
        &self,
        request: Request<proto::ListReplanRoundsRequest>,
    ) -> Result<Response<proto::ListReplanRoundsResponse>, Status> {
        let req = request.into_inner();
        // PR-2c-2: a read-only fold over the off-DAG ReplanRound facts (the live
        // re-plan loop's self-correction trail). Always available (no seam).
        let resp = crate::replan::list_replan_rounds(self.reader.as_ref(), req.limit)?;
        Ok(Response::new(resp))
    }

    async fn list_react_turns(
        &self,
        request: Request<proto::ListReactTurnsRequest>,
    ) -> Result<Response<proto::ListReactTurnsResponse>, Status> {
        let req = request.into_inner();
        // PR-2d-1: a read-only fold over the off-DAG ReactRound facts (the live
        // ReAct chain's anchor + settled branches). Always available (no seam).
        let resp = crate::react::list_react_turns(
            self.reader.as_ref(),
            Some(self.content.as_ref()), // decode the anchor warrant for the governance axes
            req.limit,
            req.instance_id.as_deref(),
            req.step_salt.as_deref(), // PR-R1: optional per-chain scope
        )?;
        Ok(Response::new(resp))
    }

    async fn list_re_rank_turns(
        &self,
        request: Request<proto::ListReRankTurnsRequest>,
    ) -> Result<Response<proto::ListReRankTurnsResponse>, Status> {
        let req = request.into_inner();
        // RC4c-2: a read-only fold over the off-DAG ReRankRound facts (the live LLM
        // listwise rerank turn's anchor + frozen outcome). Always available (no seam).
        let resp = crate::rerank::list_rerank_turns(
            self.reader.as_ref(),
            req.limit,
            req.instance_id.as_deref(),
        )?;
        Ok(Response::new(resp))
    }

    async fn score_run(
        &self,
        request: Request<proto::ScoreRunRequest>,
    ) -> Result<Response<proto::RunScore>, Status> {
        // RC1 (D172): an expectation-free per-run quality fold over the off-DAG
        // ReactRound trajectory (kx-eval `analyze_run`). Read-only, off-digest, always
        // available — the same posture as ListReactTurns (no seam).
        let req = request.into_inner();
        let resp = crate::eval::score_run(self.reader.as_ref(), &req.instance_id)?;
        Ok(Response::new(resp))
    }

    async fn list_capture_records(
        &self,
        request: Request<proto::ListCaptureRecordsRequest>,
    ) -> Result<Response<proto::ListCaptureRecordsResponse>, Status> {
        // Campaign Batch 2 (the Morphic Data Engine): a read-only page over the
        // host's durable capture.db action projection. A serve without the
        // sidecar wired degrades forward-compatibly to `unimplemented`.
        let capture = self.capture.as_ref().ok_or_else(|| {
            Status::unimplemented("ListCaptureRecords: no capture view wired (capture.db absent)")
        })?;
        let req = request.into_inner();
        let instance_id: Option<[u8; 16]> = match req.instance_id {
            None => None,
            Some(raw) => Some(<[u8; 16]>::try_from(raw.as_slice()).map_err(|_| {
                Status::invalid_argument("capture instance_id filter must be 16 bytes")
            })?),
        };
        // Clamp to the same page bounds the read-fold RPCs use (1..=500, default 200).
        let page = req.limit.map_or(200usize, |l| (l as usize).clamp(1, 500));
        let (records, has_more) = capture.list(page, instance_id)?;
        let records = records
            .into_iter()
            .map(|r| proto::CaptureRecordSummary {
                mote_id: r.mote_id.to_vec(),
                instance_id: r.instance_id.to_vec(),
                result_ref: r.result_ref.to_vec(),
                nd_class: r.nd_class,
                seq: r.seq,
                react_turn: r.react_turn,
                react_branch: r.react_branch,
            })
            .collect();
        Ok(Response::new(proto::ListCaptureRecordsResponse {
            records,
            has_more,
        }))
    }

    async fn list_mote_telemetry(
        &self,
        request: Request<proto::ListMoteTelemetryRequest>,
    ) -> Result<Response<proto::ListMoteTelemetryResponse>, Status> {
        // Batch C: a read-only page over the host's telemetry.db execution
        // exhaust. A serve without the sidecar wired degrades forward-compatibly
        // to `unimplemented`.
        let telemetry = self.telemetry.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "ListMoteTelemetry: no telemetry view wired (telemetry.db absent)",
            )
        })?;
        let req = request.into_inner();
        let instance_id: Option<[u8; 16]> = match req.instance_id {
            None => None,
            Some(raw) => Some(<[u8; 16]>::try_from(raw.as_slice()).map_err(|_| {
                Status::invalid_argument("telemetry instance_id filter must be 16 bytes")
            })?),
        };
        let mote_id: Option<[u8; 32]> = match req.mote_id {
            None => None,
            Some(raw) => Some(<[u8; 32]>::try_from(raw.as_slice()).map_err(|_| {
                Status::invalid_argument("telemetry mote_id filter must be 32 bytes")
            })?),
        };
        // Clamp to the same page bounds the read-fold RPCs use (1..=500, default 200).
        let page = req.limit.map_or(200usize, |l| (l as usize).clamp(1, 500));
        let (rows, has_more) = telemetry.list(page, instance_id, mote_id, req.before_seq)?;
        let rows = rows
            .into_iter()
            .map(|r| proto::MoteTelemetryRow {
                mote_id: r.mote_id.to_vec(),
                instance_id: r.instance_id.to_vec(),
                wall_clock_ms: r.wall_clock_ms,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                model_id: r.model_id,
                tool_id: r.tool_id,
                started_unix_ms: r.started_unix_ms,
                seq: r.seq,
            })
            .collect();
        Ok(Response::new(proto::ListMoteTelemetryResponse {
            rows,
            has_more,
        }))
    }

    async fn list_telemetry_summary(
        &self,
        request: Request<proto::ListTelemetrySummaryRequest>,
    ) -> Result<Response<proto::ListTelemetrySummaryResponse>, Status> {
        // W1a-3: the exact, cross-page per-model token rollup over the same
        // telemetry.db sidecar. A serve without it degrades forward-compatibly
        // to `unimplemented`.
        let telemetry = self.telemetry.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "ListTelemetrySummary: no telemetry view wired (telemetry.db absent)",
            )
        })?;
        let req = request.into_inner();
        let instance_id: Option<[u8; 16]> = match req.instance_id {
            None => None,
            Some(raw) => Some(<[u8; 16]>::try_from(raw.as_slice()).map_err(|_| {
                Status::invalid_argument("telemetry instance_id filter must be 16 bytes")
            })?),
        };
        let summary = telemetry.summarize(instance_id)?;
        let rows = summary
            .rows
            .into_iter()
            .map(|r| proto::ModelTokenRollup {
                model_id: r.model_id,
                count: r.count,
                total_output_tokens: r.total_output_tokens,
                total_wall_clock_ms: r.total_wall_clock_ms,
            })
            .collect();
        Ok(Response::new(proto::ListTelemetrySummaryResponse {
            rows,
            total_motes: summary.total_motes,
            total_output_tokens: summary.total_output_tokens,
        }))
    }

    async fn submit_feedback(
        &self,
        request: Request<proto::SubmitFeedbackRequest>,
    ) -> Result<Response<proto::SubmitFeedbackResponse>, Status> {
        // PR-4.1: a client-origin write into the rebuildable-to-empty feedback.db
        // sidecar. A serve without the seam degrades forward-compatibly.
        let store = self.feedback.as_ref().ok_or_else(|| {
            Status::unimplemented("SubmitFeedback: no feedback store wired (feedback.db absent)")
        })?;
        // SERVER-DERIVED identity (SN-8): the party the auth interceptor stashed —
        // never the wire request.
        let principal = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();

        // Rating MUST be UP/DOWN (UNSPECIFIED/unknown ⇒ fail-closed).
        let rating = match proto::FeedbackRating::try_from(req.rating) {
            Ok(proto::FeedbackRating::Up) => proto::FeedbackRating::Up as i32,
            Ok(proto::FeedbackRating::Down) => proto::FeedbackRating::Down as i32,
            _ => {
                return Err(Status::invalid_argument(
                    "feedback rating must be FEEDBACK_RATING_UP or FEEDBACK_RATING_DOWN",
                ))
            }
        };
        if req.message_id.is_empty() {
            return Err(Status::invalid_argument("feedback message_id is required"));
        }
        // Fail-closed comment cap BEFORE the write (never unbounded sidecar rows).
        if req.comment.len() > MAX_FEEDBACK_COMMENT_BYTES {
            return Err(Status::invalid_argument(
                "feedback comment exceeds the 4 KiB cap",
            ));
        }
        let instance_id = opt_fixed::<16>(req.instance_id, "feedback instance_id")?;
        let mote_id = opt_fixed::<32>(req.mote_id, "feedback mote_id")?;
        let content_ref = opt_fixed::<32>(req.content_ref, "feedback content_ref")?;

        // SERVER-derived, DETERMINISTIC id over (message_id, principal): a
        // re-rating of the SAME answer by the SAME party maps to the SAME id, so
        // the host's `INSERT OR REPLACE` overwrites (the "changed my mind" UX).
        // SN-8: the client can neither name nor forge it.
        let mut keyed = Vec::with_capacity(16 + req.message_id.len() + 1 + principal.len());
        keyed.extend_from_slice(b"kx-feedback-id\0");
        keyed.extend_from_slice(req.message_id.as_bytes());
        keyed.push(0);
        keyed.extend_from_slice(principal.as_bytes());
        let mut feedback_id = [0u8; 16];
        feedback_id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);

        let submitted_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));

        store.record(crate::feedback_view::FeedbackRecord {
            feedback_id,
            rating,
            message_id: req.message_id,
            instance_id,
            mote_id,
            content_ref,
            comment: req.comment,
            recipe_handle: req.recipe_handle,
            model_id: req.model_id,
            principal,
            submitted_unix_ms,
        })?;

        Ok(Response::new(proto::SubmitFeedbackResponse {
            feedback_id: feedback_id.to_vec(),
        }))
    }

    async fn list_feedback(
        &self,
        request: Request<proto::ListFeedbackRequest>,
    ) -> Result<Response<proto::ListFeedbackResponse>, Status> {
        let store = self.feedback.as_ref().ok_or_else(|| {
            Status::unimplemented("ListFeedback: no feedback store wired (feedback.db absent)")
        })?;
        let req = request.into_inner();
        let instance_id: Option<[u8; 16]> = match req.instance_id {
            None => None,
            Some(raw) => Some(<[u8; 16]>::try_from(raw.as_slice()).map_err(|_| {
                Status::invalid_argument("feedback instance_id filter must be 16 bytes")
            })?),
        };
        // The same page bounds the read-fold RPCs use (1..=500, default 200).
        let page = req.limit.map_or(200usize, |l| (l as usize).clamp(1, 500));
        let (rows, has_more) = store.list(page, instance_id, req.before_rowid)?;
        let rows = rows
            .into_iter()
            .map(|r| proto::FeedbackRow {
                feedback_id: r.feedback_id.to_vec(),
                rating: r.rating,
                message_id: r.message_id,
                instance_id: r.instance_id.to_vec(),
                mote_id: r.mote_id.to_vec(),
                content_ref: r.content_ref.to_vec(),
                comment: r.comment,
                recipe_handle: r.recipe_handle,
                model_id: r.model_id,
                submitted_unix_ms: r.submitted_unix_ms,
                rowid: r.rowid,
            })
            .collect();
        Ok(Response::new(proto::ListFeedbackResponse {
            rows,
            has_more,
        }))
    }

    async fn list_alerts(
        &self,
        request: Request<proto::ListAlertsRequest>,
    ) -> Result<Response<proto::ListAlertsResponse>, Status> {
        // W1a-2: a read-only page over the host's alerts.db read-cache (folded
        // from the journal's terminal `Failed` facts). A serve without the
        // sidecar wired degrades forward-compatibly to `unimplemented`. The
        // triage lifecycle (ack/resolve) is a Cloud capability (D156) — there is
        // no mutate RPC here.
        let view = self.alerts.as_ref().ok_or_else(|| {
            Status::unimplemented("ListAlerts: no alerts view wired (alerts.db absent)")
        })?;
        let req = request.into_inner();
        let instance_id: Option<[u8; 16]> = match req.instance_id {
            None => None,
            Some(raw) => Some(<[u8; 16]>::try_from(raw.as_slice()).map_err(|_| {
                Status::invalid_argument("alerts instance_id filter must be 16 bytes")
            })?),
        };
        // The same page bounds the read-fold RPCs use (1..=500, default 200).
        let page = req.limit.map_or(200usize, |l| (l as usize).clamp(1, 500));
        let (rows, has_more) = view.list(page, instance_id, req.before_seq)?;
        let alerts = rows
            .into_iter()
            .map(|r| proto::AlertSummary {
                alert_id: r.alert_id.to_vec(),
                mote_id: r.mote_id.to_vec(),
                instance_id: r.instance_id.to_vec(),
                reason_class: r.reason_class,
                severity: r.severity,
                seq: r.seq,
                created_unix_ms: r.created_unix_ms,
                reason_code: r.reason_code,
            })
            .collect();
        Ok(Response::new(proto::ListAlertsResponse {
            alerts,
            has_more,
        }))
    }

    async fn register_tool(
        &self,
        request: Request<proto::RegisterToolRequest>,
    ) -> Result<Response<proto::RegisterToolResponse>, Status> {
        // PR-6a: a durable write into the off-journal tools.db. The host derives
        // identity + capability server-side (HumanAuthored; net_scope = egress to
        // the SSRF-vetted server_host) — the client supplies NO warrant / tool_id
        // (SN-8). A serve without the registry wired degrades to `unimplemented`.
        // DIALING server_host (the live remote tool round) is PR-6b/Cloud.
        let admin = self.tool_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("RegisterTool: no tool registry wired (tools.db absent)")
        })?;
        let req = request.into_inner();
        // Fail-closed field caps BEFORE the durable write.
        if req.tool_name.trim().is_empty() || req.tool_version.trim().is_empty() {
            return Err(Status::invalid_argument(
                "tool_name and tool_version are required",
            ));
        }
        if req.server_host.trim().is_empty() {
            return Err(Status::invalid_argument(
                "server_host is required (the external MCP endpoint the PR-6b gateway will dial)",
            ));
        }
        if req.description.len() > MAX_TOOL_DESCRIPTION_BYTES {
            return Err(Status::invalid_argument("description too long"));
        }
        if let Some(s) = &req.input_schema {
            if s.params.len() > MAX_TOOL_PARAMS {
                return Err(Status::invalid_argument("too many tool params"));
            }
        }
        let reg = crate::ToolRegistration {
            tool_name: req.tool_name,
            tool_version: req.tool_version,
            description: req.description,
            idempotency_class: req.idempotency_class,
            input_schema: req.input_schema.map(tool_schema_from_proto),
            server_host: req.server_host,
            remote_name: req.remote_name,
        };
        let tool_id = admin.register(reg).map_err(tool_admin_status)?;
        Ok(Response::new(proto::RegisterToolResponse {
            tool_id: tool_id.to_vec(),
            registration_status: "Approved".to_string(),
        }))
    }

    async fn deregister_tool(
        &self,
        request: Request<proto::DeregisterToolRequest>,
    ) -> Result<Response<proto::DeregisterToolResponse>, Status> {
        let admin = self.tool_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("DeregisterTool: no tool registry wired (tools.db absent)")
        })?;
        let req = request.into_inner();
        if req.tool_name.trim().is_empty() || req.tool_version.trim().is_empty() {
            return Err(Status::invalid_argument(
                "tool_name and tool_version are required",
            ));
        }
        let removed = admin.deregister(&req.tool_name, &req.tool_version)?;
        Ok(Response::new(proto::DeregisterToolResponse { removed }))
    }

    async fn discover_tools(
        &self,
        request: Request<proto::DiscoverToolsRequest>,
    ) -> Result<Response<proto::DiscoverToolsResponse>, Status> {
        let admin = self.tool_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("DiscoverTools: no tool registry wired (tools.db absent)")
        })?;
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            100usize
        } else {
            (req.limit as usize).clamp(1, 256)
        };
        let after = if req.after_name.is_empty() {
            None
        } else {
            Some((req.after_name, req.after_version))
        };
        let (rows, has_more) = admin.discover(limit, after)?;
        let tools = rows
            .into_iter()
            .map(|e| proto::RegisteredTool {
                tool_id: e.tool_id.to_vec(),
                tool_name: e.tool_name,
                tool_version: e.tool_version,
                kind: e.kind,
                description: e.description,
                idempotency_class: e.idempotency_class,
                provenance: e.provenance,
                registration_status: e.registration_status,
                server_host: e.server_host,
                net_scope_summary: e.net_scope_summary,
                is_builtin: e.is_builtin,
            })
            .collect();
        Ok(Response::new(proto::DiscoverToolsResponse {
            tools,
            has_more,
        }))
    }

    async fn register_mcp_server(
        &self,
        request: Request<proto::RegisterMcpServerRequest>,
    ) -> Result<Response<proto::RegisterMcpServerResponse>, Status> {
        // PR-6b-1: the live untrusted-egress surface. The host vets the host,
        // DIALS the server (initialize -> tools/list), and registers its tools
        // into the same tools.db (each namespaced `<server>/<remote>`). SN-8: the
        // client supplies NO warrant / tool_id; ids are server-derived.
        let admin = self.mcp_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("RegisterMcpServer: no MCP gateway wired (connections.db absent)")
        })?;
        let req = request.into_inner();
        if req.server_name.trim().is_empty() {
            return Err(Status::invalid_argument("server_name is required"));
        }
        if req.endpoint.trim().is_empty() {
            return Err(Status::invalid_argument("endpoint is required"));
        }
        let credential_ref = if req.credential_ref.trim().is_empty() {
            None
        } else {
            Some(req.credential_ref)
        };
        let reg = crate::McpServerRegistration {
            server_name: req.server_name,
            transport: req.transport,
            endpoint: req.endpoint,
            args: req.args,
            tls_required: req.tls_required,
            credential_ref,
            session_mode: req.session_mode,
        };
        let out = admin.register_server(reg).map_err(mcp_admin_status)?;
        Ok(Response::new(proto::RegisterMcpServerResponse {
            connection_id: out.connection_id.to_vec(),
            discovered: out.discovered,
            health: out.health,
        }))
    }

    async fn list_mcp_servers(
        &self,
        _request: Request<proto::ListMcpServersRequest>,
    ) -> Result<Response<proto::ListMcpServersResponse>, Status> {
        let admin = self.mcp_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("ListMcpServers: no MCP gateway wired (connections.db absent)")
        })?;
        let servers = admin
            .list_servers()
            .map_err(mcp_admin_status)?
            .into_iter()
            .map(|s| proto::McpServer {
                connection_id: s.connection_id.to_vec(),
                server_name: s.server_name,
                transport: s.transport,
                endpoint: s.endpoint,
                health: s.health,
                tool_count: s.tool_count,
                credential_ref_present: s.credential_ref_present,
                session_mode: s.session_mode,
            })
            .collect();
        Ok(Response::new(proto::ListMcpServersResponse {
            servers,
            has_more: false,
        }))
    }

    async fn discover_server_tools(
        &self,
        request: Request<proto::DiscoverServerToolsRequest>,
    ) -> Result<Response<proto::DiscoverServerToolsResponse>, Status> {
        let admin = self.mcp_admin.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "DiscoverServerTools: no MCP gateway wired (connections.db absent)",
            )
        })?;
        let req = request.into_inner();
        if req.server_name.trim().is_empty() {
            return Err(Status::invalid_argument("server_name is required"));
        }
        let (rows, discovered) = admin
            .discover_server(&req.server_name)
            .map_err(mcp_admin_status)?;
        let tools = rows
            .into_iter()
            .map(|e| proto::RegisteredTool {
                tool_id: e.tool_id.to_vec(),
                tool_name: e.tool_name,
                tool_version: e.tool_version,
                kind: e.kind,
                description: e.description,
                idempotency_class: e.idempotency_class,
                provenance: e.provenance,
                registration_status: e.registration_status,
                server_host: e.server_host,
                net_scope_summary: e.net_scope_summary,
                is_builtin: e.is_builtin,
            })
            .collect();
        Ok(Response::new(proto::DiscoverServerToolsResponse {
            tools,
            discovered,
        }))
    }

    async fn test_mcp_server(
        &self,
        request: Request<proto::TestMcpServerRequest>,
    ) -> Result<Response<proto::TestMcpServerResponse>, Status> {
        let admin = self.mcp_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("TestMcpServer: no MCP gateway wired (connections.db absent)")
        })?;
        let req = request.into_inner();
        if req.server_name.trim().is_empty() {
            return Err(Status::invalid_argument("server_name is required"));
        }
        let (reachable, detail) = admin
            .test_server(&req.server_name)
            .map_err(mcp_admin_status)?;
        Ok(Response::new(proto::TestMcpServerResponse {
            reachable,
            detail,
        }))
    }

    async fn deregister_mcp_server(
        &self,
        request: Request<proto::DeregisterMcpServerRequest>,
    ) -> Result<Response<proto::DeregisterMcpServerResponse>, Status> {
        let admin = self.mcp_admin.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "DeregisterMcpServer: no MCP gateway wired (connections.db absent)",
            )
        })?;
        let req = request.into_inner();
        if req.server_name.trim().is_empty() {
            return Err(Status::invalid_argument("server_name is required"));
        }
        let removed = admin
            .deregister_server(&req.server_name)
            .map_err(mcp_admin_status)?;
        Ok(Response::new(proto::DeregisterMcpServerResponse {
            removed,
        }))
    }

    async fn call_mcp_tool(
        &self,
        request: Request<proto::CallMcpToolRequest>,
    ) -> Result<Response<proto::CallMcpToolResponse>, Status> {
        let admin = self.mcp_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("CallMcpTool: no MCP gateway wired (connections.db absent)")
        })?;
        let req = request.into_inner();
        if req.server_name.trim().is_empty() || req.remote_name.trim().is_empty() {
            return Err(Status::invalid_argument(
                "server_name and remote_name are required",
            ));
        }
        // An empty args body is the empty object (a no-arg tool); never null/garbage.
        let args_json = if req.args_json.trim().is_empty() {
            "{}".to_string()
        } else {
            req.args_json
        };
        // A diagnostic fire NEVER 500s on a tool/connector failure — it returns a
        // structured `{ok:false, error}` so the UI/CLI can surface it inline. Only a
        // truly internal fault (no seam) is a gRPC error.
        match admin.call_tool(&req.server_name, &req.remote_name, &args_json) {
            Ok(outcome) => Ok(Response::new(proto::CallMcpToolResponse {
                ok: true,
                result_json: String::from_utf8_lossy(&outcome.result).into_owned(),
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(proto::CallMcpToolResponse {
                ok: false,
                result_json: String::new(),
                error: e.to_string(),
            })),
        }
    }

    // ── MM-3 (D110): the LOCAL OS-keychain secret store. The VALUE is write-only
    // (PutSecret arg); it is never returned, listed, journaled, or in model context.
    async fn put_secret(
        &self,
        request: Request<proto::PutSecretRequest>,
    ) -> Result<Response<proto::PutSecretResponse>, Status> {
        // Authenticated caller required (server-derived identity); never wire-trusted.
        let _party = caller_principal(&request)?;
        // Loopback-only gate: secret writes plant host credential material, so they
        // are refused unless the gateway is loopback-bound (no remote peer can reach it).
        if !self.secret_writes_loopback_ok {
            return Err(Status::permission_denied(
                "PutSecret requires a loopback-bound gateway (set secrets locally, or via the environment)",
            ));
        }
        let admin = self
            .secret_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("PutSecret: no secret store wired"))?;
        let req = request.into_inner();
        if !valid_secret_name(&req.name) {
            return Err(Status::invalid_argument(
                "name must be 1..=255 chars of [A-Za-z0-9_.-]",
            ));
        }
        if req.value.is_empty() {
            return Err(Status::invalid_argument("value is required"));
        }
        admin
            .put(&req.name, &req.value)
            .map_err(secret_admin_status)?;
        // The request (and its `value`) is dropped here; nothing echoes it.
        Ok(Response::new(proto::PutSecretResponse { stored: true }))
    }

    async fn list_secret_names(
        &self,
        request: Request<proto::ListSecretNamesRequest>,
    ) -> Result<Response<proto::ListSecretNamesResponse>, Status> {
        // A read (NAMES only) needs only an authenticated caller — no loopback gate.
        let _party = caller_principal(&request)?;
        let admin = self
            .secret_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListSecretNames: no secret store wired"))?;
        let req = request.into_inner();
        let (rows, has_more) = admin
            .list_names(req.limit, &req.after_name)
            .map_err(secret_admin_status)?;
        Ok(Response::new(proto::ListSecretNamesResponse {
            names: rows
                .into_iter()
                .map(|r| proto::SecretName {
                    name: r.name,
                    created_unix_ms: r.created_unix_ms,
                    updated_unix_ms: r.updated_unix_ms,
                })
                .collect(),
            has_more,
        }))
    }

    async fn delete_secret(
        &self,
        request: Request<proto::DeleteSecretRequest>,
    ) -> Result<Response<proto::DeleteSecretResponse>, Status> {
        let _party = caller_principal(&request)?;
        if !self.secret_writes_loopback_ok {
            return Err(Status::permission_denied(
                "DeleteSecret requires a loopback-bound gateway",
            ));
        }
        let admin = self
            .secret_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("DeleteSecret: no secret store wired"))?;
        let req = request.into_inner();
        if !valid_secret_name(&req.name) {
            return Err(Status::invalid_argument(
                "name must be 1..=255 chars of [A-Za-z0-9_.-]",
            ));
        }
        let removed = admin.delete(&req.name).map_err(secret_admin_status)?;
        Ok(Response::new(proto::DeleteSecretResponse { removed }))
    }

    // ── D113 (trigger seam): event ingress. Each inbound event starts a fresh run
    // via the SAME Invoke propose-proxy the host trigger admin owns (coordinator stays
    // the sole journal writer; frozen trio untouched). SN-8: server-derived id + owner.
    async fn register_trigger(
        &self,
        request: Request<proto::RegisterTriggerRequest>,
    ) -> Result<Response<proto::RegisterTriggerResponse>, Status> {
        // The trigger fires under the REGISTRANT's party (D102.2; server-derived).
        let owner_party = caller_principal(&request)?;
        let admin = self
            .trigger_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("RegisterTrigger: no trigger admin wired"))?;
        let req = request.into_inner();
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }
        // T-APP-TRIGGER-TARGET: recipe_handle | app_handle exactly-one-of is validated in
        // the seam's register() (which also fail-fasts an App target when the App-run seam
        // is absent), so no unconditional recipe_handle check here.
        let kind = trigger_kind_str(req.kind);
        if kind.is_empty() {
            return Err(Status::invalid_argument(
                "kind must be WEBHOOK, CRON, or GRPC",
            ));
        }
        let auth = trigger_auth_str(req.auth);
        if auth.is_empty() {
            return Err(Status::invalid_argument(
                "auth must be NONE, HMAC_SHA256, or BEARER",
            ));
        }
        let trigger_id = admin
            .register(crate::TriggerRegistration {
                name: req.name,
                kind: kind.to_string(),
                recipe_handle: req.recipe_handle,
                app_handle: req.app_handle,
                auth: auth.to_string(),
                auth_secret_ref: req.auth_secret_ref,
                schedule_spec: req.schedule_spec,
                timezone: req.timezone,
                enabled: req.enabled,
                require_approval: req.require_approval,
                owner_party,
            })
            .await
            .map_err(trigger_admin_status)?;
        Ok(Response::new(proto::RegisterTriggerResponse {
            trigger_id: trigger_id.to_vec(),
        }))
    }

    async fn list_triggers(
        &self,
        request: Request<proto::ListTriggersRequest>,
    ) -> Result<Response<proto::ListTriggersResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self
            .trigger_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListTriggers: no trigger admin wired"))?;
        let req = request.into_inner();
        let (rows, has_more) = admin
            .list(req.limit, &req.after_name)
            .await
            .map_err(trigger_admin_status)?;
        Ok(Response::new(proto::ListTriggersResponse {
            triggers: rows
                .into_iter()
                .map(|t| proto::TriggerView {
                    trigger_id: t.trigger_id.to_vec(),
                    name: t.name,
                    kind: trigger_kind_proto(&t.kind),
                    recipe_handle: t.recipe_handle,
                    app_handle: t.app_handle,
                    auth: trigger_auth_proto(&t.auth),
                    auth_secret_present: t.auth_secret_present,
                    schedule_spec: t.schedule_spec,
                    timezone: t.timezone,
                    enabled: t.enabled,
                    require_approval: t.require_approval,
                    last_fire_unix_ms: t.last_fire_unix_ms,
                })
                .collect(),
            has_more,
        }))
    }

    async fn deregister_trigger(
        &self,
        request: Request<proto::DeregisterTriggerRequest>,
    ) -> Result<Response<proto::DeregisterTriggerResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self
            .trigger_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("DeregisterTrigger: no trigger admin wired"))?;
        let req = request.into_inner();
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }
        let removed = admin
            .deregister(&req.name)
            .await
            .map_err(trigger_admin_status)?;
        Ok(Response::new(proto::DeregisterTriggerResponse { removed }))
    }

    async fn submit_trigger(
        &self,
        request: Request<proto::SubmitTriggerRequest>,
    ) -> Result<Response<proto::SubmitTriggerResponse>, Status> {
        // An authenticated caller may fire a registered trigger; the run still binds
        // under the trigger's OWN owner party (the caller cannot escalate via the trigger).
        let _party = caller_principal(&request)?;
        let admin = self
            .trigger_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("SubmitTrigger: no trigger admin wired"))?;
        let req = request.into_inner();
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }
        let outcome = admin
            .submit(&req.name, &req.idempotency_key, &req.payload_json)
            .await
            .map_err(trigger_admin_status)?;
        Ok(Response::new(proto::SubmitTriggerResponse {
            instance_id: outcome.instance_id.to_vec(),
            deduped: outcome.deduped,
        }))
    }

    async fn test_trigger(
        &self,
        request: Request<proto::TestTriggerRequest>,
    ) -> Result<Response<proto::TestTriggerResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self
            .trigger_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("TestTrigger: no trigger admin wired"))?;
        let req = request.into_inner();
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }
        let (ok, detail) = admin
            .test(&req.name, &req.payload_json)
            .await
            .map_err(trigger_admin_status)?;
        Ok(Response::new(proto::TestTriggerResponse { ok, detail }))
    }

    // ----- D114 (HITL approval) + M11 (cost readout) -----

    async fn list_pending_approvals(
        &self,
        request: Request<proto::ListPendingApprovalsRequest>,
    ) -> Result<Response<proto::ListPendingApprovalsResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self.approval_admin.as_ref().ok_or_else(|| {
            Status::unimplemented("ListPendingApprovals: no approval admin wired")
        })?;
        let rows = admin
            .list_pending(request.into_inner().limit)
            .await
            .map_err(approval_admin_status)?;
        let approvals = rows
            .into_iter()
            .map(|r| proto::PendingApproval {
                request_id: r.request_id.to_vec(),
                instance_id: r.instance_id.to_vec(),
                mote_id: r.mote_id.to_vec(),
                tool_id: r.tool_id,
                tool_version: r.tool_version,
                intent: r.intent,
                deadline_unix_ms: r.deadline_unix_ms,
                created_unix_ms: r.created_unix_ms,
            })
            .collect();
        Ok(Response::new(proto::ListPendingApprovalsResponse {
            approvals,
        }))
    }

    async fn grant_approval(
        &self,
        request: Request<proto::GrantApprovalRequest>,
    ) -> Result<Response<proto::GrantApprovalResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self
            .approval_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GrantApproval: no approval admin wired"))?;
        let req = request.into_inner();
        let request_id = approval_request_id_arg(&req.request_id)?;
        let granted = admin
            .grant(request_id, &req.reason)
            .await
            .map_err(approval_admin_status)?;
        Ok(Response::new(proto::GrantApprovalResponse { granted }))
    }

    async fn deny_approval(
        &self,
        request: Request<proto::DenyApprovalRequest>,
    ) -> Result<Response<proto::DenyApprovalResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self
            .approval_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("DenyApproval: no approval admin wired"))?;
        let req = request.into_inner();
        let request_id = approval_request_id_arg(&req.request_id)?;
        let denied = admin
            .deny(request_id, &req.reason)
            .await
            .map_err(approval_admin_status)?;
        Ok(Response::new(proto::DenyApprovalResponse { denied }))
    }

    async fn get_run_cost(
        &self,
        request: Request<proto::GetRunCostRequest>,
    ) -> Result<Response<proto::GetRunCostResponse>, Status> {
        let _party = caller_principal(&request)?;
        let admin = self
            .approval_admin
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GetRunCost: no approval admin wired"))?;
        let req = request.into_inner();
        let instance_id: [u8; 16] = req
            .instance_id
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("instance_id must be 16 bytes"))?;
        let c = admin
            .run_cost(instance_id)
            .await
            .map_err(approval_admin_status)?;
        Ok(Response::new(proto::GetRunCostResponse {
            instance_id: c.instance_id.to_vec(),
            turns: c.turns,
            tool_calls: c.tool_calls,
            estimated_micro_usd: c.estimated_micro_usd,
            ceiling_micro_usd: c.ceiling_micro_usd,
            per_turn_micro_usd: c.per_turn_micro_usd,
            per_tool_call_micro_usd: c.per_tool_call_micro_usd,
            over_ceiling: c.over_ceiling,
        }))
    }

    async fn put_context_bundle(
        &self,
        request: Request<proto::PutContextBundleRequest>,
    ) -> Result<Response<proto::PutContextBundleResponse>, Status> {
        let bundles = self.bundles.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "PutContextBundle: no context-bundle store wired (bundles.db absent)",
            )
        })?;
        // SERVER-DERIVED identity (SN-8): bundles are scoped to the auth-resolved party.
        let principal = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.handle) {
            return Err(Status::invalid_argument(
                "handle must be a 'namespace/collection/name' AssetPath ([a-z0-9._-] segments)",
            ));
        }
        if req.description.len() > crate::MAX_BUNDLE_DESCRIPTION_BYTES {
            return Err(Status::invalid_argument(
                "description exceeds the server cap",
            ));
        }
        if req.items.is_empty() {
            return Err(Status::invalid_argument(
                "a context bundle needs at least one item",
            ));
        }
        if req.items.len() > crate::MAX_CONTEXT_BUNDLE_ITEMS {
            return Err(Status::invalid_argument(
                "a context bundle accepts at most 256 items",
            ));
        }
        let mut items = Vec::with_capacity(req.items.len());
        for it in req.items {
            let content_ref = hash_32(&it.content_ref, "item content_ref must be 32 bytes")?;
            items.push(crate::BundleItemRecord {
                name: it.name,
                content_ref,
                media_type: it.media_type,
            });
        }
        let (bundle_ref, deduplicated) =
            bundles.upsert(&principal, &req.handle, &req.description, &items)?;
        Ok(Response::new(proto::PutContextBundleResponse {
            bundle_ref: bundle_ref.to_vec(),
            handle: req.handle,
            deduplicated,
        }))
    }

    async fn list_context_bundles(
        &self,
        request: Request<proto::ListContextBundlesRequest>,
    ) -> Result<Response<proto::ListContextBundlesResponse>, Status> {
        let bundles = self.bundles.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "ListContextBundles: no context-bundle store wired (bundles.db absent)",
            )
        })?;
        let principal = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            100
        } else {
            (req.limit as usize).min(256)
        };
        let after = if req.after_handle.is_empty() {
            None
        } else {
            Some(req.after_handle.as_str())
        };
        let (manifests, has_more) = bundles.list(&principal, limit, after)?;
        Ok(Response::new(proto::ListContextBundlesResponse {
            bundles: manifests.into_iter().map(manifest_to_proto).collect(),
            has_more,
        }))
    }

    async fn get_context_bundle(
        &self,
        request: Request<proto::GetContextBundleRequest>,
    ) -> Result<Response<proto::GetContextBundleResponse>, Status> {
        let bundles = self.bundles.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "GetContextBundle: no context-bundle store wired (bundles.db absent)",
            )
        })?;
        let principal = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();
        // Uniform not-found for absent OR not-owned (no cross-party existence oracle).
        match bundles.get(&principal, &req.handle)? {
            Some(m) => Ok(Response::new(proto::GetContextBundleResponse {
                bundle: Some(manifest_to_proto(m)),
                found: true,
            })),
            None => Ok(Response::new(proto::GetContextBundleResponse {
                bundle: None,
                found: false,
            })),
        }
    }

    async fn delete_context_bundle(
        &self,
        request: Request<proto::DeleteContextBundleRequest>,
    ) -> Result<Response<proto::DeleteContextBundleResponse>, Status> {
        let bundles = self.bundles.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "DeleteContextBundle: no context-bundle store wired (bundles.db absent)",
            )
        })?;
        let principal = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();
        let removed = bundles.delete(&principal, &req.handle)?;
        Ok(Response::new(proto::DeleteContextBundleResponse {
            removed,
        }))
    }

    // ----- D155 Phase-A — branched data (read / snapshot) -----

    async fn create_branch(
        &self,
        request: Request<proto::CreateBranchRequest>,
    ) -> Result<Response<proto::CreateBranchResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("CreateBranch: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.handle) {
            return Err(Status::invalid_argument(
                "handle must be a 'namespace/collection/name' AssetPath ([a-z0-9._-] segments)",
            ));
        }
        if req.description.len() > crate::MAX_BRANCH_DESCRIPTION_BYTES {
            return Err(Status::invalid_argument(
                "description exceeds the server cap",
            ));
        }
        let parent = optional_handle(&req.parent_handle)?;
        let (manifest, deduplicated) =
            branches.create(&principal, &req.handle, parent, &req.description)?;
        Ok(Response::new(proto::CreateBranchResponse {
            branch_ref: manifest.branch_ref.to_vec(),
            handle: req.handle,
            deduplicated,
        }))
    }

    async fn snapshot_into(
        &self,
        request: Request<proto::SnapshotIntoRequest>,
    ) -> Result<Response<proto::SnapshotIntoResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("SnapshotInto: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.handle) {
            return Err(Status::invalid_argument(
                "handle must be a 'namespace/collection/name' AssetPath ([a-z0-9._-] segments)",
            ));
        }
        if req.description.len() > crate::MAX_BRANCH_DESCRIPTION_BYTES {
            return Err(Status::invalid_argument(
                "description exceeds the server cap",
            ));
        }
        if req.paths.is_empty() {
            return Err(Status::invalid_argument(
                "snapshot needs at least one path to read",
            ));
        }
        if req.paths.len() > crate::MAX_SNAPSHOT_PATHS {
            return Err(Status::invalid_argument(
                "snapshot accepts at most 256 paths per call",
            ));
        }
        if req.paths.iter().any(|p| p.trim().is_empty()) {
            return Err(Status::invalid_argument("a snapshot path may not be empty"));
        }
        let parent = optional_handle(&req.parent_handle)?;
        let (manifest, ingested, deduplicated) = branches.snapshot_into(
            &principal,
            &req.handle,
            parent,
            &req.description,
            &req.paths,
        )?;
        let proto_branch = branch_to_proto(manifest);
        Ok(Response::new(proto::SnapshotIntoResponse {
            branch_ref: proto_branch.branch_ref,
            handle: req.handle,
            items: proto_branch.items,
            ingested: u32::try_from(ingested).unwrap_or(u32::MAX),
            deduplicated,
        }))
    }

    async fn list_branches(
        &self,
        request: Request<proto::ListBranchesRequest>,
    ) -> Result<Response<proto::ListBranchesResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("ListBranches: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            100
        } else {
            (req.limit as usize).min(256)
        };
        let after = if req.after_handle.is_empty() {
            None
        } else {
            Some(req.after_handle.as_str())
        };
        let (manifests, has_more) = branches.list(&principal, limit, after)?;
        Ok(Response::new(proto::ListBranchesResponse {
            branches: manifests.into_iter().map(branch_to_proto).collect(),
            has_more,
        }))
    }

    async fn get_branch(
        &self,
        request: Request<proto::GetBranchRequest>,
    ) -> Result<Response<proto::GetBranchResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("GetBranch: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        // Uniform not-found for absent OR not-owned (no cross-party existence oracle).
        match branches.get(&principal, &req.handle)? {
            Some(m) => Ok(Response::new(proto::GetBranchResponse {
                branch: Some(branch_to_proto(m)),
                found: true,
            })),
            None => Ok(Response::new(proto::GetBranchResponse {
                branch: None,
                found: false,
            })),
        }
    }

    async fn delete_branch(
        &self,
        request: Request<proto::DeleteBranchRequest>,
    ) -> Result<Response<proto::DeleteBranchResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("DeleteBranch: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let removed = branches.delete(&principal, &req.handle)?;
        Ok(Response::new(proto::DeleteBranchResponse { removed }))
    }

    async fn advance_branch(
        &self,
        request: Request<proto::AdvanceBranchRequest>,
    ) -> Result<Response<proto::AdvanceBranchResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("AdvanceBranch: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.handle) {
            return Err(Status::invalid_argument(
                "handle must be a 'namespace/collection/name' AssetPath ([a-z0-9._-] segments)",
            ));
        }
        if req.path.trim().is_empty() {
            return Err(Status::invalid_argument(
                "advance requires a non-empty path",
            ));
        }
        // POC-5b — the per-App lock write chokepoint: a locked branch refuses every
        // agentic in-CAS edit (the agent-write authority gate). A real lock-store
        // error fails closed; an absent lock seam degrades open (additive feature).
        if let Some(locks) = self.locks.as_ref() {
            if locks.is_locked(&principal, &req.handle)? {
                return Err(with_refusal_code(
                    Status::failed_precondition(
                        "branch is locked; agentic in-CAS edits are refused (unlock the App to edit)",
                    ),
                    crate::locks_view::LOCKED_BRANCH_REFUSAL_CODE,
                ));
            }
        }
        // The edited body the ReAct loop committed; an unresolvable ref is rejected
        // in the store (fail-closed) — here we only enforce the 32-byte shape.
        let content_ref: [u8; 32] = req
            .content_ref
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("content_ref must be exactly 32 bytes"))?;
        let (manifest, deduplicated) =
            branches.advance(&principal, &req.handle, &req.path, content_ref)?;
        let proto_branch = branch_to_proto(manifest);
        Ok(Response::new(proto::AdvanceBranchResponse {
            branch_ref: proto_branch.branch_ref,
            handle: req.handle,
            items: proto_branch.items,
            deduplicated,
        }))
    }

    async fn get_branch_content(
        &self,
        request: Request<proto::GetBranchContentRequest>,
    ) -> Result<Response<proto::GetBranchContentResponse>, Status> {
        let branches = self.branches.as_ref().ok_or_else(|| {
            Status::unimplemented("GetBranchContent: no branch store wired (branches.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        // Caller-scoped: resolve the body THROUGH the caller's OWN branch manifest.
        // Uniform `found=false` for absent branch / absent path / unresolvable ref —
        // no cross-party existence oracle (the GetBranch / GetApp posture).
        let payload = branches.get(&principal, &req.handle)?.and_then(|manifest| {
            let item = manifest.items.iter().find(|it| it.path == req.path)?;
            self.content
                .get(&kx_content::ContentRef::from_bytes(item.content_ref))
        });
        Ok(Response::new(match payload {
            Some(payload) => proto::GetBranchContentResponse {
                payload,
                found: true,
            },
            None => proto::GetBranchContentResponse {
                payload: Vec::new(),
                found: false,
            },
        }))
    }

    async fn lock_app(
        &self,
        request: Request<proto::LockAppRequest>,
    ) -> Result<Response<proto::LockAppResponse>, Status> {
        let locks = self.locks.as_ref().ok_or_else(|| {
            Status::unimplemented("LockApp: no lock store wired (locks.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.branch_handle) {
            return Err(Status::invalid_argument(
                "branch_handle must be a 'namespace/collection/name' AssetPath",
            ));
        }
        let locked = locks.lock(&principal, &req.branch_handle)?;
        Ok(Response::new(proto::LockAppResponse { locked }))
    }

    async fn unlock_app(
        &self,
        request: Request<proto::UnlockAppRequest>,
    ) -> Result<Response<proto::UnlockAppResponse>, Status> {
        let locks = self.locks.as_ref().ok_or_else(|| {
            Status::unimplemented("UnlockApp: no lock store wired (locks.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        if !valid_bundle_handle(&req.branch_handle) {
            return Err(Status::invalid_argument(
                "branch_handle must be a 'namespace/collection/name' AssetPath",
            ));
        }
        let unlocked = locks.unlock(&principal, &req.branch_handle)?;
        Ok(Response::new(proto::UnlockAppResponse { unlocked }))
    }

    async fn scaffold_app(
        &self,
        request: Request<proto::ScaffoldAppRequest>,
    ) -> Result<Response<proto::ScaffoldAppResponse>, Status> {
        let scaffolder = self.scaffolder.as_ref().ok_or_else(|| {
            Status::unimplemented(
                "ScaffoldApp: this serve has no scaffold orchestrator (no served model / branch store)",
            )
        })?;
        let apps = self.apps.as_ref().ok_or_else(|| {
            Status::unimplemented("ScaffoldApp: no app catalog wired (apps.db absent)")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        // The App must exist + be caller-owned (uniform not-found — no oracle).
        let Some((record, _envelope)) = apps.get(&principal, &req.handle)? else {
            return Err(Status::not_found("app not found"));
        };
        // One-App-one-branch: the project branch defaults to the App's own handle.
        let branch_handle = if req.branch_handle.trim().is_empty() {
            req.handle.clone()
        } else {
            req.branch_handle.clone()
        };
        if !valid_bundle_handle(&branch_handle) {
            return Err(Status::invalid_argument(
                "branch_handle must be a 'namespace/collection/name' AssetPath",
            ));
        }
        // POC-5b: a locked branch refuses the scaffold (the agent-write gate).
        if let Some(locks) = self.locks.as_ref() {
            if locks.is_locked(&principal, &branch_handle)? {
                return Err(with_refusal_code(
                    Status::failed_precondition("branch is locked; scaffold refused"),
                    crate::locks_view::LOCKED_BRANCH_REFUSAL_CODE,
                ));
            }
        }
        // The authoring goal: the instruction, else the App's name.
        let goal = if req.instruction.trim().is_empty() {
            record.name.clone()
        } else {
            req.instruction.clone()
        };
        // The host driver creates/resumes the branch + spawns the background loop and
        // returns immediately (progress via GetScaffoldStatus + GetBranch).
        let resumed = scaffolder.start(&principal, &branch_handle, &goal)?;
        Ok(Response::new(proto::ScaffoldAppResponse {
            // Multi-run by design — correlate by `branch_handle` (poll GetScaffoldStatus
            // + GetBranch). Left empty rather than asserting a single run id (GR15).
            instance_id: Vec::new(),
            branch_handle,
            resumed,
        }))
    }

    async fn get_scaffold_status(
        &self,
        request: Request<proto::GetScaffoldStatusRequest>,
    ) -> Result<Response<proto::GetScaffoldStatusResponse>, Status> {
        let scaffolder = self.scaffolder.as_ref().ok_or_else(|| {
            Status::unimplemented("GetScaffoldStatus: no scaffold orchestrator wired")
        })?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let status = scaffolder.status(&principal, &req.branch_handle)?;
        Ok(Response::new(proto::GetScaffoldStatusResponse {
            phase: scaffold_phase_to_proto(status.phase) as i32,
            files_done: status.files_done,
            files_pending: status.files_pending,
            detail: status.detail,
        }))
    }

    async fn list_tool_manifests(
        &self,
        _request: Request<proto::ListToolManifestsRequest>,
    ) -> Result<Response<proto::ListToolManifestsResponse>, Status> {
        let view = self
            .toolscout
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListToolManifests: no toolscout view wired"))?;
        let manifests = view
            .list_manifests()
            .into_iter()
            .map(crate::toolscout_view::tool_manifest_to_proto)
            .collect();
        Ok(Response::new(proto::ListToolManifestsResponse {
            manifests,
        }))
    }

    async fn score_task_bundle(
        &self,
        request: Request<proto::ScoreTaskBundleRequest>,
    ) -> Result<Response<proto::ScoreTaskBundleResponse>, Status> {
        let view = self
            .toolscout
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ScoreTaskBundle: no toolscout view wired"))?;
        // Fail-closed caps BEFORE the seam — the host never sees an unbounded,
        // empty, or duplicate-bearing spec (`invalid_argument` on violation).
        let spec = crate::toolscout_view::validate_bundle_spec(&request.into_inner())
            .map_err(Status::invalid_argument)?;
        // ADVISORY end to end (SN-8): the view ranks + dry-runs the real
        // lowering gate; no journal write, no digest change, no authorization.
        let score = view.score_bundle(&spec);
        Ok(Response::new(crate::toolscout_view::bundle_score_to_proto(
            score,
        )))
    }

    async fn list_recipes(
        &self,
        _request: Request<proto::ListRecipesRequest>,
    ) -> Result<Response<proto::ListRecipesResponse>, Status> {
        let catalog = self
            .catalog_recipes
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListRecipes: no recipe catalog wired"))?;
        let recipes = catalog
            .list_recipes()
            .into_iter()
            .map(|handle| recipe_summary_to_proto(catalog.as_ref(), handle))
            .collect();
        Ok(Response::new(proto::ListRecipesResponse { recipes }))
    }

    async fn search_recipes(
        &self,
        request: Request<proto::SearchRecipesRequest>,
    ) -> Result<Response<proto::SearchRecipesResponse>, Status> {
        let catalog = self
            .catalog_recipes
            .as_ref()
            .ok_or_else(|| Status::unimplemented("SearchRecipes: no recipe catalog wired"))?;
        let req = request.into_inner();
        // The seam clamps the limit to its own cap; a None/0 request limit means
        // "the server default" (the seam owns the policy). Capped here too so a
        // huge value can never widen the host's own bound.
        let limit = req
            .limit
            .map_or(SEARCH_RECIPES_DEFAULT_LIMIT, |l| l as usize)
            .clamp(1, SEARCH_RECIPES_MAX_LIMIT);
        // `None` ⇒ the host did not provision discovery ⇒ honest unimplemented
        // (mirrors the catalog-not-wired arm). An empty Vec is a valid "no match".
        let ranked = catalog
            .search_recipes(&req.intent, &req.keywords, limit)
            .ok_or_else(|| Status::unimplemented("SearchRecipes: discovery not provisioned"))?
            .into_iter()
            .map(|e| proto::ScoredRecipe {
                recipe: Some(proto::RecipeSummary {
                    handle: e.handle,
                    recipe_fingerprint: Vec::new(),
                    description: e.metadata.description,
                    tags: e.metadata.tags,
                    version: e.metadata.version,
                }),
                score_bp: e.score_bp,
            })
            .collect();
        Ok(Response::new(proto::SearchRecipesResponse { ranked }))
    }

    async fn get_recipe_form(
        &self,
        request: Request<proto::GetRecipeFormRequest>,
    ) -> Result<Response<proto::GetRecipeFormResponse>, Status> {
        let catalog = self
            .catalog_recipes
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GetRecipeForm: no recipe catalog wired"))?;
        let handle = request.into_inner().handle;
        // A public discovery surface: `not_found` for an unknown handle is intended
        // (the catalog is authoritative for WHAT recipes exist), NOT collapsed like
        // the Invoke execution surface — Invoke remains the authorization gate.
        let fields = catalog
            .get_recipe_form(&handle)
            .ok_or_else(|| Status::not_found("recipe not found"))?;
        let fields = fields.into_iter().map(form_field_to_proto).collect();
        Ok(Response::new(proto::GetRecipeFormResponse {
            handle,
            fields,
        }))
    }

    async fn list_teams(
        &self,
        _request: Request<proto::ListTeamsRequest>,
    ) -> Result<Response<proto::ListTeamsResponse>, Status> {
        let view = self
            .membership
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListTeams: no membership view wired"))?;
        let teams = view
            .list_teams()
            .into_iter()
            .map(team_summary_to_proto)
            .collect();
        Ok(Response::new(proto::ListTeamsResponse { teams }))
    }

    async fn list_team_members(
        &self,
        request: Request<proto::ListTeamMembersRequest>,
    ) -> Result<Response<proto::ListTeamMembersResponse>, Status> {
        let view = self
            .membership
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListTeamMembers: no membership view wired"))?;
        let req = request.into_inner();
        // A public viewer surface: `not_found` for an unknown team is intended (not
        // collapsed like the Invoke execution surface — these RPCs are view-only).
        let members = view
            .list_members(&req.team_id, req.asset_ref.as_deref())
            .ok_or_else(|| Status::not_found("team not found"))?;
        Ok(Response::new(proto::ListTeamMembersResponse {
            owner: members.owner,
            members: members
                .members
                .into_iter()
                .map(team_member_to_proto)
                .collect(),
        }))
    }

    async fn list_asset_grants(
        &self,
        request: Request<proto::ListAssetGrantsRequest>,
    ) -> Result<Response<proto::ListAssetGrantsResponse>, Status> {
        let view = self
            .grants_view
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListAssetGrants: no grant view wired"))?;
        let asset_ref = request.into_inner().asset_ref;
        let grants = view
            .list_asset_grants(&asset_ref)
            .ok_or_else(|| Status::not_found("asset not found"))?;
        Ok(Response::new(proto::ListAssetGrantsResponse {
            owner: grants.owner,
            grants: grants
                .grants
                .into_iter()
                .map(grant_entry_to_proto)
                .collect(),
        }))
    }

    async fn list_datasets(
        &self,
        _request: Request<proto::ListDatasetsRequest>,
    ) -> Result<Response<proto::ListDatasetsResponse>, Status> {
        let view = self
            .datasets
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListDatasets: no dataset view wired"))?;
        let datasets = view
            .list_datasets()
            .into_iter()
            .map(crate::datasets::dataset_summary_to_proto)
            .collect();
        Ok(Response::new(proto::ListDatasetsResponse { datasets }))
    }

    async fn ingest_documents(
        &self,
        request: Request<proto::IngestDocumentsRequest>,
    ) -> Result<Response<proto::IngestDocumentsResponse>, Status> {
        let view = self
            .datasets
            .as_ref()
            .ok_or_else(|| Status::unimplemented("IngestDocuments: no dataset view wired"))?;
        let req = request.into_inner();
        // Borrow content + the optional client vector from the request (no copy
        // before the host dedups). An empty `embedding` ⇒ ask the host to embed.
        let docs: Vec<crate::datasets::IngestDoc<'_>> = req
            .documents
            .iter()
            .map(|d| crate::datasets::IngestDoc {
                content: &d.content,
                embedding: (!d.embedding.is_empty()).then_some(d.embedding.as_slice()),
            })
            .collect();
        let out = view
            .ingest(&req.dataset, &docs)
            .map_err(crate::datasets::dataset_status)?;
        Ok(Response::new(proto::IngestDocumentsResponse {
            dataset_id: out.dataset_id,
            doc_count: out.doc_count,
            inserted: out.inserted,
            dim: out.dim,
        }))
    }

    async fn query_dataset(
        &self,
        request: Request<proto::QueryDatasetRequest>,
    ) -> Result<Response<proto::QueryDatasetResponse>, Status> {
        let view = self
            .datasets
            .as_ref()
            .ok_or_else(|| Status::unimplemented("QueryDataset: no dataset view wired"))?;
        let req = request.into_inner();
        // A non-empty client vector takes precedence (the FFI-free path); an empty
        // one falls back to embedding `query_text` (needs an embedder).
        let qe = (!req.query_embedding.is_empty()).then_some(req.query_embedding.as_slice());
        let mode = crate::datasets::retrieval_mode_from_proto(req.retrieval_mode);
        let hits = view
            .query(
                &req.dataset,
                qe,
                &req.query_text,
                req.k as usize,
                mode,
                req.rerank,
            )
            .map_err(crate::datasets::dataset_status)?;
        let hits = hits
            .into_iter()
            .map(crate::datasets::dataset_hit_to_proto)
            .collect();
        Ok(Response::new(proto::QueryDatasetResponse { hits }))
    }

    // ---- RC5a: durable multi-tier MEMORY (semantic recall + episodic store) ------

    async fn store_memory(
        &self,
        request: Request<proto::StoreMemoryRequest>,
    ) -> Result<Response<proto::StoreMemoryResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("StoreMemory: no memory view wired"))?;
        // The namespace is DERIVED from the server-resolved principal — never trusted
        // from the wire (verdict #5: no cross-principal write).
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        let embedding = (!req.embedding.is_empty()).then_some(req.embedding.as_slice());
        let out = view
            .store(crate::memory::MemoryWrite {
                namespace: &ns,
                content: &req.content,
                embedding,
                kind: crate::memory::memory_kind_from_proto(req.kind),
                // An operator/SDK write is not part of a run (all-zero instance_id).
                instance_id: [0u8; 16],
            })
            .map_err(crate::memory::memory_status)?;
        Ok(Response::new(proto::StoreMemoryResponse {
            memory_id: out.memory_id.to_vec(),
            inserted: out.inserted,
            dim: out.dim,
        }))
    }

    async fn list_memories(
        &self,
        request: Request<proto::ListMemoriesRequest>,
    ) -> Result<Response<proto::ListMemoriesResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListMemories: no memory view wired"))?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        let limit = req.limit.map_or(200, |l| l.clamp(1, 500)) as usize;
        let instance_filter = req
            .instance_id
            .as_deref()
            .and_then(|b| <[u8; 16]>::try_from(b).ok());
        // Fetch one extra to compute `has_more` without a second query.
        let mut rows = view
            .list(&ns, instance_filter, limit + 1, req.include_tombstoned)
            .map_err(crate::memory::memory_status)?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let memories = rows
            .into_iter()
            .map(crate::memory::memory_summary_to_proto)
            .collect();
        Ok(Response::new(proto::ListMemoriesResponse {
            memories,
            has_more,
        }))
    }

    async fn recall_memory(
        &self,
        request: Request<proto::RecallMemoryRequest>,
    ) -> Result<Response<proto::RecallMemoryResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("RecallMemory: no memory view wired"))?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        // A non-empty client vector takes precedence (FFI-free); else embed query_text.
        let qe = (!req.query_embedding.is_empty()).then_some(req.query_embedding.as_slice());
        let k = if req.k == 0 {
            5
        } else {
            (req.k as usize).min(64)
        };
        let hits = view
            .recall(&ns, qe, &req.query_text, k)
            .map_err(crate::memory::memory_status)?;
        let hits = hits
            .into_iter()
            .map(crate::memory::memory_hit_to_proto)
            .collect();
        Ok(Response::new(proto::RecallMemoryResponse { hits }))
    }

    async fn forget_memory(
        &self,
        request: Request<proto::ForgetMemoryRequest>,
    ) -> Result<Response<proto::ForgetMemoryResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ForgetMemory: no memory view wired"))?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        let id = <[u8; 32]>::try_from(req.memory_id.as_slice())
            .map_err(|_| Status::invalid_argument("memory_id must be 32 bytes"))?;
        let forgotten = view
            .forget(&ns, &id)
            .map_err(crate::memory::memory_status)?;
        Ok(Response::new(proto::ForgetMemoryResponse { forgotten }))
    }

    async fn decay_memory(
        &self,
        request: Request<proto::DecayMemoryRequest>,
    ) -> Result<Response<proto::DecayMemoryResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("DecayMemory: no memory view wired"))?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        // Server-side clamps: a 0 TTL falls back to 90 days; TTL is bounded; salience
        // floor is bounded. A decay is authored by the caller over their OWN namespace.
        let ttl_days = if req.ttl_days == 0 {
            90
        } else {
            req.ttl_days.min(3650)
        };
        let ttl_ms = i64::from(ttl_days) * 86_400_000;
        let min_access = req.min_access.min(1_000_000);
        let report = view
            .decay(&ns, ttl_ms, min_access, req.dry_run)
            .map_err(crate::memory::memory_status)?;
        let now_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(i64::MAX);
        Ok(Response::new(crate::memory::decay_report_to_proto(
            report, now_ms,
        )))
    }

    async fn memory_stats(
        &self,
        request: Request<proto::MemoryStatsRequest>,
    ) -> Result<Response<proto::MemoryStatsResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("MemoryStats: no memory view wired"))?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        let stats = view.stats(&ns).map_err(crate::memory::memory_status)?;
        Ok(Response::new(crate::memory::memory_stats_to_proto(
            stats, ns,
        )))
    }

    async fn restore_memory(
        &self,
        request: Request<proto::RestoreMemoryRequest>,
    ) -> Result<Response<proto::RestoreMemoryResponse>, Status> {
        let view = self
            .memory
            .as_ref()
            .ok_or_else(|| Status::unimplemented("RestoreMemory: no memory view wired"))?;
        let principal = caller_principal(&request)?;
        let req = request.into_inner();
        let ns = crate::memory::memory_namespace(&principal, &req.namespace);
        let id = <[u8; 32]>::try_from(req.memory_id.as_slice())
            .map_err(|_| Status::invalid_argument("memory_id must be 32 bytes"))?;
        let restored = view
            .restore(&ns, &id)
            .map_err(crate::memory::memory_status)?;
        Ok(Response::new(proto::RestoreMemoryResponse { restored }))
    }

    async fn fuzzy_discovery(
        &self,
        request: Request<proto::FuzzyDiscoveryRequest>,
    ) -> Result<Response<proto::FuzzyDiscoveryResponse>, Status> {
        let view = self.fuzzy.as_ref().ok_or_else(|| {
            Status::unimplemented("FuzzyDiscovery: no fuzzy-discovery view wired")
        })?;
        let req = request.into_inner();
        // The client-vector path takes precedence (FFI-free); an empty vector
        // falls back to embedding `query_text` (needs an embedder). Mirrors
        // `query_dataset` exactly — but the response carries refs + bp only (no
        // content echo, exact-out).
        let qe = (!req.query_embedding.is_empty()).then_some(req.query_embedding.as_slice());
        let mode = crate::datasets::retrieval_mode_from_proto(req.retrieval_mode);
        let hits = view
            .discover(&req.dataset, qe, &req.query_text, req.k as usize, mode)
            .map_err(crate::datasets::dataset_status)?;
        let hits = hits
            .into_iter()
            .map(crate::fuzzy_discovery::fuzzy_hit_to_proto)
            .collect();
        Ok(Response::new(proto::FuzzyDiscoveryResponse { hits }))
    }
}

/// Map a gateway-core team summary into the wire type.
fn team_summary_to_proto(t: TeamSummaryEntry) -> proto::TeamSummary {
    proto::TeamSummary {
        team_id: t.team_id,
        display_name: t.display_name,
        owner: t.owner,
        member_count: t.member_count,
    }
}

/// Map a gateway-core team member (with its optional warrant) into the wire type.
fn team_member_to_proto(m: TeamMemberEntry) -> proto::TeamMember {
    proto::TeamMember {
        party: m.party,
        role: m.role,
        action_caps: m.action_caps,
        resolved_warrant: m.resolved_warrant.map(warrant_view_to_proto),
    }
}

/// Map a gateway-core warrant projection into the wire type.
fn warrant_view_to_proto(w: WarrantProjection) -> proto::WarrantView {
    proto::WarrantView {
        executor_class: w.executor_class,
        model_route: w.model_route,
        net_scope: w.net_scope,
        fs_scope: w.fs_scope,
        max_calls: w.max_calls,
        cpu_milli: w.cpu_milli,
        wall_clock_ms: w.wall_clock_ms,
    }
}

/// Map a gateway-core grant entry into the wire type.
fn grant_entry_to_proto(g: GrantEntry) -> proto::GrantView {
    proto::GrantView {
        grantor: g.grantor,
        grantee: g.grantee,
        actions: g.actions,
        runtime_scope: g.runtime_scope,
        is_root: g.is_root,
        revoked: g.revoked,
    }
}

/// Map a gateway-core form field into the wire type.
/// Build a wire `RecipeSummary` for `handle` from the catalog seam — the
/// fingerprint (PR-2.1 join key) + the advisory metadata (PR-4 Batch D). Shared
/// by `ListRecipes`; `SearchRecipes` builds its own (the ranker already carries
/// metadata, and a search hit need not re-resolve the fingerprint).
fn recipe_summary_to_proto(catalog: &dyn RecipeCatalog, handle: String) -> proto::RecipeSummary {
    let recipe_fingerprint = catalog
        .recipe_fingerprint(&handle)
        .map_or_else(Vec::new, |f| f.to_vec());
    let meta = catalog.recipe_metadata(&handle).unwrap_or_default();
    proto::RecipeSummary {
        handle,
        recipe_fingerprint,
        description: meta.description,
        tags: meta.tags,
        version: meta.version,
    }
}

fn form_field_to_proto(f: RecipeFormFieldEntry) -> proto::RecipeFormField {
    let ty = match f.kind {
        RecipeParamKind::Unspecified => proto::RecipeParamType::Unspecified,
        RecipeParamKind::Str => proto::RecipeParamType::Str,
        RecipeParamKind::Int => proto::RecipeParamType::Int,
        RecipeParamKind::Bool => proto::RecipeParamType::Bool,
        RecipeParamKind::Bytes => proto::RecipeParamType::Bytes,
        RecipeParamKind::Enum => proto::RecipeParamType::Enum,
    };
    proto::RecipeFormField {
        name: f.name,
        r#type: ty as i32,
        required: f.required,
        max_len: f.max_len,
        allowed: f.allowed,
    }
}
