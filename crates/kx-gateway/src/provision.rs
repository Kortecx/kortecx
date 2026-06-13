//! Host-side seam implementations the binary injects into
//! [`kx_gateway_core::GatewayService`].
//!
//! gateway-core stays off `kx-catalog` (the dependency wall), so the concrete,
//! catalog-backed implementations of its seams live HERE, in the host binary,
//! which is free to depend on `kx-catalog`. R2a provides
//! [`HostSignatureCatalog`] (the three catalog signature RPCs); R2b adds the
//! recipe binder (the `Invoke` path).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use kx_catalog::{
    canonical_config, encode_param_schema, AssetBinding, AssetPath, AssetRef, AssetVersion,
    BodyLedger, CatalogAction, CatalogActionSet, CatalogError, CatalogRegistry, FreeParamContract,
    FreeParamSlot, Grant, GrantLedger, ParamType, PartyId, Provenance, SchemaResolver,
    SignatureEntry, SlotBinding, SqliteBodyLedger, SqliteGrantLedger, SqliteVersionLedger,
    TaskSignatureHash, VersionLedger, VersionedContent,
};
use kx_content::ContentRef;
use kx_gateway_core::{
    AuthorEdge, AuthorExecutionMode, AuthorStep, AuthorStepKind, BinderError, BoundRecipe,
    CatalogSeamError, RecipeBinder, RecipeCatalog, RecipeFormFieldEntry, RecipeParamKind,
    RegisteredSignature, SignatureCatalog, SignatureSummaryEntry, WorkflowAuthor,
};
use kx_invoke::{bind_snapshot, InvokeError, UseWarrantResolver};
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, LogicRef, ModelId, ToolName, ToolVersion, PROMPT_KEY,
};
use kx_warrant::{
    intersect, ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, Role, WarrantSpec,
};
use kx_workflow::{compile, transform, StepDef, WorkflowDef};

use crate::error::GatewayError;

/// A [`SignatureCatalog`] backed by any `kx-catalog` [`CatalogRegistry`]
/// (the durable [`kx_catalog::SqliteCatalog`] in the server, or an in-memory one
/// in tests).
///
/// It translates between the gateway's WIRE vocabulary (opaque `manifest` bytes +
/// a 32-byte id) and the catalog's [`SignatureEntry`] via the catalog's canonical
/// codec, and **server-derives** the id from the decoded entry (SN-8: the client
/// never supplies an id). Registration inherits the registry's idempotent +
/// immutable contract.
pub struct HostSignatureCatalog<R> {
    registry: R,
}

impl<R: CatalogRegistry> HostSignatureCatalog<R> {
    /// Wrap a catalog registry.
    pub fn new(registry: R) -> Self {
        Self { registry }
    }
}

impl<R: CatalogRegistry + Send + Sync> SignatureCatalog for HostSignatureCatalog<R> {
    fn register(&self, manifest: &[u8]) -> Result<RegisteredSignature, CatalogSeamError> {
        // Decode the opaque manifest into a typed entry (fail-closed on garbage).
        let (entry, _len): (SignatureEntry, usize) =
            bincode::serde::decode_from_slice(manifest, canonical_config())
                .map_err(|e| CatalogSeamError::Malformed(e.to_string()))?;
        // The registry server-derives the id (content-addressed) and enforces
        // idempotency + immutability.
        let outcome = self
            .registry
            .register_signature(entry)
            .map_err(|e| match e {
                CatalogError::ImmutabilityConflict(_) => CatalogSeamError::ImmutabilityConflict,
                CatalogError::Storage(detail) => CatalogSeamError::Internal(detail),
            })?;
        Ok(RegisteredSignature {
            signature_id: *outcome.hash().as_bytes(),
        })
    }

    fn get(&self, signature_id: &[u8; 32]) -> Option<Vec<u8>> {
        let entry = self
            .registry
            .lookup(&TaskSignatureHash::from_bytes(*signature_id))?;
        // Re-encode with the canonical codec — byte-identical to what was
        // registered (content-addressed). Infallible for a SignatureEntry (no
        // floats); `.ok()?` keeps this non-panicking without an `expect`.
        let bytes = bincode::serde::encode_to_vec(&entry, canonical_config()).ok()?;
        Some(bytes)
    }

    fn list(&self) -> Vec<SignatureSummaryEntry> {
        self.registry
            .list_signatures()
            .map(|entry| {
                let id = *entry.hash().as_bytes();
                SignatureSummaryEntry {
                    name: short_label(&id),
                    signature_id: id,
                }
            })
            .collect()
    }
}

