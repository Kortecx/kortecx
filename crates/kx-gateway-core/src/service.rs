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

use crate::datasets::DatasetView;
use crate::error::{hash_32, instance_id_16};
use crate::identity::CallerParty;
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
    ) -> Result<BoundRecipe, BinderError>;
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
    /// The `StreamEvents` tailer. Defaults to [`SnapshotTailer`]; the host injects
    /// a live tailer via [`GatewayService::with_event_tailer`].
    tailer: Arc<dyn EventTailer>,
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
}

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
            tailer: Arc::new(SnapshotTailer),
            critics_supported: false,
            react_supported: false,
            registered_tools: std::collections::BTreeSet::new(),
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

    /// Wire a live `StreamEvents` tailer (R5 — `kx-gateway`'s `LiveTailer`),
    /// replacing the default snapshot-to-head [`SnapshotTailer`]. Read-side only;
    /// it never changes the journal or the digest.
    #[must_use]
    pub fn with_event_tailer(mut self, tailer: Arc<dyn EventTailer>) -> Self {
        self.tailer = tailer;
        self
    }
}

fn submit_status(err: SubmitterError) -> Status {
    match err {
        SubmitterError::Rejected(detail) => Status::failed_precondition(detail),
        SubmitterError::Transport(detail) => Status::unavailable(detail),
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
            // B3: enforce the CROSS-Mote critic refusals (R-2/R-4/R-5/R-6) the per-Mote
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
                Status::failed_precondition(format!("critic admission refused: {e}"))
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
            .bind(&party, &req.handle, &req.args)
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
        // the fail-closed backstop against drift.
        for (_, warrant) in &bound.motes {
            if let Some(grant) = warrant.tool_grants.iter().find(|g| {
                !self
                    .registered_tools
                    .contains(&(g.tool_id.0.clone(), g.tool_version.0.clone()))
            }) {
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
        let react_seed = bound.react_seed;
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
        let instance_id = instance_id_16(&req.instance_id)?;
        let content_ref = hash_32(&req.content_ref, "content_ref must be 32 bytes")?;
        let payload = view::get_owned_content(
            self.reader.as_ref(),
            self.content.as_ref(),
            instance_id,
            content_ref,
        )?;
        Ok(Response::new(proto::ContentBlob { payload }))
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
            req.limit,
            req.instance_id.as_deref(),
        )?;
        Ok(Response::new(resp))
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
            .map(|handle| proto::RecipeSummary { handle })
            .collect();
        Ok(Response::new(proto::ListRecipesResponse { recipes }))
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
        let hits = view
            .query(&req.dataset, qe, &req.query_text, req.k as usize)
            .map_err(crate::datasets::dataset_status)?;
        let hits = hits
            .into_iter()
            .map(crate::datasets::dataset_hit_to_proto)
            .collect();
        Ok(Response::new(proto::QueryDatasetResponse { hits }))
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