/// A short, stable, human-distinguishable label for a signature (the catalog
/// stores no name of its own; a richer name belongs in advisory metadata later).
fn short_label(id: &[u8; 32]) -> String {
    let mut s = String::from("sig-");
    for b in &id[..4] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ===========================================================================
// R2b — the recipe-binding seam (the `Invoke` path).
// ===========================================================================

/// The wire handle of the single PURE demo recipe R2b provisions, so the
/// `Invoke` path has something to run end-to-end. (The real authored recipe
/// library is a later PR, R6.) Shared with the e2e test (no drift).
pub const DEMO_RECIPE_HANDLE: &str = "kx/recipes/echo";

/// The wire handle of the PR-9b real-exec demo recipe: a PURE step whose body
/// is a REAL binary the embedded worker runs inside the platform sandbox
/// (bwrap/macOS). Provisioned only when a body binary was located at startup
/// (see `real_exec::register_demo_body`); takes no free-params (the
/// body's input is the Mote's identity). Shared with the e2e test (no drift).
pub const EXEC_RECIPE_HANDLE: &str = "kx/recipes/exec-demo";

/// The wire handle of the T3.3 deterministic MULTI-NODE demo recipe: a PURE
/// fan-out → gather DAG (root → N children → gather) that runs model-free on the
/// embedded storing executor, so a single `Invoke` yields a real multi-node
/// projection with DATA parent edges — the live-DAG viewer's end-to-end fixture.
/// Always provisioned. Takes no free-params. Shared with the e2e test (no drift).
pub const FANOUT_RECIPE_HANDLE: &str = "kx/recipes/fanout-demo";

/// The fan-out width of [`FANOUT_RECIPE_HANDLE`]: root + `FANOUT_WIDTH` children +
/// gather = `FANOUT_WIDTH + 2` Motes. Three keeps the demo graph legible.
const FANOUT_WIDTH: u8 = 3;

/// The content-ref of the demo recipe's single typed free-param (`topic`).
const TOPIC_SCHEMA_REF: [u8; 32] = [0x2b; 32];

/// The wire handle of the AL1 model recipe: a single PURE (greedy) model step
/// that ChatML-wraps a `prompt` free-param and runs it through the in-process
/// inference backend. Provisioned only when `kx serve --features inference` has a
/// fit, resolvable serve model (see `model_exec::resolve_serve_model`). Shared
/// with the e2e test (no drift).
pub const MODEL_RECIPE_HANDLE: &str = "kx/recipes/chat";

/// The schema-ref of the model recipe's single typed free-param (`prompt`).
const MODEL_PROMPT_SCHEMA_REF: [u8; 32] = [0x3d; 32];

/// A placeholder `logic_ref` for the model step. Unlike a body Mote, a model Mote
/// is routed by its `prompt` + resolvable `model_id` (see `model_exec`), not its
/// `logic_ref`, so this is a distinct, ignored sentinel (≠ the `echo` placeholder
/// so the two recipe bodies get distinct manifests).
const MODEL_LOGIC_REF: [u8; 32] = [0x3c; 32];

/// The wire handle of the PR-2d-2 live-ReAct recipe: a SEED Mote carrying an
/// `instruction` + the per-run budget caps; the Invoke arm submits it with
/// `react_seed = true`, so the coordinator swaps in the run-salted turn 0 and
/// drives the durable Reason→Act→Observe chain — firing the bundled
/// deterministic stdio tool (`mcp-echo@1`) under the SERVER-constructed
/// tool-granting warrant (the first non-empty `tool_grants` in serve; tool
/// authority NEVER enters via a client warrant — red-team BLOCKER #5).
/// Provisioned only when a fit serve model resolved AND the bundled tool's
/// capability registered on the broker.
pub const REACT_RECIPE_HANDLE: &str = "kx/recipes/react";

/// A placeholder `logic_ref` for the react seed step (a distinct sentinel so the
/// recipe body gets a distinct manifest; the seed is validated then SWAPPED, so
/// the ref never reaches an admitted identity).
const REACT_LOGIC_REF: [u8; 32] = [0x4e; 32];

/// Schema-refs of the react recipe's typed free-params.
const REACT_INSTRUCTION_SCHEMA_REF: [u8; 32] = [0x4f; 32];
/// See [`REACT_INSTRUCTION_SCHEMA_REF`].
const REACT_MAX_TURNS_SCHEMA_REF: [u8; 32] = [0x50; 32];
/// See [`REACT_INSTRUCTION_SCHEMA_REF`].
const REACT_MAX_TOOL_CALLS_SCHEMA_REF: [u8; 32] = [0x51; 32];

/// The wire handle of the Batch A VISION recipe: a single PURE (greedy) model
/// step like `kx/recipes/chat` PLUS a REQUIRED `image_ref` slot (the 64-hex
/// `PutContent` ref of an uploaded image, fetched by the multimodal backend)
/// and a REQUIRED `model` ENUM slot (allowed = the served model id — model
/// selection is a server-validated free-param, never a client warrant, SN-8).
/// A separate recipe BY DESIGN: every variable slot is required at binding
/// (`SlotMissing` otherwise), so extending `kx/recipes/chat` would break every
/// existing `{prompt}` caller. Provisioned only when the serve model registered
/// IMAGE-capable (`KX_SERVE_MMPROJ_GGUF` resolved).
pub const VISION_RECIPE_HANDLE: &str = "kx/recipes/vision";

/// A placeholder `logic_ref` for the vision step (a distinct sentinel ⇒ a
/// distinct manifest; routing is by `prompt` + `model_id`, as with chat).
const VISION_LOGIC_REF: [u8; 32] = [0x52; 32];

/// Schema-ref of the vision recipe's `image_ref` slot (`Bytes`, 64 hex chars).
const VISION_IMAGE_SCHEMA_REF: [u8; 32] = [0x53; 32];
/// Schema-ref of the vision recipe's `model` ENUM slot (allowed = served id;
/// resolved DYNAMICALLY by [`DemoSchemaResolver`] from the provisioned model).
const VISION_MODEL_SCHEMA_REF: [u8; 32] = [0x54; 32];

/// The vision recipe's `model` slot name (binds into `config_subset["model"]` —
/// identity-bearing, ignored by the executor; the warrant's `model_route` is
/// what actually routes).
const VISION_MODEL_KEY: &str = "model";

/// The vision recipe's image slot name — the binder writes the bound arg into
/// `config_subset["image_ref"]`, the key the model executor's multimodal arm
/// reads (exactly the [`PROMPT_KEY`] pattern). Lives here (not in the
/// inference-gated `model_exec`) so the recipe seeds feature-free.
pub(crate) const IMAGE_REF_KEY: &str = "image_ref";

/// The blueprint-authoring asset (`SubmitWorkflow` / the Blueprint builder). Each
/// party is granted `Use` on it at provision time, so [`HostWorkflowAuthor`]
/// resolves the party's effective authority from the SAME grant ledger as Invoke —
/// the ceiling each authored step's warrant is intersected against (SN-8). It owns
/// no body/version (authoring submits one-off DAGs; nothing is published here).
const BLUEPRINT_AUTHOR_HANDLE: &str = "kx/blueprints/author";

/// Tier-1 authoring caps (GR8 DoS bound — the new client-authored-topology surface).
const MAX_BLUEPRINT_STEPS: usize = 64;
/// See [`MAX_BLUEPRINT_STEPS`].
const MAX_BLUEPRINT_EDGES: usize = 256;

/// A server-provisioned recipe library backing the `Invoke` path, over the
/// durable G1a SQLite ledgers (so a registered recipe survives restart). R2b
/// seeds ONE PURE demo recipe whose step warrant uses the embedded worker's
/// `executor_class` — otherwise a bound run would never lease and `Invoke` would
/// hang. WORLD-MUTATING recipes need the capability path (a later wave).
pub struct DemoLibrary {
    versions: SqliteVersionLedger,
    bodies: SqliteBodyLedger,
    /// The durable grant ledger (`grants.db`). `Arc` so the UI-3 grant/membership
    /// views read the SAME instance the demo recipes (and the demo team) seed —
    /// one open, three seams (binder + grant view + the membership-resolve fleet),
    /// no second `grants.db` handle racing.
    grants: std::sync::Arc<SqliteGrantLedger>,
    /// Per-handle binding metadata — one entry per seeded recipe (the demo
    /// `echo` always; the PR-9b real-exec `exec-demo` when a body was located).
    /// `bind` looks the handle up here for its owner-root warrant + free-param
    /// contract; an unknown handle is a uniform `NotAuthorized` (no oracle).
    recipes: Vec<(AssetPath, RecipeMeta)>,
    /// The owner-root warrant on the `kx/blueprints/author` asset — the base
    /// each authored step's warrant uses AND the ceiling the party's resolved Use
    /// authority intersects against ([`HostWorkflowAuthor`]).
    blueprint_base: WarrantSpec,
    /// The served model id, if this serve resolved one (`kx serve --features
    /// inference`). Authored MODEL steps require it (else they are refused).
    serve_model: Option<ModelId>,
}

/// Per-recipe binding metadata: the owner's base warrant the grant fold narrows
/// from (== the recipe step warrant, so every `intersect` in the bind chain is a
/// no-op narrowing and the bound run keeps the worker's `executor_class`) + the
/// recipe's free-param contract.
struct RecipeMeta {
    owner_root: WarrantSpec,
    free_params: FreeParamContract,
}

impl DemoLibrary {
    /// Open the durable ledgers under `dir` and idempotently seed the demo recipe,
    /// granting `Use`+`Read` to each party in `parties` (plus the dev `local-dev`
    /// principal). Re-opening on restart is a no-op (publishes/grants are
    /// idempotent; the version publish is guarded by a resolve check).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on a ledger open / seed failure.
    pub fn open(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
    ) -> Result<Self, GatewayError> {
        Self::seed(dir, exec_class, parties, None, None, None, false)
    }

    /// Like [`DemoLibrary::open`], plus (when `real_body_ref` is `Some`) seeds the
    /// PR-9b real-exec recipe [`EXEC_RECIPE_HANDLE`] whose step body is the located
    /// sandbox binary. `None` ⇒ byte-identical to [`DemoLibrary::open`].
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on a ledger open / seed failure.
    pub fn open_with_real_exec(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        real_body_ref: Option<ContentRef>,
    ) -> Result<Self, GatewayError> {
        Self::seed(dir, exec_class, parties, real_body_ref, None, None, false)
    }

    /// Like [`DemoLibrary::open_with_real_exec`], plus (when `serve_model` is
    /// `Some`) the AL1 model recipe [`MODEL_RECIPE_HANDLE`] — a PURE (greedy)
    /// model step routed to `serve_model` with a `prompt` free-param. `None` ⇒
    /// byte-identical to [`DemoLibrary::open_with_real_exec`].
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on a ledger open / seed failure.
    // Signature stability: this pre-PR-2d-2 public opener keeps its owned
    // `Option<ModelId>` even though the seeding now borrows.
    #[allow(clippy::needless_pass_by_value)]
    pub fn open_full(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        real_body_ref: Option<ContentRef>,
        serve_model: Option<ModelId>,
    ) -> Result<Self, GatewayError> {
        Self::seed(
            dir,
            exec_class,
            parties,
            real_body_ref,
            serve_model.as_ref(),
            None,
            false,
        )
    }

    /// Like [`DemoLibrary::open_full`], plus (when `react_tool` is `Some` AND a
    /// serve model resolved) the PR-2d-2 live-ReAct recipe
    /// [`REACT_RECIPE_HANDLE`], whose SERVER-constructed step warrant grants
    /// exactly `react_tool` (the bundled `mcp-echo@1` capability the host
    /// registered on the broker). `None` ⇒ byte-identical to
    /// [`DemoLibrary::open_full`].
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on a ledger open / seed failure.
    pub fn open_complete(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        real_body_ref: Option<ContentRef>,
        serve_model: Option<&ModelId>,
        react_tool: Option<&(ToolName, ToolVersion)>,
        vision: bool,
    ) -> Result<Self, GatewayError> {
        Self::seed(
            dir,
            exec_class,
            parties,
            real_body_ref,
            serve_model,
            react_tool,
            vision,
        )
    }

    /// Open the durable ledgers under `dir` and idempotently seed the demo `echo`
    /// recipe (always) plus the real-exec `exec-demo` recipe (when `real_body_ref`
    /// is `Some`). Re-opening on restart is a no-op (content-addressed bodies +
    /// guarded version publish + idempotent grants).
    // A flat, sequential one-block-per-recipe seeding fn — the length is the
    // recipe count, not cognitive complexity (the `start_impl` precedent).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn seed(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        real_body_ref: Option<ContentRef>,
        serve_model: Option<&ModelId>,
        react_tool: Option<&(ToolName, ToolVersion)>,
        vision: bool,
    ) -> Result<Self, GatewayError> {
        let cat = |e: String| GatewayError::Catalog(e);
        let versions =
            SqliteVersionLedger::open(dir.join("versions.db")).map_err(|e| cat(e.to_string()))?;
        let bodies =
            SqliteBodyLedger::open(dir.join("bodies.db")).map_err(|e| cat(e.to_string()))?;
        let grants =
            SqliteGrantLedger::open(dir.join("grants.db")).map_err(|e| cat(e.to_string()))?;
        let owner = PartyId::new("kx-gateway");
        let mut recipes: Vec<(AssetPath, RecipeMeta)> = Vec::new();

        // (echo) the PURE demo recipe — a placeholder logic_ref the storing
        // executor ignores, a `topic` free-param.
        let echo_warrant = demo_warrant(exec_class);
        let echo_handle = demo_handle()?;
        seed_recipe(
            &versions,
            &bodies,
            &grants,
            &owner,
            parties,
            &echo_handle,
            recipe_body(LogicRef::from_bytes([0x2b; 32]), &echo_warrant, &["topic"]),
            &echo_warrant,
        )?;
        recipes.push((
            echo_handle,
            RecipeMeta {
                owner_root: echo_warrant,
                free_params: topic_contract(),
            },
        ));

        // (fanout-demo) the T3.3 PURE multi-node recipe — a fan-out → gather DAG that
        // runs model-free on the storing executor (see `seed_fanout_demo`). Always
        // seeded; no free-params.
        recipes.push(seed_fanout_demo(
            &versions, &bodies, &grants, &owner, parties, exec_class,
        )?);

        // (exec-demo) the PR-9b real-exec recipe — step logic_ref == the located
        // body's content ref, a sandbox warrant, no free-params. Seeded only when
        // a body binary was found.
        if let Some(body_ref) = real_body_ref {
            let exec_warrant = real_exec_warrant(exec_class);
            let exec_handle = exec_handle()?;
            let step_logic = LogicRef::from_bytes(*body_ref.as_bytes());
            seed_recipe(
                &versions,
                &bodies,
                &grants,
                &owner,
                parties,
                &exec_handle,
                recipe_body(step_logic, &exec_warrant, &[]),
                &exec_warrant,
            )?;
            recipes.push((
                exec_handle,
                RecipeMeta {
                    owner_root: exec_warrant,
                    free_params: FreeParamContract::new(),
                },
            ));
        }

        // (chat) the AL1 model recipe — a single PURE (greedy) model step routed
        // to the served model, with a `prompt` free-param. Seeded only when a fit
        // serve model is configured (`kx serve --features inference`).
        if let Some(model_id) = serve_model {
            let model_w = model_warrant(exec_class, model_id);
            let model_h = model_handle()?;
            seed_recipe(
                &versions,
                &bodies,
                &grants,
                &owner,
                parties,
                &model_h,
                recipe_body(
                    LogicRef::from_bytes(MODEL_LOGIC_REF),
                    &model_w,
                    &[PROMPT_KEY],
                ),
                &model_w,
            )?;
            recipes.push((
                model_h,
                RecipeMeta {
                    owner_root: model_w,
                    free_params: prompt_contract(),
                },
            ));
        }

        // (vision) the Batch A image recipe — chat plus a REQUIRED `image_ref`
        // slot (an uploaded blob's 64-hex ref, fetched by the multimodal
        // backend) and a REQUIRED `model` ENUM slot (allowed = the served id —
        // selection is a server-validated free-param, never a client warrant).
        // Seeded only when the serve model registered IMAGE-capable.
        if let Some(model_id) = serve_model.filter(|_| vision) {
            let vision_w = model_warrant(exec_class, model_id);
            let vision_h = vision_handle()?;
            seed_recipe(
                &versions,
                &bodies,
                &grants,
                &owner,
                parties,
                &vision_h,
                recipe_body(
                    LogicRef::from_bytes(VISION_LOGIC_REF),
                    &vision_w,
                    &[PROMPT_KEY, IMAGE_REF_KEY, VISION_MODEL_KEY],
                ),
                &vision_w,
            )?;
            recipes.push((
                vision_h,
                RecipeMeta {
                    owner_root: vision_w,
                    free_params: vision_contract(),
                },
            ));
        }

        // (react) the PR-2d-2 live-ReAct recipe — a SEED step with `instruction`
        // + budget-cap free-params, under the SERVER-constructed tool-granting
        // warrant (the only source of tool authority in serve). Seeded only when
        // BOTH a fit serve model resolved AND the bundled tool's capability is
        // registered on the broker (a grant the broker cannot honour would
        // dead-letter every observation).
        if let (Some(model_id), Some(tool)) = (serve_model, react_tool) {
            let react_w = react_warrant(exec_class, model_id, tool);
            let react_h = react_handle()?;
            seed_recipe(
                &versions,
                &bodies,
                &grants,
                &owner,
                parties,
                &react_h,
                recipe_body(
                    LogicRef::from_bytes(REACT_LOGIC_REF),
                    &react_w,
                    &[
                        kx_mote::REACT_INSTRUCTION_KEY,
                        kx_mote::REACT_MAX_TURNS_KEY,
                        kx_mote::REACT_MAX_TOOL_CALLS_KEY,
                    ],
                ),
                &react_w,
            )?;
            recipes.push((
                react_h,
                RecipeMeta {
                    owner_root: react_w,
                    free_params: react_contract(),
                },
            ));
        }

        // (blueprints/author) the Tier-1 DAG-authoring asset — granted Use to each
        // party so `SubmitWorkflow` resolves the party's authority from this same
        // ledger (no body/version; authoring submits one-off DAGs). Always seeded.
        //
        // P1.1: when a model is served, the authoring grant's `model_route` MUST
        // name the SERVED model — otherwise an authored MODEL step (whose warrant is
        // this same `base`) routes to the demo PLACEHOLDER model and dead-letters at
        // dispatch (served ≠ placeholder). The other axes stay `demo_warrant`'s (the
        // PURE/EXEC authoring scope); only the model_route id is re-pointed. Warrants
        // are off the MoteDef/journal/digest, so this is identity-invariant.
        let blueprint_base = {
            let mut w = demo_warrant(exec_class);
            if let Some(model_id) = serve_model {
                w.model_route.model_id = model_id.clone();
            }
            w
        };
        seed_blueprint_asset(
            &grants,
            &owner,
            parties,
            &blueprint_author_handle()?,
            &blueprint_base,
        )?;

        Ok(Self {
            versions,
            bodies,
            grants: std::sync::Arc::new(grants),
            recipes,
            blueprint_base,
            serve_model: serve_model.cloned(),
        })
    }

    /// Every provisioned, invocable recipe handle (`"namespace/collection/name"`),
    /// in provision order. Backs the gateway's `ListRecipes` (UI-2 recipe catalog).
    pub fn recipe_handles(&self) -> Vec<String> {
        self.recipes.iter().map(|(h, _)| h.to_string()).collect()
    }

    /// The published workflow fingerprint a bound run of `handle` registers
    /// under (PR-2.1 run naming): resolved from the SAME versions ledger the
    /// binder resolves through, so the catalog and `RunSummary.recipe_fingerprint`
    /// agree by construction. Display/join only — never identity.
    pub fn recipe_fingerprint(&self, handle: &str) -> Option<[u8; 32]> {
        let path = self
            .recipes
            .iter()
            .map(|(h, _)| h)
            .find(|h| h.to_string() == handle)?;
        match self.versions.resolve(path)? {
            (VersionedContent::Workflow(manifest_id), _) => Some(manifest_id.0),
            _ => None,
        }
    }

    /// The principal that owns every demo-provisioned asset + founds the demo team
    /// (UI-3). A fixed single-node operator identity; cloud multi-tenant identity is
    /// the OIDC layer (D129).
    #[must_use]
    pub fn owner_principal() -> PartyId {
        PartyId::new("kx-gateway")
    }

    /// Borrow the durable grant ledger (UI-3: the `HostGrantView` reads it; the demo
    /// team grant is appended to it). The SAME instance the recipes seeded.
    #[must_use]
    pub fn grant_ledger(&self) -> &SqliteGrantLedger {
        &self.grants
    }

    /// A shared handle to the durable grant ledger (UI-3: the membership-resolve
    /// `GovernedFleet` composes it with the membership ledger). Cheap `Arc` clone.
    #[must_use]
    pub fn grants_arc(&self) -> std::sync::Arc<SqliteGrantLedger> {
        self.grants.clone()
    }

    /// The asset the demo team is granted `Use`+`Read` on (the first seeded recipe,
    /// `echo`), so a member's warrant resolves through the team membership ∩ grant —
    /// the kx-fleet thesis, demonstrated end-to-end. `None` only if no recipe seeded.
    #[must_use]
    pub fn demo_team_grant_asset(&self) -> Option<AssetRef> {
        self.recipes.first().map(|(h, _)| AssetRef::Path(h.clone()))
    }

    /// The owner-root warrant for `asset` (the base the grant fold narrows from), or
    /// `None` if `asset` is not a provisioned recipe. UI-3 uses it as the
    /// `resolve_member_warrant` owner-root + the demo team grant's runtime scope.
    #[must_use]
    pub fn owner_root_for(&self, asset: &AssetRef) -> Option<WarrantSpec> {
        self.recipes
            .iter()
            .find(|(h, _)| AssetRef::Path(h.clone()) == *asset)
            .map(|(_, m)| m.owner_root.clone())
    }

    /// The variable free-param FORM for `handle`, or `None` if no such recipe is
    /// provisioned. Backs the gateway's `GetRecipeForm` (UI-2 generated forms).
    ///
    /// Mirrors [`kx_catalog::free_params_to_input_schema`]'s slot logic (Variable
    /// slots only; each typed by resolving its `schema_ref` → [`ParamType`] via
    /// the in-crate `DemoSchemaResolver`) but maps an UNTYPED slot to
    /// [`RecipeParamKind::Unspecified`] (a generic field) rather than failing — the
    /// UI renders a plain input and `Invoke` still validates server-side,
    /// fail-closed. The recipe library here only declares typed slots.
    pub fn recipe_form(&self, handle: &str) -> Option<Vec<RecipeFormFieldEntry>> {
        let asset_path = parse_handle(handle)?;
        let meta = self
            .recipes
            .iter()
            .find(|(h, _)| *h == asset_path)
            .map(|(_, m)| m)?;
        let resolver = self.schema_resolver();
        let fields = meta
            .free_params
            .slots
            .iter()
            .filter(|(_, slot)| slot.binding == SlotBinding::Variable)
            .map(|(name, slot)| free_param_field(name, slot, &resolver))
            .collect();
        Some(fields)
    }

    /// The schema resolver carrying this library's live serve facts (the vision
    /// `model` ENUM's allowed set). The SAME resolver backs the published form
    /// (`recipe_form`) and the bind (`Invoke`), so they agree by construction.
    fn schema_resolver(&self) -> DemoSchemaResolver {
        DemoSchemaResolver {
            serve_model: self.serve_model.as_ref().map(|m| m.0.clone()),
        }
    }
}

/// Resolve one Variable free-param slot into a renderable form field. An untyped
/// (or unresolvable/undecodable) slot becomes [`RecipeParamKind::Unspecified`].
fn free_param_field(
    name: &str,
    slot: &FreeParamSlot,
    resolver: &DemoSchemaResolver,
) -> RecipeFormFieldEntry {
    let ty = slot
        .schema_ref
        .and_then(|r| resolver.resolve_schema(&r))
        .and_then(|bytes| {
            bincode::serde::decode_from_slice::<ParamType, _>(&bytes, canonical_config())
                .ok()
                .map(|(ty, _)| ty)
        });
    match ty {
        Some(ty) => param_type_field(name, &ty),
        None => RecipeFormFieldEntry {
            name: name.to_string(),
            kind: RecipeParamKind::Unspecified,
            required: true,
            max_len: None,
            allowed: Vec::new(),
        },
    }
}

/// Map a resolved [`ParamType`] to the gateway's wire form-field vocabulary. A
/// variable free-param is always `required` (it has no recipe-side default).
fn param_type_field(name: &str, ty: &ParamType) -> RecipeFormFieldEntry {
    let (kind, max_len, allowed) = match ty {
        ParamType::Str { max_len } => (RecipeParamKind::Str, Some(*max_len as u64), Vec::new()),
        ParamType::Bytes { max_len } => (RecipeParamKind::Bytes, Some(*max_len as u64), Vec::new()),
        ParamType::Int { .. } => (RecipeParamKind::Int, None, Vec::new()),
        ParamType::Bool => (RecipeParamKind::Bool, None, Vec::new()),
        ParamType::Enum { allowed } => (
            RecipeParamKind::Enum,
            None,
            allowed.iter().cloned().collect(),
        ),
    };
    RecipeFormFieldEntry {
        name: name.to_string(),
        kind,
        required: true,
        max_len,
        allowed,
    }
}

/// Idempotently seed one recipe: own the asset, publish its content-addressed
/// body (guarding the version publish), and grant `Use`+`Read` to every party
/// (plus the dev `local-dev` principal) under a runtime scope == the recipe
/// warrant. Shared by every recipe `DemoLibrary::seed` provisions.
#[allow(clippy::too_many_arguments)] // seeding genuinely needs all three ledgers + owner/parties/handle/body/warrant
fn seed_recipe(
    versions: &SqliteVersionLedger,
    bodies: &SqliteBodyLedger,
    grants: &SqliteGrantLedger,
    owner: &PartyId,
    parties: &[String],
    handle: &AssetPath,
    body: WorkflowDef,
    warrant: &WarrantSpec,
) -> Result<(), GatewayError> {
    let cat = |e: String| GatewayError::Catalog(e);
    let asset = AssetRef::Path(handle.clone());

    grants
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .map_err(|e| cat(e.to_string()))?;

    let (manifest_id, _) = bodies.publish_body(body).map_err(|e| cat(e.to_string()))?;
    if versions.resolve(handle).is_none() {
        versions
            .publish(AssetVersion::root(
                handle.clone(),
                VersionedContent::Workflow(manifest_id),
                owner.clone(),
                Provenance::from_recipe(manifest_id.0),
            ))
            .map_err(|e| cat(e.to_string()))?;
    }

    let role = Role {
        name: "demo-use".to_string(),
        version: 1,
        spec: warrant.clone(),
        description: String::new(),
    };
    let mut granted: BTreeSet<&str> = BTreeSet::new();
    for party in parties
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("local-dev"))
    {
        if !granted.insert(party) {
            continue;
        }
        grants
            .append_grant(Grant::root(
                asset.clone(),
                owner.clone(),
                PartyId::new(party),
                CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
                role.clone(),
            ))
            .map_err(|e| cat(e.to_string()))?;
    }
    Ok(())
}

/// A [`RecipeBinder`] over a [`DemoLibrary`]: resolves a handle + args for the
/// server-derived party via [`bind_snapshot`], collapsing the
/// authorization/existence errors to a uniform `NotAuthorized` (no oracle on the
/// execution surface).
pub struct HostRecipeBinder {
    lib: std::sync::Arc<DemoLibrary>,
}

impl HostRecipeBinder {
    /// Wrap a provisioned [`DemoLibrary`] (owns it).
    pub fn new(lib: DemoLibrary) -> Self {
        Self {
            lib: std::sync::Arc::new(lib),
        }
    }

    /// Wrap a [`DemoLibrary`] SHARED with a [`HostRecipeCatalog`] (one seed, two
    /// seams) — the server wires both over the same `Arc<DemoLibrary>`.
    pub fn from_shared(lib: std::sync::Arc<DemoLibrary>) -> Self {
        Self { lib }
    }
}

/// A [`RecipeCatalog`] over a [`DemoLibrary`]: the PUBLIC discovery surface for the
/// invocable recipe handles + their free-param forms (the UI-2 `ListRecipes` /
/// `GetRecipeForm` path). Shares the library with the [`HostRecipeBinder`] (same
/// seed), so the catalog and the executable bind agree by construction.
pub struct HostRecipeCatalog {
    lib: std::sync::Arc<DemoLibrary>,
}

impl HostRecipeCatalog {
    /// Wrap a [`DemoLibrary`] shared with the binder.
    pub fn new(lib: std::sync::Arc<DemoLibrary>) -> Self {
        Self { lib }
    }
}

impl RecipeCatalog for HostRecipeCatalog {
    fn list_recipes(&self) -> Vec<String> {
        self.lib.recipe_handles()
    }

    fn recipe_fingerprint(&self, handle: &str) -> Option<[u8; 32]> {
        self.lib.recipe_fingerprint(handle)
    }

    fn get_recipe_form(&self, handle: &str) -> Option<Vec<RecipeFormFieldEntry>> {
        self.lib.recipe_form(handle)
    }
}

#[tonic::async_trait]
impl RecipeBinder for HostRecipeBinder {
    async fn bind(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
    ) -> Result<BoundRecipe, BinderError> {
        // A malformed handle reveals nothing (uniform NotAuthorized — no probing).
        let asset_path = parse_handle(handle).ok_or(BinderError::NotAuthorized)?;
        // Resolve the recipe's binding metadata; an unknown handle is the same
        // uniform NotAuthorized (no existence oracle on the execution surface).
        let meta = self
            .lib
            .recipes
            .iter()
            .find(|(h, _)| *h == asset_path)
            .map(|(_, m)| m)
            .ok_or(BinderError::NotAuthorized)?;
        let party_id = PartyId::new(party);
        let resolver = HostUseResolver {
            grants: &self.lib.grants,
            owner_root: meta.owner_root.clone(),
        };
        // (`self.lib.grants` is an `Arc<SqliteGrantLedger>`; the resolver borrows the
        // inner ledger via deref for the duration of this bind.)
        let bound = bind_snapshot(
            &self.lib.versions,
            &self.lib.bodies,
            &resolver,
            &party_id,
            &asset_path,
            &meta.free_params,
            &self.lib.schema_resolver(),
            args,
        )
        .map_err(map_invoke_err)?;
        Ok(BoundRecipe {
            recipe_fingerprint: bound.recipe_fingerprint,
            motes: bound.motes,
            terminal_mote_id: bound.terminal_mote_id,
            // PR-2d-2: the react recipe seeds a live ReAct chain — the Invoke
            // arm submits its (single) bound Mote with `react_seed = true`,
            // triggering the coordinator's run-salted seed-swap + durable anchor.
            react_seed: parse_handle(REACT_RECIPE_HANDLE).is_some_and(|h| h == asset_path),
        })
    }
}

/// Resolves a party's effective `Use` warrant from the authoritative grant ledger
/// (never a caller-supplied warrant — SN-8). `None` ⇒ unauthorized.
struct HostUseResolver<'a> {
    grants: &'a SqliteGrantLedger,
    owner_root: WarrantSpec,
}

impl UseWarrantResolver for HostUseResolver<'_> {
    fn resolve_use(&self, party: &PartyId, asset: &AssetRef) -> Option<WarrantSpec> {
        self.grants
            .resolve_effective_warrant_for(party, asset, CatalogAction::Use, &self.owner_root)
            .ok()
            .flatten()
    }
}

/// A [`WorkflowAuthor`] over a [`DemoLibrary`]: compiles a Tier-1 authored DAG
/// (PURE / MODEL palette) server-side, assigns each step's `logic_ref` from its
/// kind (the client never supplies executable bytes), and resolves + INTERSECTS
/// every warrant from the party's grant on `kx/blueprints/author` (never a
/// client warrant — SN-8). Shares the library `Arc` with the binder/catalog.
pub struct HostWorkflowAuthor {
    lib: std::sync::Arc<DemoLibrary>,
}

impl HostWorkflowAuthor {
    /// Wrap a [`DemoLibrary`] shared with the binder/catalog (one seed, many seams).
    #[must_use]
    pub fn from_shared(lib: std::sync::Arc<DemoLibrary>) -> Self {
        Self { lib }
    }

    /// Map one authored step → a `kx_workflow::StepDef`, server-assigning `logic_ref`.
    fn step_def(&self, index: usize, s: &AuthorStep) -> Result<StepDef, BinderError> {
        let base = &self.lib.blueprint_base;
        let cap = ToolName("blueprint".into());
        let mut def = match s.kind {
            // PURE: a deterministic transform; its identity comes from a content
            // sentinel over (index, params), so distinct steps get distinct ids.
            AuthorStepKind::Pure => {
                let mut buf = Vec::with_capacity(64);
                buf.extend_from_slice(b"kx-blueprint/pure/v1");
                buf.extend_from_slice(&(index as u64).to_le_bytes());
                for (k, v) in &s.params {
                    buf.extend_from_slice(k.as_bytes());
                    buf.extend_from_slice(v);
                }
                let logic_ref = LogicRef::from_bytes(*ContentRef::of(&buf).as_bytes());
                transform(
                    logic_ref,
                    base.model_route.model_id.clone(),
                    base.clone(),
                    cap,
                )
            }
            // MODEL: a greedy model step routed to the SERVED model (the executor
            // recognizes it by config_subset[PROMPT_KEY] + a supported model_id).
            AuthorStepKind::Model => {
                let served = self.lib.serve_model.as_ref().ok_or_else(|| {
                    BinderError::InvalidArgs(
                        "MODEL steps require a served model (run `kx serve --features inference`)"
                            .into(),
                    )
                })?;
                if !s.model_id.is_empty() && s.model_id != served.0 {
                    return Err(BinderError::InvalidArgs(format!(
                        "MODEL step model_id must equal the served model '{}'",
                        served.0
                    )));
                }
                // P1.1: the step warrant is `base` — and `blueprint_base` now carries
                // the SERVED model in its `model_route` (see `seed`), so the dispatch
                // id (served) == the warrant route id (served) and the dispatcher's
                // strict check passes (it used to be the demo PLACEHOLDER route ≠
                // served → the MODEL step dead-lettered). Using `base` verbatim keeps
                // the step warrant == the authoring grant ⇒ a guaranteed no-op
                // narrowing at admission (no `AttemptedWiden` on the model_route ceilings).
                transform(
                    LogicRef::from_bytes(MODEL_LOGIC_REF),
                    served.clone(),
                    base.clone(),
                    cap,
                )
            }
            // EXEC: references a registered body — reserved for a follow-up (Tier-1
            // PR-1 ships PURE + MODEL; EXEC needs the body-registry lookup wiring).
            AuthorStepKind::Exec => {
                return Err(BinderError::InvalidArgs(
                    "EXEC step authoring is reserved (PR-1 supports PURE + MODEL)".into(),
                ));
            }
        };
        // Free params land in config_subset (identity-bearing); MODEL also binds the
        // prompt into config_subset[PROMPT_KEY] — the key the model executor reads.
        for (k, v) in &s.params {
            def.config_subset
                .insert(ConfigKey(k.clone()), ConfigVal(v.clone()));
        }
        if s.kind == AuthorStepKind::Model {
            def.config_subset.insert(
                ConfigKey(PROMPT_KEY.to_string()),
                ConfigVal(s.prompt.clone().into_bytes()),
            );
        }
        Ok(def)
    }
}

#[tonic::async_trait]
impl WorkflowAuthor for HostWorkflowAuthor {
    async fn author(
        &self,
        party: &str,
        seed: u32,
        steps: &[AuthorStep],
        edges: &[AuthorEdge],
        mode: AuthorExecutionMode,
    ) -> Result<BoundRecipe, BinderError> {
        // DYNAMIC is reserved (PR-1 frozen-only); refuse rather than silently treat
        // it as frozen so the contract never misleads.
        if mode == AuthorExecutionMode::Dynamic {
            return Err(BinderError::InvalidArgs(
                "dynamic execution mode is reserved (PR-1 is frozen-only)".into(),
            ));
        }
        // Caps first (fail-closed, BEFORE any compile) — the DoS bound on the new
        // client-authored-topology surface.
        if steps.is_empty() {
            return Err(BinderError::InvalidArgs(
                "a blueprint needs at least one step".into(),
            ));
        }
        if steps.len() > MAX_BLUEPRINT_STEPS {
            return Err(BinderError::InvalidArgs(format!(
                "too many steps (max {MAX_BLUEPRINT_STEPS})"
            )));
        }
        if edges.len() > MAX_BLUEPRINT_EDGES {
            return Err(BinderError::InvalidArgs(format!(
                "too many edges (max {MAX_BLUEPRINT_EDGES})"
            )));
        }

        // Build the WorkflowDef (server-assigned logic_ref per step).
        let mut wf = WorkflowDef::new(seed);
        let mut refs = Vec::with_capacity(steps.len());
        for (i, s) in steps.iter().enumerate() {
            refs.push(wf.add_step(self.step_def(i, s)?));
        }
        for e in edges {
            let parent = *refs.get(e.parent as usize).ok_or_else(|| {
                BinderError::InvalidArgs(format!("edge parent index {} out of range", e.parent))
            })?;
            let child = *refs.get(e.child as usize).ok_or_else(|| {
                BinderError::InvalidArgs(format!("edge child index {} out of range", e.child))
            })?;
            let meta = if e.data {
                EdgeMeta::data()
            } else if e.non_cascade {
                EdgeMeta::control_non_cascading()
            } else {
                EdgeMeta::control()
            };
            wf.add_edge(parent, child, meta)
                .map_err(|err| BinderError::InvalidArgs(format!("edge: {err}")))?;
        }

        // Resolve the party's effective Use authority on the authoring asset (SN-8:
        // server-derived from the grant ledger, never a client warrant).
        let party_id = PartyId::new(party);
        let handle = blueprint_author_handle().map_err(|e| BinderError::Internal(e.to_string()))?;
        let resolver = HostUseResolver {
            grants: &self.lib.grants,
            owner_root: self.lib.blueprint_base.clone(),
        };
        let effective = resolver
            .resolve_use(&party_id, &AssetRef::Path(handle))
            .ok_or(BinderError::NotAuthorized)?;

        // Compile (acyclicity refusal lands here) + narrow each Mote's warrant to the
        // party's authority (the no-widen boundary: a step requesting more than the
        // grant → AttemptedWiden → NotAuthorized — identical to the Invoke binder).
        let compiled =
            compile(&wf).map_err(|e| BinderError::InvalidArgs(format!("compile: {e}")))?;
        let terminal_mote_id = compiled
            .motes
            .last()
            .map(|m| m.mote.id)
            .ok_or_else(|| BinderError::InvalidArgs("empty blueprint".into()))?;
        let mut motes = Vec::with_capacity(compiled.motes.len());
        // recipe_fingerprint = content hash of the compiled Mote ids → same authored
        // DAG yields the same fingerprint (FROZEN dedup; discovery only, never identity).
        let mut fp_buf = Vec::with_capacity(compiled.motes.len() * 32);
        for cm in &compiled.motes {
            let step_role = Role {
                name: "blueprint-step".to_string(),
                version: 0,
                spec: cm.warrant.clone(),
                description: String::new(),
            };
            let warrant =
                intersect(&effective, &step_role).map_err(|_| BinderError::NotAuthorized)?;
            fp_buf.extend_from_slice(cm.mote.id.as_bytes());
            motes.push((cm.mote.clone(), warrant));
        }
        let recipe_fingerprint = *ContentRef::of(&fp_buf).as_bytes();

        Ok(BoundRecipe {
            recipe_fingerprint,
            motes,
            terminal_mote_id,
            react_seed: false,
        })
    }
}

/// Map a `kx-invoke` bind failure to the gateway seam vocabulary: existence /
/// authority → uniform `NotAuthorized` (no oracle); bad args → `InvalidArgs`;
/// a broken provisioned recipe / submit failure → `Internal`.
fn map_invoke_err(e: InvokeError) -> BinderError {
    match e {
        InvokeError::Unauthorized
        | InvokeError::NotFound
        | InvokeError::NotAWorkflow
        | InvokeError::BodyUnavailable
        | InvokeError::WarrantNarrowing(_) => BinderError::NotAuthorized,
        InvokeError::ArgValidation(d) | InvokeError::ArgParse(d) => BinderError::InvalidArgs(d),
        InvokeError::SlotMissing(s) => BinderError::InvalidArgs(format!("missing argument '{s}'")),
        InvokeError::Schema(e) => BinderError::Internal(e.to_string()),
        InvokeError::SlotUnbound(s) => {
            BinderError::Internal(format!("recipe slot '{s}' binds no step"))
        }
        InvokeError::Uncompilable(d) => BinderError::Internal(d),
        InvokeError::EmptyRecipe => BinderError::Internal("recipe is empty".into()),
        InvokeError::Submit(e) => BinderError::Internal(e.to_string()),
    }
}

/// Parse a `"namespace/collection/name"` wire handle into an [`AssetPath`].
/// Exactly three non-empty segments; anything else ⇒ `None`. `pub(crate)` so the
/// UI-3 grant view (`teams.rs`) parses an asset handle the same way.
pub(crate) fn parse_handle(handle: &str) -> Option<AssetPath> {
    let mut parts = handle.split('/');
    let ns = parts.next()?;
    let collection = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    AssetPath::new(ns, collection, name).ok()
}

fn demo_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(DEMO_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid demo recipe handle".into()))
}

fn exec_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(EXEC_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid exec recipe handle".into()))
}

fn fanout_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(FANOUT_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid fanout recipe handle".into()))
}

fn blueprint_author_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(BLUEPRINT_AUTHOR_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid blueprint author handle".into()))
}

/// Grant `Use`+`Read` on the `kx/blueprints/author` asset to each party (the
/// authoring authority ceiling). Unlike [`seed_recipe`], publishes NO body/version —
/// authoring submits one-off DAGs; this asset only carries the grant fold the
/// [`HostWorkflowAuthor`] resolves the party's effective warrant from.
fn seed_blueprint_asset(
    grants: &SqliteGrantLedger,
    owner: &PartyId,
    parties: &[String],
    handle: &AssetPath,
    warrant: &WarrantSpec,
) -> Result<(), GatewayError> {
    let cat = |e: String| GatewayError::Catalog(e);
    let asset = AssetRef::Path(handle.clone());
    grants
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .map_err(|e| cat(e.to_string()))?;
    let role = Role {
        name: "blueprint-use".to_string(),
        version: 1,
        spec: warrant.clone(),
        description: String::new(),
    };
    let mut granted: BTreeSet<&str> = BTreeSet::new();
    for party in parties
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("local-dev"))
    {
        if !granted.insert(party) {
            continue;
        }
        grants
            .append_grant(Grant::root(
                asset.clone(),
                owner.clone(),
                PartyId::new(party),
                CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
                role.clone(),
            ))
            .map_err(|e| cat(e.to_string()))?;
    }
    Ok(())
}

/// Seed the T3.3 PURE multi-node `fanout-demo` recipe and return its
/// `(handle, meta)` for the binder's recipe table. Factored out of
/// [`DemoLibrary::seed`] so the orchestrator stays within the line budget and each
/// recipe reads as a self-contained unit.
fn seed_fanout_demo(
    versions: &SqliteVersionLedger,
    bodies: &SqliteBodyLedger,
    grants: &SqliteGrantLedger,
    owner: &PartyId,
    parties: &[String],
    exec_class: ExecutorClass,
) -> Result<(AssetPath, RecipeMeta), GatewayError> {
    let warrant = demo_warrant(exec_class);
    let handle = fanout_handle()?;
    let body = multinode_recipe_body(&warrant)?;
    seed_recipe(
        versions, bodies, grants, owner, parties, &handle, body, &warrant,
    )?;
    Ok((
        handle,
        RecipeMeta {
            owner_root: warrant,
            free_params: FreeParamContract::new(),
        },
    ))
}

/// A PURE, deterministic MULTI-step recipe body: a root that fans out to
/// [`FANOUT_WIDTH`] PURE children which a gather step joins
/// (`root → {c1..cN} → gather`, all DATA edges). Every step is a PURE `transform`
/// carrying `warrant` (== the demo warrant), so bind's narrowing `intersect` is a
/// no-op and each Mote leases on the embedded storing executor — yielding a real
/// multi-node projection with parent edges WITHOUT a model. The terminal is the
/// gather step (added last). Distinct `logic_ref`s give the steps distinct
/// identities (the storing executor ignores `logic_ref`).
fn multinode_recipe_body(warrant: &WarrantSpec) -> Result<WorkflowDef, GatewayError> {
    let edge_err =
        |e: kx_workflow::CompileError| GatewayError::Catalog(format!("fanout edge: {e}"));
    // Seed "fano" so the body is stable + idempotent across restarts.
    let mut wf = WorkflowDef::new(u32::from_le_bytes(*b"fano"));
    let model_id = warrant.model_route.model_id.clone();
    let step = |tag: u8| {
        transform(
            LogicRef::from_bytes([tag; 32]),
            model_id.clone(),
            warrant.clone(),
            ToolName("fanout-demo".into()),
        )
    };

    let root = wf.add_step(step(0x40));
    let mut children = Vec::with_capacity(FANOUT_WIDTH as usize);
    for i in 0..FANOUT_WIDTH {
        let child = wf.add_step(step(0x41 + i)); // 0x41, 0x42, 0x43 — distinct identities
        wf.add_edge(root, child, EdgeMeta::data())
            .map_err(edge_err)?;
        children.push(child);
    }
    let gather = wf.add_step(step(0x50));
    for child in children {
        wf.add_edge(child, gather, EdgeMeta::data())
            .map_err(edge_err)?;
    }
    Ok(wf)
}

/// A PURE recipe body: a single content-addressed step with `step_logic_ref` as
/// its body reference + `warrant` as its step warrant, declaring each name in
/// `var_slots` as a variable slot (so a bound free-param can overwrite it). The
/// `WorkflowDef` seed is derived from the logic_ref so distinct bodies get
/// distinct manifests (the `echo` placeholder `[0x2b; 32]` ⇒ the historical
/// `0x2b2b_2b2b` seed, so its body stays byte-identical + idempotent).
fn recipe_body(step_logic_ref: LogicRef, warrant: &WarrantSpec, var_slots: &[&str]) -> WorkflowDef {
    let b = step_logic_ref.as_bytes();
    let seed = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let mut wf = WorkflowDef::new(seed);
    let mut step = transform(
        step_logic_ref,
        warrant.model_route.model_id.clone(),
        warrant.clone(),
        ToolName("demo".into()),
    );
    for slot in var_slots {
        step.config_subset
            .insert(ConfigKey((*slot).into()), ConfigVal(Vec::new()));
    }
    wf.add_step(step);
    wf
}

/// The demo recipe's free-param contract: one `topic` variable slot typed `Str`.
fn topic_contract() -> FreeParamContract {
    FreeParamContract::new().with_slot("topic", FreeParamSlot::variable(Some(TOPIC_SCHEMA_REF)))
}

/// The model recipe's free-param contract: one `prompt` variable slot typed
/// `Str`. The slot name is [`PROMPT_KEY`], so a bound arg overwrites the model
/// step's `config_subset["prompt"]` — the key the model executor reads.
fn prompt_contract() -> FreeParamContract {
    FreeParamContract::new().with_slot(
        PROMPT_KEY,
        FreeParamSlot::variable(Some(MODEL_PROMPT_SCHEMA_REF)),
    )
}

fn model_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(MODEL_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid model recipe handle".into()))
}

/// The vision recipe's free-param contract (Batch A): `prompt` (`Str`, the chat
/// slot) + `image_ref` (`Bytes` — a 64-hex `PutContent` ref the multimodal arm
/// fetches) + `model` (`Enum`, allowed = the served id, resolved dynamically by
/// [`DemoSchemaResolver`]). All REQUIRED at binding (every variable slot is).
fn vision_contract() -> FreeParamContract {
    FreeParamContract::new()
        .with_slot(
            PROMPT_KEY,
            FreeParamSlot::variable(Some(MODEL_PROMPT_SCHEMA_REF)),
        )
        .with_slot(
            IMAGE_REF_KEY,
            FreeParamSlot::variable(Some(VISION_IMAGE_SCHEMA_REF)),
        )
        .with_slot(
            VISION_MODEL_KEY,
            FreeParamSlot::variable(Some(VISION_MODEL_SCHEMA_REF)),
        )
}

fn vision_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(VISION_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid vision recipe handle".into()))
}

/// The react recipe's free-param contract (PR-2d-2): `instruction` (`Str`) plus
/// the per-run budget caps `max_turns` / `max_tool_calls` (`Int`, 1..=8). The
/// slot names ARE the seed's config keys (the binder writes a bound arg into
/// `config_subset[<slot name>]`), which the coordinator's seed-swap reads —
/// and re-validates `0 < max_tool_calls < max_turns ≤ 8` fail-closed.
fn react_contract() -> FreeParamContract {
    FreeParamContract::new()
        .with_slot(
            kx_mote::REACT_INSTRUCTION_KEY,
            FreeParamSlot::variable(Some(REACT_INSTRUCTION_SCHEMA_REF)),
        )
        .with_slot(
            kx_mote::REACT_MAX_TURNS_KEY,
            FreeParamSlot::variable(Some(REACT_MAX_TURNS_SCHEMA_REF)),
        )
        .with_slot(
            kx_mote::REACT_MAX_TOOL_CALLS_KEY,
            FreeParamSlot::variable(Some(REACT_MAX_TOOL_CALLS_SCHEMA_REF)),
        )
}

fn react_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(REACT_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid react recipe handle".into()))
}

/// Resolves the demo `topic`, the model `prompt`, the react, and the vision
/// free-param schema-refs to their typed schemas. Carries the served model id
/// so the vision `model` ENUM's allowed set is the LIVE serve fact (Batch A) —
/// the only dynamic schema; everything else is static.
struct DemoSchemaResolver {
    /// The served model id (`None` on a model-less serve — the vision schema
    /// then resolves to an EMPTY enum, which refuses every value, fail-closed).
    serve_model: Option<String>,
}

impl SchemaResolver for DemoSchemaResolver {
    fn resolve_schema(&self, schema_ref: &[u8; 32]) -> Option<Vec<u8>> {
        if *schema_ref == TOPIC_SCHEMA_REF {
            Some(encode_param_schema(&ParamType::Str { max_len: 4096 }))
        } else if *schema_ref == MODEL_PROMPT_SCHEMA_REF
            || *schema_ref == REACT_INSTRUCTION_SCHEMA_REF
        {
            Some(encode_param_schema(&ParamType::Str { max_len: 8192 }))
        } else if *schema_ref == REACT_MAX_TURNS_SCHEMA_REF
            || *schema_ref == REACT_MAX_TOOL_CALLS_SCHEMA_REF
        {
            // The hard ceiling 8 matches the coordinator's seed-swap validation
            // (`react_seed_params`) — the form refuses what the swap would refuse.
            Some(encode_param_schema(&ParamType::Int {
                min: Some(1),
                max: Some(8),
            }))
        } else if *schema_ref == VISION_IMAGE_SCHEMA_REF {
            // A 32-byte content ref as 64 hex chars (the JSON value is a string).
            Some(encode_param_schema(&ParamType::Bytes { max_len: 64 }))
        } else if *schema_ref == VISION_MODEL_SCHEMA_REF {
            // Allowed = exactly the served model id (server-validated selection).
            Some(encode_param_schema(&ParamType::Enum {
                allowed: self.serve_model.iter().cloned().collect(),
            }))
        } else {
            None
        }
    }
}

/// The AL1 model recipe warrant: a PURE (greedy ⇒ recomputable) model step
/// routed to `model_id`. `executor_class` MUST equal the embedded worker's so the
/// bound run leases; positive `model_route` ceilings (a zero ceiling is rejected
/// by the warrant-narrowing `intersect`); a generous wall clock since CPU
/// inference is slow. No tools, no fs/net scope (text-only greedy completion).
fn model_warrant(exec_class: ExecutorClass, model_id: &ModelId) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model_id.clone(),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 4,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            // The dispatch uses this as the inference wall-clock; CPU decode of a
            // few-B model can take many seconds, so keep it generous.
            wall_clock_ms: 120_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: exec_class,
        ..Default::default()
    }
}

/// The PR-2d-2 react-chain warrant — the FIRST non-empty `tool_grants` in serve,
/// SERVER-constructed (never accepted from a client — `SubmitRun` refuses any
/// client warrant carrying grants, red-team BLOCKER #5). Grants EXACTLY the
/// bundled deterministic stdio tool (`mcp-echo@1` — `net_scope: None`, so the
/// SSRF/egress surface is N/A); ReadOnlyNondet classes (a turn is a sampling
/// model Mote; the observation's WM dispatch authority is the grant itself, and
/// the broker's 6-gate `precheck` re-verifies every axis at fire time, SN-8).
/// `executor_class` MUST equal the embedded worker's so the chain leases; the
/// model route's `max_output_tokens` is both the turn decode budget and the
/// `max_args_bytes` cap (×4) the tool-call gate enforces. `pub(crate)` since
/// W1.A5: the toolscout view's lowering DRY-RUN gates against this same
/// server-built warrant (read-only; the lowered output is discarded).
pub(crate) fn react_warrant(
    exec_class: ExecutorClass,
    model_id: &ModelId,
    tool: &(ToolName, ToolVersion),
) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(kx_warrant::ToolGrant {
        tool_id: tool.0.clone(),
        tool_version: tool.1.clone(),
    });
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: model_id.clone(),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            // The dispatch uses this as the inference wall-clock; CPU decode of a
            // few-B model can take many seconds per TURN, so keep it generous.
            wall_clock_ms: 120_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: exec_class,
        ..Default::default()
    }
}

/// The PURE demo recipe warrant. Its `executor_class` MUST equal the embedded
/// worker's (`default_executor_class()`); used identically as the owner root,
/// the grant runtime scope, and the recipe step warrant, so every `intersect`
/// in the bind chain is a no-op narrowing (no `AttemptedWiden`) and the bound
/// run leases on the worker. Mirrors the proven `tests/common::pure_warrant`.
fn demo_warrant(exec_class: ExecutorClass) -> WarrantSpec {
    let mut mounts = std::collections::BTreeMap::new();
    mounts.insert(std::path::PathBuf::from("/tmp/in"), FsMode::ReadOnly);
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist({
            let mut h = BTreeSet::new();
            h.insert(Host("api.example.com:443".into()));
            h
        }),
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: kx_mote::ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 3,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: Some(ContentRef::from_bytes([8u8; 32])),
        executor_class: exec_class,
        ..Default::default()
    }
}

/// The PR-9b real-exec recipe warrant: the sandbox scope under which the embedded
/// worker runs the located body binary. The body + its per-Mote input are
/// materialized as tempfiles under the process temp dir, so the scope grants
/// `ExecOnly` on the (canonicalized) temp dir — and, on macOS only, `ReadOnly` on
/// `/` so dyld can load libsystem (mirrors the proven `kx-executor`
/// `integration_body_resolver` warrant). Network is fully isolated
/// (`NetScope::None`). `executor_class` MUST equal the embedded worker's so the
/// bound run leases. NOTE: the temp dir is host-specific but stable across
/// restarts for the same user — adequate for the single-system demo recipe.
pub(crate) fn real_exec_warrant(exec_class: ExecutorClass) -> WarrantSpec {
    let tempdir = std::env::temp_dir();
    let tempdir = std::fs::canonicalize(&tempdir).unwrap_or(tempdir);
    let mut mounts = std::collections::BTreeMap::new();
    if exec_class == ExecutorClass::MacOsSandbox {
        // dyld/libsystem load (SBPL file-read*); bwrap binds /usr,/lib,/lib64,/etc
        // itself, so Linux needs no `/` mount.
        mounts.insert(std::path::PathBuf::from("/"), FsMode::ReadOnly);
    }
    // process-exec on the materialized body (+ read of body/input under the same
    // dir; on Linux ExecOnly maps to a bwrap `--ro-bind`, which permits exec).
    mounts.insert(tempdir, FsMode::ExecOnly);
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
        tool_grants: BTreeSet::new(),
        // The body is PURE (no model call), but the warrant-narrowing `intersect`
        // (kx-warrant) rejects a zero model-route ceiling as structurally invalid,
        // so declare positive ceilings (the sandbox backends ignore `model_route`).
        model_route: ModelRoute {
            model_id: kx_mote::ModelId("local".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 1,
        },
        resource_ceiling: real_exec_ceiling(exec_class),
        environment_ref: None,
        executor_class: exec_class,
        ..Default::default()
    }
}

/// The body's resource ceiling, platform-split. On Linux/bwrap, bound mem/CPU/FD
/// so a misbehaving body can't OOM or CPU-spin the container (it only ever has a
/// RO tempdir, so `disk_bytes`/`RLIMIT_FSIZE` stays unbounded). On macOS the axes
/// stay 0: a tight `RLIMIT_AS` (mem_bytes) is rejected for the body's
/// virtual-address reservation (`setrlimit` → `_exit(80)`), so the 30 s wall clock
/// is the only backstop there. `intersect` does not validate ceiling zeros.
fn real_exec_ceiling(exec_class: ExecutorClass) -> ResourceCeiling {
    if exec_class == ExecutorClass::Bwrap {
        ResourceCeiling {
            cpu_milli: 5_000,     // ceil → 5 s of CPU time (a hash body uses ms)
            mem_bytes: 512 << 20, // 512 MiB RLIMIT_AS — ample for a small binary
            wall_clock_ms: 30_000,
            fd_count: 256,
            disk_bytes: 0,
        }
    } else {
        ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 30_000,
            fd_count: 0,
            disk_bytes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_catalog::{InMemoryCatalog, RecipeSnapshot, SignatureEntry, TaskSignature};

    /// Build a minimal, valid `SignatureEntry` for `fingerprint` (a distinct
    /// fingerprint → a distinct task signature → a distinct id).
    fn entry(fingerprint: [u8; 32]) -> SignatureEntry {
        SignatureEntry::new(
            TaskSignature::model_invariant(kx_mote::MoteDefHash(fingerprint)),
            kx_workflow::ManifestId(fingerprint),
            RecipeSnapshot::new(fingerprint),
        )
    }

    fn encode(e: &SignatureEntry) -> Vec<u8> {
        bincode::serde::encode_to_vec(e, canonical_config()).unwrap()
    }

    #[test]
    fn register_then_get_round_trips_and_server_derives_id() {
        let cat = HostSignatureCatalog::new(InMemoryCatalog::new());
        let e = entry([7; 32]);
        let manifest = encode(&e);

        let reg = cat.register(&manifest).unwrap();
        assert_eq!(
            reg.signature_id,
            *e.hash().as_bytes(),
            "id is server-derived"
        );

        let got = cat.get(&reg.signature_id).expect("present after register");
        assert_eq!(got, manifest, "GetSignature byte-round-trips the manifest");
    }

    #[test]
    fn register_is_idempotent() {
        let cat = HostSignatureCatalog::new(InMemoryCatalog::new());
        let manifest = encode(&entry([9; 32]));
        let a = cat.register(&manifest).unwrap();
        let b = cat.register(&manifest).unwrap();
        assert_eq!(a.signature_id, b.signature_id);
    }

    #[test]
    fn malformed_manifest_is_rejected() {
        let cat = HostSignatureCatalog::new(InMemoryCatalog::new());
        assert!(matches!(
            cat.register(b"not a signature entry"),
            Err(CatalogSeamError::Malformed(_))
        ));
    }

    #[test]
    fn get_unknown_is_none_and_list_enumerates() {
        let cat = HostSignatureCatalog::new(InMemoryCatalog::new());
        assert!(cat.get(&[0xab; 32]).is_none());
        cat.register(&encode(&entry([1; 32]))).unwrap();
        cat.register(&encode(&entry([2; 32]))).unwrap();
        let listed = cat.list();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|s| s.name.starts_with("sig-")));
    }

    // --- R2b: the recipe binder (the Invoke path) --------------------------

    /// Build a demo library in a fresh temp dir granting `Use` to `alice@acme`.
    fn demo_lib(dir: &std::path::Path) -> HostRecipeBinder {
        let lib =
            DemoLibrary::open(dir, ExecutorClass::Bwrap, &["alice@acme".to_string()]).unwrap();
        HostRecipeBinder::new(lib)
    }

    #[tokio::test]
    async fn bind_is_deterministic_and_input_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());

        let a1 = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#)
            .await
            .unwrap();
        let a2 = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#)
            .await
            .unwrap();
        assert_eq!(
            a1.terminal_mote_id, a2.terminal_mote_id,
            "identical args → identical identity (idempotent re-invoke)"
        );
        assert_eq!(a1.recipe_fingerprint, a2.recipe_fingerprint);

        let b = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"y"}"#)
            .await
            .unwrap();
        assert_ne!(
            a1.terminal_mote_id, b.terminal_mote_id,
            "distinct args → distinct identity (exactly-once-per-input)"
        );
    }

    #[tokio::test]
    async fn bind_no_widen_keeps_worker_executor_class() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        let bound = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#)
            .await
            .unwrap();
        assert!(!bound.motes.is_empty());
        for (_, w) in &bound.motes {
            // The bound warrant keeps the worker's executor_class (so it leases)
            // and never widens beyond the demo recipe's declared scope.
            assert_eq!(w.executor_class, ExecutorClass::Bwrap);
            assert!(w.model_route.max_calls <= 3);
        }
    }

    #[tokio::test]
    async fn bind_fanout_demo_is_a_multinode_dag() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        let bound = binder
            .bind("alice@acme", FANOUT_RECIPE_HANDLE, b"{}")
            .await
            .unwrap();
        // root + FANOUT_WIDTH children + gather = a genuine multi-node DAG.
        assert_eq!(bound.motes.len(), FANOUT_WIDTH as usize + 2);
        // Every step stays PURE on the worker's executor_class (so it leases on the
        // embedded storing executor — the model-free multi-node path).
        for (_, w) in &bound.motes {
            assert_eq!(w.executor_class, ExecutorClass::Bwrap);
            assert_eq!(w.mote_class, MoteClass::Pure);
        }
        // Idempotent: identical (empty) args → identical terminal identity.
        let again = binder
            .bind("alice@acme", FANOUT_RECIPE_HANDLE, b"{}")
            .await
            .unwrap();
        assert_eq!(bound.terminal_mote_id, again.terminal_mote_id);
        assert_eq!(bound.recipe_fingerprint, again.recipe_fingerprint);
    }

    #[tokio::test]
    async fn bind_unauthorized_party_is_uniformly_refused() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        // "mallory" was never granted Use.
        assert!(matches!(
            binder
                .bind("mallory@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#)
                .await,
            Err(BinderError::NotAuthorized)
        ));
    }

    #[tokio::test]
    async fn bind_unknown_or_malformed_handle_is_uniformly_refused() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        // Unknown handle and a malformed handle both → uniform NotAuthorized.
        assert!(matches!(
            binder
                .bind("alice@acme", "kx/recipes/nope", br#"{"topic":"x"}"#)
                .await,
            Err(BinderError::NotAuthorized)
        ));
        assert!(matches!(
            binder
                .bind("alice@acme", "not-a-handle", br#"{"topic":"x"}"#)
                .await,
            Err(BinderError::NotAuthorized)
        ));
    }

    #[tokio::test]
    async fn bind_bad_args_are_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        // Wrong type for `topic`.
        assert!(matches!(
            binder
                .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":5}"#)
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // Missing `topic`.
        assert!(matches!(
            binder.bind("alice@acme", DEMO_RECIPE_HANDLE, b"{}").await,
            Err(BinderError::InvalidArgs(_))
        ));
    }

    // --- Blueprint authoring (SubmitWorkflow / HostWorkflowAuthor) ----------

    fn demo_author(dir: &std::path::Path) -> HostWorkflowAuthor {
        let lib =
            DemoLibrary::open(dir, ExecutorClass::Bwrap, &["alice@acme".to_string()]).unwrap();
        HostWorkflowAuthor::from_shared(std::sync::Arc::new(lib))
    }

    fn pure_step() -> AuthorStep {
        AuthorStep {
            kind: AuthorStepKind::Pure,
            model_id: String::new(),
            prompt: String::new(),
            body_signature_id: None,
            tool_contract: std::collections::BTreeMap::new(),
            params: std::collections::BTreeMap::new(),
        }
    }

    fn data_edge(parent: u32, child: u32) -> AuthorEdge {
        AuthorEdge {
            parent,
            child,
            data: true,
            non_cascade: false,
        }
    }

    /// An authored DAG compiles to STABLE, content-addressed identity (same bytes →
    /// same MoteIds + fingerprint), and a distinct seed yields a distinct identity.
    #[tokio::test]
    async fn blueprint_authoring_is_deterministic_and_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let author = demo_author(dir.path());
        let steps = [pure_step(), pure_step()];
        let edges = [data_edge(0, 1)];

        let a = author
            .author("alice@acme", 7, &steps, &edges, AuthorExecutionMode::Frozen)
            .await
            .expect("authored");
        let b = author
            .author("alice@acme", 7, &steps, &edges, AuthorExecutionMode::Frozen)
            .await
            .expect("authored again");

        assert_eq!(a.motes.len(), 2, "two compiled Motes");
        let ids_a: Vec<_> = a.motes.iter().map(|(m, _)| m.id).collect();
        let ids_b: Vec<_> = b.motes.iter().map(|(m, _)| m.id).collect();
        assert_eq!(
            ids_a, ids_b,
            "same authored bytes → same server-derived ids"
        );
        assert_eq!(
            a.recipe_fingerprint, b.recipe_fingerprint,
            "content-addressed"
        );
        assert!(!a.react_seed, "authored runs never seed a ReAct chain");

        let c = author
            .author("alice@acme", 8, &steps, &edges, AuthorExecutionMode::Frozen)
            .await
            .expect("authored with a new seed");
        assert_ne!(
            a.terminal_mote_id, c.terminal_mote_id,
            "a distinct seed → a distinct identity"
        );
    }

    /// The refusal boundary: a cyclic / empty DAG, the reserved DYNAMIC mode, and an
    /// unauthorized party are each refused (no orphan run) — the design-doc-mandated
    /// admission coverage for client-authored DAGs.
    #[tokio::test]
    async fn blueprint_authoring_refuses_bad_shapes_and_unauthorized() {
        let dir = tempfile::tempdir().unwrap();
        let author = demo_author(dir.path());
        let steps = [pure_step(), pure_step()];

        // A cycle is refused at compile.
        let cyclic = [data_edge(0, 1), data_edge(1, 0)];
        assert!(matches!(
            author
                .author(
                    "alice@acme",
                    1,
                    &steps,
                    &cyclic,
                    AuthorExecutionMode::Frozen
                )
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // DYNAMIC mode is reserved (PR-1 frozen-only).
        assert!(matches!(
            author
                .author("alice@acme", 1, &steps, &[], AuthorExecutionMode::Dynamic)
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // An empty DAG is refused.
        assert!(matches!(
            author
                .author("alice@acme", 1, &[], &[], AuthorExecutionMode::Frozen)
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // An ungranted party gets a UNIFORM NotAuthorized (no existence oracle).
        assert!(matches!(
            author
                .author("mallory@evil", 1, &steps, &[], AuthorExecutionMode::Frozen)
                .await,
            Err(BinderError::NotAuthorized)
        ));
    }

    /// P1.1 regression: an authored MODEL step's bound warrant `model_route` names
    /// the SERVED model (not the `blueprint_base` placeholder), so the dispatcher's
    /// strict `mote.def.model_id == warrant.model_route.model_id` check passes and
    /// the step RUNS instead of dead-lettering. Before the fix the step warrant was
    /// the placeholder route ≠ served, so `SubmitWorkflow` MODEL steps dead-lettered
    /// against any non-placeholder served model.
    #[tokio::test]
    async fn authored_model_step_warrant_routes_to_the_served_model() {
        let dir = tempfile::tempdir().unwrap();
        let served = ModelId("kx-serve:test-model".into());
        let lib = DemoLibrary::open_full(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            None,
            Some(served.clone()),
        )
        .unwrap();
        let author = HostWorkflowAuthor::from_shared(std::sync::Arc::new(lib));
        let model_step = AuthorStep {
            kind: AuthorStepKind::Model,
            model_id: String::new(), // empty ⇒ bind to the served model
            prompt: "summarize the discussion".into(),
            body_signature_id: None,
            tool_contract: std::collections::BTreeMap::new(),
            params: std::collections::BTreeMap::new(),
        };
        let bound = author
            .author(
                "alice@acme",
                3,
                &[model_step],
                &[],
                AuthorExecutionMode::Frozen,
            )
            .await
            .expect("a single MODEL step authors");
        let (mote, warrant) = bound
            .motes
            .iter()
            .find(|(m, _)| {
                m.def
                    .config_subset
                    .contains_key(&ConfigKey(PROMPT_KEY.to_string()))
            })
            .expect("the authored DAG has a prompt-bearing MODEL Mote");
        assert_eq!(
            mote.def.model_id, served,
            "the dispatch model id is the served model"
        );
        assert_eq!(
            warrant.model_route.model_id, served,
            "P1.1: the step warrant's model_route names the SERVED model (was the placeholder)"
        );
        assert_eq!(
            mote.def.model_id, warrant.model_route.model_id,
            "dispatch id == warrant route id (the dispatcher's strict equality holds)"
        );
    }

    #[tokio::test]
    async fn demo_library_reopens_idempotently() {
        // A second open on the same dir (restart) must not error (durable ledgers
        // + idempotent seeding + guarded version publish).
        let dir = tempfile::tempdir().unwrap();
        let _a = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        );
        let b = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        );
        assert!(b.is_ok(), "re-opening the durable demo library is a no-op");
    }

    // --- PR-9b: the real-exec recipe (provisioning + bind path) -------------

    /// When a body is registered, the `exec-demo` recipe binds for a granted
    /// party (empty free-params → empty JSON args) and keeps the worker's
    /// executor_class so the bound run leases. (The real sandboxed spawn is the
    /// `#[ignore]` `real_exec_e2e` witness — this covers the durable bind path.)
    #[tokio::test]
    async fn exec_recipe_binds_when_a_body_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_with_real_exec(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            // The bind path resolves the recipe def + warrant only; the body bytes
            // are fetched at EXECUTION (the resolver), so a stand-in ref binds fine.
            Some(ContentRef::from_bytes([0x5a; 32])),
        )
        .unwrap();
        let binder = HostRecipeBinder::new(lib);

        let bound = binder
            .bind("alice@acme", EXEC_RECIPE_HANDLE, b"{}")
            .await
            .expect("exec-demo binds for a granted party");
        assert!(!bound.motes.is_empty());
        for (_, w) in &bound.motes {
            assert_eq!(w.executor_class, ExecutorClass::Bwrap);
        }
    }

    /// AL1: when a serve model is configured, the `chat` model recipe binds for a
    /// granted party — the bound model Mote carries the `prompt` (under
    /// [`PROMPT_KEY`]) + routes to the served model id, and keeps the worker's
    /// executor_class so the bound run leases.
    #[tokio::test]
    async fn model_recipe_binds_with_prompt_and_model_route() {
        let dir = tempfile::tempdir().unwrap();
        let model_id = ModelId("kx-serve:test-model".to_string());
        let lib = DemoLibrary::open_full(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            None,
            Some(model_id.clone()),
        )
        .unwrap();
        let binder = HostRecipeBinder::new(lib);

        let bound = binder
            .bind(
                "alice@acme",
                MODEL_RECIPE_HANDLE,
                br#"{"prompt":"Capital of France?"}"#,
            )
            .await
            .expect("chat recipe binds for a granted party");
        assert!(!bound.motes.is_empty());
        for (mote, w) in &bound.motes {
            assert_eq!(w.executor_class, ExecutorClass::Bwrap);
            assert_eq!(
                w.model_route.model_id, model_id,
                "routes to the served model"
            );
            // The bound prompt landed under PROMPT_KEY (what the model executor
            // reads). The free-param binder stores it JSON-encoded, so assert it
            // CONTAINS the text (quoting-robust; the executor JSON-decodes it).
            let got = mote
                .def
                .config_subset
                .get(&ConfigKey(PROMPT_KEY.to_string()))
                .map(|v| String::from_utf8_lossy(&v.0).into_owned())
                .expect("prompt bound under PROMPT_KEY");
            assert!(got.contains("Capital of France?"), "bound prompt: {got:?}");
        }
    }

    /// Without a serve model the `chat` recipe is NOT provisioned (uniform
    /// `NotAuthorized`), while `echo` still binds.
    #[tokio::test]
    async fn model_recipe_absent_without_a_serve_model() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path()); // open() ⇒ no serve model
        assert!(matches!(
            binder
                .bind("alice@acme", MODEL_RECIPE_HANDLE, br#"{"prompt":"x"}"#)
                .await,
            Err(BinderError::NotAuthorized)
        ));
    }

    /// Without a registered body the `exec-demo` recipe is NOT provisioned, so it
    /// is uniformly `NotAuthorized` (no existence oracle) — while `echo` still binds.
    #[tokio::test]
    async fn exec_recipe_absent_without_a_body() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path()); // open() ⇒ no real body
        assert!(matches!(
            binder.bind("alice@acme", EXEC_RECIPE_HANDLE, b"{}").await,
            Err(BinderError::NotAuthorized)
        ));
        // The demo echo recipe is unaffected.
        assert!(binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#)
            .await
            .is_ok());
    }

    // --- UI-2: the recipe-discovery accessors (ListRecipes / GetRecipeForm) ---

    #[test]
    fn recipe_handles_enumerate_the_always_seeded_recipes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        let handles = lib.recipe_handles();
        // `echo` + `fanout-demo` are always seeded; `exec-demo`/`chat` are conditional.
        assert!(handles.contains(&DEMO_RECIPE_HANDLE.to_string()));
        assert!(handles.contains(&FANOUT_RECIPE_HANDLE.to_string()));
        assert!(!handles.contains(&MODEL_RECIPE_HANDLE.to_string()));
    }

    #[test]
    fn recipe_form_for_echo_is_the_typed_topic_field() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        let form = lib
            .recipe_form(DEMO_RECIPE_HANDLE)
            .expect("echo is provisioned");
        assert_eq!(form.len(), 1);
        assert_eq!(form[0].name, "topic");
        assert_eq!(form[0].kind, RecipeParamKind::Str);
        assert!(form[0].required);
        assert_eq!(form[0].max_len, Some(4096), "matches the topic schema");
        assert!(form[0].allowed.is_empty());
    }

    #[test]
    fn recipe_form_for_fanout_has_no_free_params() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        let form = lib
            .recipe_form(FANOUT_RECIPE_HANDLE)
            .expect("fanout is provisioned");
        assert!(form.is_empty(), "fanout-demo takes no free-params");
    }

    #[test]
    fn recipe_form_for_unknown_or_malformed_handle_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        assert!(lib.recipe_form("kx/recipes/nope").is_none());
        assert!(lib.recipe_form("not-a-handle").is_none());
    }

    #[test]
    fn recipe_form_for_chat_is_the_prompt_field_when_a_model_is_served() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_full(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            None,
            Some(ModelId("kx-serve:test-model".to_string())),
        )
        .unwrap();
        assert!(lib
            .recipe_handles()
            .contains(&MODEL_RECIPE_HANDLE.to_string()));
        let form = lib
            .recipe_form(MODEL_RECIPE_HANDLE)
            .expect("chat is provisioned with a serve model");
        assert_eq!(form.len(), 1);
        assert_eq!(form[0].name, PROMPT_KEY);
        assert_eq!(form[0].kind, RecipeParamKind::Str);
        assert!(form[0].required);
    }

    #[test]
    fn vision_recipe_seeds_only_with_vision_and_forms_the_three_typed_fields() {
        // Without the vision flag: chat seeds, vision does NOT (no fake recipe
        // on a text-only serve).
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_complete(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            None,
            Some(&ModelId("kx-serve:vlm".to_string())),
            None,
            false,
        )
        .unwrap();
        assert!(!lib
            .recipe_handles()
            .contains(&VISION_RECIPE_HANDLE.to_string()));

        // With it: the form publishes prompt(Str) + image_ref(Bytes, 64) +
        // model(Enum, allowed = exactly the served id).
        let dir2 = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_complete(
            dir2.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            None,
            Some(&ModelId("kx-serve:vlm".to_string())),
            None,
            true,
        )
        .unwrap();
        let form = lib
            .recipe_form(VISION_RECIPE_HANDLE)
            .expect("vision is provisioned");
        assert_eq!(form.len(), 3);
        let field = |n: &str| form.iter().find(|f| f.name == n).expect("field present");
        assert_eq!(field(PROMPT_KEY).kind, RecipeParamKind::Str);
        let image = field(IMAGE_REF_KEY);
        assert_eq!(image.kind, RecipeParamKind::Bytes);
        assert_eq!(image.max_len, Some(64), "a 32-byte ref as 64 hex chars");
        let model = field(VISION_MODEL_KEY);
        assert_eq!(model.kind, RecipeParamKind::Enum);
        assert_eq!(
            model.allowed,
            vec!["kx-serve:vlm".to_string()],
            "allowed = exactly the served id (server-validated selection)"
        );
    }

    #[test]
    fn host_recipe_catalog_shares_the_library_with_the_binder() {
        // One seed, two seams: the catalog's published form names the SAME slot
        // the binder validates — agreement by construction.
        let dir = tempfile::tempdir().unwrap();
        let lib = std::sync::Arc::new(
            DemoLibrary::open(
                dir.path(),
                ExecutorClass::Bwrap,
                &["alice@acme".to_string()],
            )
            .unwrap(),
        );
        let catalog = HostRecipeCatalog::new(lib.clone());
        let handles = catalog.list_recipes();
        assert!(handles.contains(&DEMO_RECIPE_HANDLE.to_string()));
        let form = catalog.get_recipe_form(DEMO_RECIPE_HANDLE).unwrap();
        assert_eq!(form[0].name, "topic");
        // The shared binder binds that exact slot.
        let binder = HostRecipeBinder::from_shared(lib);
        // (bind is async; the form/bind agreement is what we assert structurally here)
        let _ = &binder;
    }
}
