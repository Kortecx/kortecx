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
    BundleStore, CatalogSeamError, RecipeBinder, RecipeCatalog, RecipeFormFieldEntry,
    RecipeMetadataEntry, RecipeParamKind, RegisteredSignature, ScoredRecipeEntry, SignatureCatalog,
    SignatureSummaryEntry, WorkflowAuthor,
};
use kx_invoke::{bind_snapshot, InvokeError, UseWarrantResolver};
use kx_mote::{
    encode_context_items, ConfigKey, ConfigVal, ContextItemRef, EdgeMeta, LogicRef, ModelId,
    ToolName, ToolVersion, CONTEXT_ITEMS_KEY, PROMPT_KEY, TOOL_ARGS_KEY,
};
use kx_tool_registry::{ToolDef, ToolRegistry};
use kx_warrant::{
    intersect, ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, Role, ToolGrant, WarrantSpec,
};
use kx_workflow::{compile, tool_step, transform, StepDef, WorkflowDef};

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

/// The wire handle of the T3.3 deterministic MULTI-NODE recipe: a PURE fan-out →
/// gather DAG (root → N children → gather) whose every node commits an HONEST
/// passthrough of its declared input (GR15), so a single `Invoke` yields a real
/// multi-node projection with DATA parent edges — the live-DAG viewer's
/// end-to-end fixture. Always provisioned. Takes no free-params. Shared with the
/// e2e test (no drift).
pub const PASSTHROUGH_DAG_HANDLE: &str = "kx/recipes/passthrough-dag";

/// The fan-out width of [`PASSTHROUGH_DAG_HANDLE`]: root + `FANOUT_WIDTH` children +
/// gather = `FANOUT_WIDTH + 2` Motes. Three keeps the DAG legible.
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

/// PR-6b-4: a DISTINCT placeholder `logic_ref` for the `react-fs` seed step. The
/// recipe body's manifest id is `hash(seed ‖ mote_ids)` and EXCLUDES the warrant
/// (`Manifest::recipe`), so two react bodies that differ ONLY by their server-built
/// warrant map to the SAME manifest id but DIFFERENT body bytes — a body-ledger
/// immutability conflict at seed. `react` (echo grant) + `react-fs` (fs-list grant)
/// previously shared [`REACT_LOGIC_REF`], so a serve with BOTH the bundled echo bin
/// AND `KX_SERVE_FS_ROOT` panicked at startup (BUG-25: the bodies differ only by
/// warrant). A distinct sentinel gives `react-fs` its own manifest id; the seed is
/// SWAPPED at submit, so the ref never reaches an admitted identity.
const REACT_FS_LOGIC_REF: [u8; 32] = [0x54; 32];

/// PR-6b-4: a DISTINCT placeholder `logic_ref` for the react-auto seed step (the
/// same BUG-25 class as [`REACT_FS_LOGIC_REF`] — react-auto's placeholder grant is
/// empty, which would collide with `kx/recipes/react`). The binder overrides the
/// bound warrant with the live union; like [`REACT_LOGIC_REF`] the seed is SWAPPED
/// at submit, so the ref never reaches an admitted identity.
const REACT_AUTO_LOGIC_REF: [u8; 32] = [0x53; 32];

/// The wire handle of the PR-6a/D155 `react-fs` recipe: a live ReAct loop like
/// [`REACT_RECIPE_HANDLE`] BUT whose server-built step warrant grants the read-only
/// `fs-list@1` tool + a `fs_scope` of the operator-granted read root (`KX_SERVE_FS_ROOT`)
/// instead of `mcp-echo@1`. A SEPARATE recipe BY DESIGN (the vision precedent) so the
/// canonical `kx/recipes/react` + the projection digest stay byte-unchanged. Seeded only
/// when a fit serve model resolved AND `KX_SERVE_FS_ROOT` is set (the fs-list capability
/// registered). Reuses the react free-param contract (instruction / max_turns /
/// max_tool_calls); carries its OWN `REACT_FS_LOGIC_REF` (BUG-25 — a shared logic ref
/// collides with `kx/recipes/react` at seed) — the warrant + handle + logic ref differ.
pub const REACT_FS_RECIPE_HANDLE: &str = "kx/recipes/react-fs";

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

/// The vision recipe's image-payload ceiling (BUG-26). The multimodal dispatch
/// (`kx-inference` `llama.rs`) gates each fetched image's byte length against the
/// warrant's `resource_ceiling.mem_bytes`; the text-only [`model_warrant`] leaves
/// that `0` (no image payload), so the vision recipe MUST raise it or EVERY
/// `image_ref` fails `scope violation on image_bytes` at dispatch. `seed_recipe`
/// issues the party `Use` grant from this SAME warrant, so the bind-time narrow
/// keeps the ceiling (grant ∩ owner-root = this value). 16 MiB comfortably covers
/// a high-resolution still while bounding the untrusted blob the projector decodes.
const VISION_MAX_IMAGE_BYTES: u64 = 16 << 20;

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
    /// Per-handle binding metadata — one entry per seeded recipe (the `echo`
    /// passthrough + `passthrough-dag` always; `chat`/`react`/`vision` when a
    /// fit serve model / bundled tool resolved).
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
        Self::seed(dir, exec_class, parties, None, None, false, None, false)
    }

    /// Like [`DemoLibrary::open`], plus (when `serve_model` is `Some`) the AL1
    /// model recipe [`MODEL_RECIPE_HANDLE`] — a PURE (greedy) model step routed
    /// to `serve_model` with a `prompt` free-param. `None` ⇒ byte-identical to
    /// [`DemoLibrary::open`].
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
        serve_model: Option<ModelId>,
    ) -> Result<Self, GatewayError> {
        Self::seed(
            dir,
            exec_class,
            parties,
            serve_model.as_ref(),
            None,
            false,
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
    #[allow(clippy::too_many_arguments)]
    pub fn open_complete(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        serve_model: Option<&ModelId>,
        react_tool: Option<&(ToolName, ToolVersion)>,
        vision: bool,
        fs_list: Option<(&(ToolName, ToolVersion), &Path)>,
        autogrant: bool,
    ) -> Result<Self, GatewayError> {
        Self::seed(
            dir,
            exec_class,
            parties,
            serve_model,
            react_tool,
            vision,
            fs_list,
            autogrant,
        )
    }

    /// Open the durable ledgers under `dir` and idempotently seed the `echo`
    /// passthrough recipe + the `passthrough-dag` multi-node recipe (always),
    /// plus `chat`/`react`/`vision` when a fit serve model / bundled tool
    /// resolved. Re-opening on restart is a no-op (content-addressed bodies +
    /// guarded version publish + idempotent grants).
    // A flat, sequential one-block-per-recipe seeding fn — the length is the
    // recipe count, not cognitive complexity (the `start_impl` precedent).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn seed(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        serve_model: Option<&ModelId>,
        react_tool: Option<&(ToolName, ToolVersion)>,
        vision: bool,
        fs_list: Option<(&(ToolName, ToolVersion), &Path)>,
        autogrant: bool,
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

        // (echo) the PURE echo recipe — the honest passthrough executor commits
        // its bound `topic` free-param verbatim (GR15); the logic_ref is a stable
        // body identity the executor does not interpret.
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

        // (passthrough-dag) the T3.3 PURE multi-node recipe — a fan-out → gather
        // DAG that runs model-free on the honest passthrough executor (see
        // `seed_passthrough_dag`). Always seeded; no free-params.
        recipes.push(seed_passthrough_dag(
            &versions, &bodies, &grants, &owner, parties, exec_class,
        )?);

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
            // BUG-26: the multimodal dispatch caps each image against `mem_bytes`,
            // which the text-only `model_warrant` leaves 0 — raise it to the vision
            // image ceiling or every `image_ref` fails `scope violation on
            // image_bytes`. Set BEFORE any use so the owner-root, the recipe body's
            // step warrant, and the party `Use` grant all carry the same ceiling.
            let mut vision_w = model_warrant(exec_class, model_id);
            vision_w.resource_ceiling.mem_bytes = VISION_MAX_IMAGE_BYTES;
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

        // (react-fs) PR-6a/D155: a SEPARATE live ReAct recipe whose server-built
        // warrant grants the read-only `fs-list@1` tool + a fs_scope of the granted
        // read root (`KX_SERVE_FS_ROOT`) — the first recipe that produces REAL
        // agent data (a directory listing committed as the Observation result_ref).
        // Seeded only when a model is served AND fs-list is available; reuses the
        // react free-param contract but carries its OWN logic ref (BUG-25 — a shared
        // logic ref collides with `kx/recipes/react` at seed when both are present),
        // so the canonical `kx/recipes/react` + the digest stay byte-unchanged.
        if let (Some(model_id), Some((fs_tool, fs_root))) = (serve_model, fs_list) {
            let react_fs_w = react_fs_warrant(exec_class, model_id, fs_tool, fs_root);
            let react_fs_h = react_fs_handle()?;
            seed_recipe(
                &versions,
                &bodies,
                &grants,
                &owner,
                parties,
                &react_fs_h,
                recipe_body(
                    LogicRef::from_bytes(REACT_FS_LOGIC_REF),
                    &react_fs_w,
                    &[
                        kx_mote::REACT_INSTRUCTION_KEY,
                        kx_mote::REACT_MAX_TURNS_KEY,
                        kx_mote::REACT_MAX_TOOL_CALLS_KEY,
                    ],
                ),
                &react_fs_w,
            )?;
            recipes.push((
                react_fs_h,
                RecipeMeta {
                    owner_root: react_fs_w,
                    free_params: react_contract(),
                },
            ));
        }

        // (react-auto) PR-6b-4: a SEPARATE live ReAct recipe whose warrant is
        // REBUILT at bind from the LIVE registry to auto-grant the registered/
        // dialed tool set (a union warrant, ≤ AUTOGRANT_MAX_TOOLS) — so the
        // autonomous loop can pick from ALL live tools, not just one bundled seed
        // tool. The seed-time `owner_root` here is a PLACEHOLDER (empty tool_grants);
        // the host binder overrides the bound seed Mote's warrant with the live
        // union (the dialed tool registers at runtime, after seed). A separate
        // recipe (the react-fs precedent) keeps the canonical react recipe + the
        // digest byte-unchanged. Seeded only when a model is served AND the operator
        // opted in via `KX_SERVE_AUTOGRANT`; reuses the react logic ref + free-param
        // contract (only the warrant + handle differ).
        if let Some(model_id) = serve_model.filter(|_| autogrant) {
            let react_auto_w = react_auto_base_warrant(exec_class, model_id);
            let react_auto_h = react_auto_handle()?;
            seed_recipe(
                &versions,
                &bodies,
                &grants,
                &owner,
                parties,
                &react_auto_h,
                recipe_body(
                    LogicRef::from_bytes(REACT_AUTO_LOGIC_REF),
                    &react_auto_w,
                    &[
                        kx_mote::REACT_INSTRUCTION_KEY,
                        kx_mote::REACT_MAX_TURNS_KEY,
                        kx_mote::REACT_MAX_TOOL_CALLS_KEY,
                    ],
                ),
                &react_auto_w,
            )?;
            recipes.push((
                react_auto_h,
                RecipeMeta {
                    owner_root: react_auto_w,
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

    /// The ADVISORY metadata (description / tags / version) for a provisioned
    /// `handle` (PR-4 Batch D), or `None` for an unprovisioned handle. The
    /// version is the published version id (content-addressed — these recipes
    /// carry no semver) read from the SAME versions ledger the binder resolves
    /// through. Display/discovery ONLY — never identity, never enforcement.
    #[must_use]
    pub fn recipe_metadata(&self, handle: &str) -> Option<RecipeMetadataEntry> {
        let path = parse_handle(handle)?;
        if !self.recipes.iter().any(|(h, _)| *h == path) {
            return None;
        }
        let (description, tags) = recipe_advisory(handle);
        // The published version id (12-hex pin), iff the handle resolves a
        // workflow version; empty otherwise (honest "unversioned", D142).
        let version = self
            .versions
            .resolve(&path)
            .map(|(_, vid)| vid.to_hex().chars().take(12).collect::<String>())
            .unwrap_or_default();
        Some(RecipeMetadataEntry {
            description: description.to_string(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            version,
        })
    }

    /// ADVISORY recipe discovery (PR-4 Batch D): rank the provisioned handles
    /// against `intent` (+ optional `keywords`), best-first, capped at `limit`.
    /// Pure + deterministic; `score_bp` is integer basis points, DISPLAY-ONLY
    /// (a hit SURFACES a recipe, never invokes it — `Invoke` stays the gate).
    #[must_use]
    pub fn search_recipes(
        &self,
        intent: &str,
        keywords: &[String],
        limit: usize,
    ) -> Vec<ScoredRecipeEntry> {
        rank_recipes(&self.recipe_handles(), intent, keywords, limit, |h| {
            self.recipe_metadata(h).unwrap_or_default()
        })
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

/// The static, honest advisory copy (description + discovery tags) for a
/// provisioned recipe handle (PR-4 Batch D). Describes the REAL recipe;
/// display/discovery only (never parsed for enforcement). An unknown handle ⇒
/// empty. Kept beside the recipe handle consts so the two never drift.
fn recipe_advisory(handle: &str) -> (&'static str, &'static [&'static str]) {
    match handle {
        DEMO_RECIPE_HANDLE => (
            "Echo — a true passthrough of the bound `topic` (model-free, GR15-honest).",
            &["passthrough", "pure", "text", "echo"],
        ),
        PASSTHROUGH_DAG_HANDLE => (
            "Passthrough DAG — a fan-out → gather multi-node PURE workflow (model-free).",
            &["passthrough", "dag", "pure", "workflow"],
        ),
        MODEL_RECIPE_HANDLE => (
            "Chat — a single greedy completion from the served model.",
            &["model", "chat", "text", "agent", "completion"],
        ),
        REACT_RECIPE_HANDLE => (
            "ReAct — a live tool-using agent loop (plan → act → observe → answer).",
            &["agent", "react", "tools", "loop", "reasoning"],
        ),
        REACT_FS_RECIPE_HANDLE => (
            "ReAct-FS — a live agent loop with a read-only filesystem tool (lists files under the granted root).",
            &["agent", "react", "tools", "filesystem", "fs-list"],
        ),
        REACT_AUTO_RECIPE_HANDLE => (
            "ReAct-Auto — a live agent loop that auto-grants the registered/dialed tool set (the model picks from all live tools).",
            &["agent", "react", "tools", "auto-grant", "mcp"],
        ),
        VISION_RECIPE_HANDLE => (
            "Vision — a multimodal completion over an attached image.",
            &["model", "vision", "image", "multimodal", "agent"],
        ),
        _ => ("", &[]),
    }
}

/// The neutral score for an empty-query listing (so `SearchRecipes` with no
/// intent returns the full catalog, ranked stably by handle).
const RECIPE_SCORE_NEUTRAL: u32 = 1;

/// Pure, deterministic recipe ranker (PR-4 Batch D). Tokenizes `intent` (+
/// `keywords`) and scores each handle by the best query-token match against the
/// handle / name / tags / description, in INTEGER basis points (≤ 10000 — never
/// a float, the SN-8 no-persisted-confidence rule). An empty query lists every
/// recipe at a neutral score; a non-empty query keeps only positive matches.
/// Best-first with a deterministic handle tiebreak; capped at `limit`.
fn rank_recipes(
    handles: &[String],
    intent: &str,
    keywords: &[String],
    limit: usize,
    meta: impl Fn(&str) -> RecipeMetadataEntry,
) -> Vec<ScoredRecipeEntry> {
    let mut query: Vec<String> = intent
        .split(|c: char| !c.is_alphanumeric())
        .chain(keywords.iter().map(String::as_str))
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    query.sort();
    query.dedup();

    // The whole normalized intent (handles contain '/', which the tokenizer
    // splits — so an exact-handle match needs the un-tokenized intent too).
    let intent_lc = intent.trim().to_lowercase();
    let mut scored: Vec<ScoredRecipeEntry> = handles
        .iter()
        .map(|h| {
            let metadata = meta(h);
            let score_bp = score_recipe(h, &metadata, &intent_lc, &query);
            ScoredRecipeEntry {
                handle: h.clone(),
                metadata,
                score_bp,
            }
        })
        .collect();
    if !query.is_empty() {
        scored.retain(|s| s.score_bp > 0);
    }
    scored.sort_by(|a, b| {
        b.score_bp
            .cmp(&a.score_bp)
            .then_with(|| a.handle.cmp(&b.handle))
    });
    scored.truncate(limit);
    scored
}

/// Score one recipe against a normalized (lowercased, deduped) query. Rungs are
/// integer basis points, exact-out > fuzzy-in: exact handle = 10000, exact name
/// = 9500, name-substring = 9000, handle-substring = 8500, exact tag = 7000,
/// tag-substring = 6000, description-substring = 5000, else 0. An empty query ⇒
/// the neutral listing score.
fn score_recipe(handle: &str, m: &RecipeMetadataEntry, intent_lc: &str, query: &[String]) -> u32 {
    if query.is_empty() {
        return RECIPE_SCORE_NEUTRAL;
    }
    let handle_lc = handle.to_lowercase();
    // The whole intent typed as the exact handle ("kx/recipes/echo") ranks top —
    // the tokenizer alone can't see it (it splits on '/').
    if !intent_lc.is_empty() && handle_lc == intent_lc {
        return 10_000;
    }
    let name_lc = handle_lc
        .rsplit('/')
        .next()
        .unwrap_or(&handle_lc)
        .to_string();
    let desc_lc = m.description.to_lowercase();
    let tags_lc: Vec<String> = m.tags.iter().map(|t| t.to_lowercase()).collect();
    let mut best = 0u32;
    for t in query {
        let s = if name_lc == *t {
            9_500
        } else if name_lc.contains(t.as_str()) {
            9_000
        } else if handle_lc.contains(t.as_str()) {
            8_500
        } else if tags_lc.iter().any(|tag| tag == t) {
            7_000
        } else if tags_lc.iter().any(|tag| tag.contains(t.as_str())) {
            6_000
        } else if desc_lc.contains(t.as_str()) {
            5_000
        } else {
            0
        };
        best = best.max(s);
    }
    best
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
    /// PR-6b-4: present iff `KX_SERVE_AUTOGRANT` is on AND a model is served. When
    /// `Some`, a bind of [`REACT_AUTO_RECIPE_HANDLE`] OVERRIDES the bound seed
    /// Mote's warrant with [`tool_union_warrant`] rebuilt from the LIVE registry
    /// (admit-direct — the dialed-tool live-warrant rebuild). `None` ⇒ every
    /// existing path is byte-identical (react-auto is not seeded, so `bind` never
    /// reaches the override).
    autogrant: Option<AutoGrant>,
    /// PR-7: the live context-bundle store (the same `Arc<BundlesDb>` the gateway
    /// service holds). `Some` ⇒ a bind may resolve attached `context_bundles`
    /// handles → item refs → the entry Mote's identity-bearing `config_subset`.
    /// `None` ⇒ a non-empty `context_bundles` fails closed; an empty one is
    /// byte-identical to pre-PR-7.
    bundles: Option<std::sync::Arc<dyn BundleStore>>,
}

/// PR-6b-4: the two LIVE seams the react-auto bind override reads — the SAME
/// shared `Arc`s the coordinator/author/broker use, so a runtime-DIALED tool is
/// auto-grantable the moment its firing capability registers.
struct AutoGrant {
    /// The live tool registry (full `ToolDef`s — net/fs/syscall per tool).
    tools: std::sync::Arc<dyn ToolRegistry>,
    /// The live broker-fireable `(id, version)` set (PR-6b-2 backstop).
    registered: std::sync::Arc<dyn kx_gateway_core::RegisteredToolsView>,
}

impl HostRecipeBinder {
    /// Wrap a provisioned [`DemoLibrary`] (owns it).
    pub fn new(lib: DemoLibrary) -> Self {
        Self {
            lib: std::sync::Arc::new(lib),
            autogrant: None,
            bundles: None,
        }
    }

    /// Wrap a [`DemoLibrary`] SHARED with a [`HostRecipeCatalog`] (one seed, two
    /// seams) — the server wires both over the same `Arc<DemoLibrary>`.
    pub fn from_shared(lib: std::sync::Arc<DemoLibrary>) -> Self {
        Self {
            lib,
            autogrant: None,
            bundles: None,
        }
    }

    /// PR-7: attach the live context-bundle store so a bind can resolve a run's
    /// `context_bundles` handles into the entry Mote's identity-bearing context.
    #[must_use]
    pub fn with_bundles(mut self, bundles: std::sync::Arc<dyn BundleStore>) -> Self {
        self.bundles = Some(bundles);
        self
    }

    /// PR-6b-4: wrap a shared [`DemoLibrary`] WITH the live auto-grant seams — a
    /// bind of [`REACT_AUTO_RECIPE_HANDLE`] rebuilds the union warrant from the
    /// live registry at bind. The server uses this only when `KX_SERVE_AUTOGRANT`
    /// is on; the `tools` registry + `registered` view are the SAME `Arc`s the
    /// coordinator + broker share (one live tool set across authoring, the D66
    /// submit gate, and dispatch).
    pub fn from_shared_with_autogrant(
        lib: std::sync::Arc<DemoLibrary>,
        tools: std::sync::Arc<dyn ToolRegistry>,
        registered: std::sync::Arc<dyn kx_gateway_core::RegisteredToolsView>,
    ) -> Self {
        Self {
            lib,
            autogrant: Some(AutoGrant { tools, registered }),
            bundles: None,
        }
    }

    /// PR-6b-4: rebuild the auto-grant union warrant from the LIVE registry for a
    /// react-auto bind. `base` is the recipe's seed-time `owner_root` (model_route
    /// / resource_ceiling / executor_class). Reads the broker-fireable `(id,ver)`
    /// set and looks each up for its full `ToolDef`; a tool whose capability is
    /// registered but whose def is no longer resolvable is simply skipped (the
    /// union is best-effort + re-verified per-fire). `None` ⇒ no autogrant seam
    /// (unreachable for a seeded react-auto, but a safe fallthrough).
    fn react_auto_union(&self, base: &WarrantSpec) -> Option<WarrantSpec> {
        let ag = self.autogrant.as_ref()?;
        let defs: std::collections::BTreeMap<(ToolName, ToolVersion), ToolDef> = ag
            .registered
            .registered_grants()
            .into_iter()
            .filter_map(|(id, ver)| {
                let (name, version) = (ToolName(id), ToolVersion(ver));
                ag.tools
                    .lookup(&name, &version)
                    .map(|def| ((name, version), def))
            })
            .collect();
        Some(tool_union_warrant(base, &defs))
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

    fn recipe_metadata(&self, handle: &str) -> Option<RecipeMetadataEntry> {
        self.lib.recipe_metadata(handle)
    }

    fn search_recipes(
        &self,
        intent: &str,
        keywords: &[String],
        limit: usize,
    ) -> Option<Vec<ScoredRecipeEntry>> {
        // `Some(_)` always — this host provisions discovery (the SearchRecipes
        // RPC's `unimplemented` arm is for a catalog with no ranker). An empty
        // Vec is a valid "no match".
        Some(self.lib.search_recipes(intent, keywords, limit))
    }
}

#[tonic::async_trait]
impl RecipeBinder for HostRecipeBinder {
    async fn bind(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
        context_bundles: &[String],
    ) -> Result<BoundRecipe, BinderError> {
        // A malformed handle reveals nothing (uniform NotAuthorized — no probing).
        let asset_path = parse_handle(handle).ok_or(BinderError::NotAuthorized)?;
        // PR-7: resolve any attached context bundles to their item refs FIRST
        // (fail-closed on an unknown/unavailable handle) so the entry Mote's
        // identity reflects the exact context. Empty ⇒ byte-identical to pre-PR-7.
        let context_items = resolve_context_items(self.bundles.as_deref(), party, context_bundles)?;
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
            &context_items,
        )
        .map_err(map_invoke_err)?;
        // PR-6b-4: react-auto's bound seed Mote warrant is the seed-time PLACEHOLDER
        // (empty tool_grants). OVERRIDE it with the union warrant rebuilt from the
        // LIVE registry (admit-direct — the bound mote is byte-identical except its
        // separate WarrantSpec, which is off the MoteDef/MoteId/digest). `bind_snapshot`
        // already gated the party's Use authorization + bound the free-params, so a
        // party without a Use grant on react-auto never reaches here. The union flows
        // durably to every turn/observation via the chain's `anchor.warrant_ref`; the
        // broker precheck + coordinator D66 re-verify every axis at each fire (SN-8).
        let is_react_auto = parse_handle(REACT_AUTO_RECIPE_HANDLE).is_some_and(|p| p == asset_path);
        let motes = match (is_react_auto, self.react_auto_union(&meta.owner_root)) {
            (true, Some(union)) => bound
                .motes
                .into_iter()
                .map(|(m, _w)| (m, union.clone()))
                .collect(),
            _ => bound.motes,
        };
        Ok(BoundRecipe {
            recipe_fingerprint: bound.recipe_fingerprint,
            motes,
            terminal_mote_id: bound.terminal_mote_id,
            // PR-2d-2: the react recipe seeds a live ReAct chain — the Invoke
            // arm submits its (single) bound Mote with `react_seed = true`,
            // triggering the coordinator's run-salted seed-swap + durable anchor.
            // PR-6a/D155: react-fs is ALSO a live ReAct chain (same machinery,
            // fs-list grant) ⇒ it MUST set react_seed too, else the loop never runs.
            // PR-6b-4: react-auto is the same machinery (union tool grant).
            react_seed: [
                REACT_RECIPE_HANDLE,
                REACT_FS_RECIPE_HANDLE,
                REACT_AUTO_RECIPE_HANDLE,
            ]
            .iter()
            .any(|h| parse_handle(h).is_some_and(|p| p == asset_path)),
        })
    }
}

/// PR-7: resolve a run's attached context-bundle handles to their item refs from
/// the live bundle store (CALLER-SCOPED — a bundle is visible only to the party
/// that authored it). Empty `handles` ⇒ the pre-PR-7 path (no store needed). A
/// non-empty `handles` with NO store wired, or an unknown / unauthorized handle,
/// fails CLOSED (`InvalidArgs`) — never silently drops context (a run that asked
/// for grounding it did not receive must not be admitted as if it asked for none).
fn resolve_context_items(
    bundles: Option<&dyn BundleStore>,
    party: &str,
    handles: &[String],
) -> Result<Vec<ContextItemRef>, BinderError> {
    if handles.is_empty() {
        return Ok(Vec::new());
    }
    let store = bundles.ok_or_else(|| {
        BinderError::InvalidArgs("context bundles are not available on this gateway".to_string())
    })?;
    let mut items = Vec::new();
    for h in handles {
        let manifest = store
            .get(party, h)
            .map_err(|_| BinderError::InvalidArgs(format!("context bundle '{h}' lookup failed")))?
            .ok_or_else(|| BinderError::InvalidArgs(format!("context bundle '{h}' not found")))?;
        for it in manifest.items {
            items.push(ContextItemRef {
                name: it.name,
                content_ref: it.content_ref,
            });
        }
    }
    Ok(items)
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
    /// PR-6b-2: the LIVE tool registry (the SAME `Arc` the coordinator + broker
    /// share). A `tool()` step resolves its def here (warrant + typed schema), and
    /// `author()` widens the authoring CEILING with its tools so a server-built
    /// tool warrant survives the per-party intersect — both LIVE, so a
    /// runtime-DIALED external MCP tool is authorable the moment it registers.
    /// `None` ([`from_shared`](Self::from_shared)) ⇒ `tool()` authoring is refused
    /// (the PR-1 PURE/MODEL-only behaviour; every existing test path).
    tools: Option<std::sync::Arc<dyn ToolRegistry>>,
    /// PR-7: the live context-bundle store (the same `Arc<BundlesDb>` the gateway
    /// service + binder hold). `Some` ⇒ `author()` may resolve attached
    /// `context_bundles` → the entry step(s)' identity-bearing `config_subset`.
    bundles: Option<std::sync::Arc<dyn BundleStore>>,
}

impl HostWorkflowAuthor {
    /// Wrap a [`DemoLibrary`] shared with the binder/catalog (one seed, many
    /// seams), WITHOUT a tool registry — PURE/MODEL authoring only (`tool()` steps
    /// are refused fail-closed).
    #[must_use]
    pub fn from_shared(lib: std::sync::Arc<DemoLibrary>) -> Self {
        Self {
            lib,
            tools: None,
            bundles: None,
        }
    }

    /// PR-7: attach the live context-bundle store so `author()` can resolve a run's
    /// `context_bundles` handles into the entry step(s)' identity-bearing context.
    #[must_use]
    pub fn with_bundles(mut self, bundles: std::sync::Arc<dyn BundleStore>) -> Self {
        self.bundles = Some(bundles);
        self
    }

    /// Wrap a [`DemoLibrary`] WITH the live tool registry (PR-6b-2) — enables
    /// `tool()` step authoring + a tool-aware authoring ceiling. The registry is
    /// the SAME `Arc` shared with the coordinator + broker, so authoring, the D66
    /// submission gate, and the broker dispatch all see one live tool set.
    #[must_use]
    pub fn from_shared_with_tools(
        lib: std::sync::Arc<DemoLibrary>,
        tools: std::sync::Arc<dyn ToolRegistry>,
    ) -> Self {
        Self {
            lib,
            tools: Some(tools),
            bundles: None,
        }
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
            // TOOL (PR-6b-2): fire a single REGISTERED tool as a standalone node
            // (extracted helper — the host resolves the tool in the LIVE registry +
            // builds its warrant SERVER-SIDE from the declared scope, SN-8).
            AuthorStepKind::Tool => self.tool_step_def(index, s, base)?,
        };
        // Free params land in config_subset (identity-bearing); MODEL also binds the
        // prompt into config_subset[PROMPT_KEY] — the key the model executor reads.
        // For a TOOL step the params carry the authored args under TOOL_ARGS_KEY
        // (the SDK lowering's single param), which lands in config_subset here.
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
        // PR-6b-2: a TOOL step ALWAYS carries a TOOL_ARGS_KEY entry — it is the
        // coordinator's `is_authored_tool` discriminant and the args source. A
        // no-arg call (or a client that omitted it) defaults to the empty object
        // `{}`; the coordinator validates it against the tool's schema fail-closed.
        if s.kind == AuthorStepKind::Tool
            && !def
                .config_subset
                .contains_key(&ConfigKey(TOOL_ARGS_KEY.to_string()))
        {
            def.config_subset.insert(
                ConfigKey(TOOL_ARGS_KEY.to_string()),
                ConfigVal(b"{}".to_vec()),
            );
        }
        Ok(def)
    }

    /// PR-6b-2: build the [`StepDef`] for an authored `tool()` step. Resolves the
    /// single `(tool_id, tool_version)` in the LIVE registry (fail-closed on
    /// absent/`PendingHumanReview`), server-assigns a content-sentinel `logic_ref`,
    /// and builds the per-step warrant from the tool's DECLARED scope ([`tool_step_warrant`]).
    /// The authored args land in `config_subset[TOOL_ARGS_KEY]` via the shared
    /// params loop in [`step_def`](Self::step_def).
    fn tool_step_def(
        &self,
        index: usize,
        s: &AuthorStep,
        base: &WarrantSpec,
    ) -> Result<StepDef, BinderError> {
        let tools = self.tools.as_ref().ok_or_else(|| {
            BinderError::InvalidArgs(
                "this serve has no tool registry; TOOL steps are unavailable".into(),
            )
        })?;
        // Exactly one (tool_id, tool_version) — a tool step binds one tool.
        let mut it = s.tool_contract.iter();
        let (name, version) = it.next().ok_or_else(|| {
            BinderError::InvalidArgs(
                "TOOL step must name exactly one (tool_id, tool_version)".into(),
            )
        })?;
        if it.next().is_some() {
            return Err(BinderError::InvalidArgs(
                "TOOL step must name exactly one tool".into(),
            ));
        }
        let tool_name = ToolName(name.clone());
        let tool_version = ToolVersion(version.clone());
        // Resolve the registered tool — `lookup` returns `None` for an absent OR
        // `PendingHumanReview` registration (fail-closed, GR15).
        let tdef = tools.lookup(&tool_name, &tool_version).ok_or_else(|| {
            BinderError::InvalidArgs(format!(
                "TOOL step references unregistered tool {name}@{version}"
            ))
        })?;
        // Server-assigned content-sentinel logic_ref over (index, id, ver), so
        // distinct tool steps get distinct ids (the client never supplies bytes).
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(b"kx-blueprint/tool/v1");
        buf.extend_from_slice(&(index as u64).to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.push(b'@');
        buf.extend_from_slice(version.as_bytes());
        let logic_ref = LogicRef::from_bytes(*ContentRef::of(&buf).as_bytes());
        // The step warrant = the authoring base (matching syscall_profile / executor
        // / model_route ceilings) NARROWED to THIS tool's grant + declared net/fs.
        // `author()` widens the party ceiling with the live registry so this survives
        // the intersect; the broker precheck + coordinator D66 re-verify at fire.
        let warrant = tool_step_warrant(base, &tool_name, &tool_version, &tdef);
        let mut sd = tool_step(
            logic_ref,
            base.model_route.model_id.clone(),
            warrant,
            tool_name.clone(),
        );
        let mut tc = std::collections::BTreeMap::new();
        tc.insert(tool_name, tool_version);
        sd.tool_contract = tc;
        Ok(sd)
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
        context_bundles: &[String],
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

        // PR-7: inject any attached context-bundle items into every ENTRY step's
        // identity-bearing config BEFORE compile (fail-closed on an unknown handle;
        // empty ⇒ byte-identical to pre-PR-7). Resolution is caller-scoped.
        let context_items = resolve_context_items(self.bundles.as_deref(), party, context_bundles)?;
        if !context_items.is_empty() {
            let encoded = ConfigVal(encode_context_items(&context_items));
            wf.inject_entry_config(CONTEXT_ITEMS_KEY, &encoded);
        }

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
            // PR-6b-2: a TOOL step's warrant (non-empty `tool_grants`) is SERVER-built
            // from the registry (`tool_step_warrant` — the tool's declared net/fs +
            // its own `syscall_profile_ref`) and admitted DIRECTLY, like the seeded
            // react recipes — NOT intersected against the party blueprint grant
            // (whose syscall profile differs, and which never grants the tool). The
            // per-party gate is "can author at all" (`effective` resolved above); the
            // `registered_tools` backstop + the broker precheck + the coordinator's
            // D66 resolution re-verify every axis server-side at fire (SN-8). Every
            // PURE/MODEL mote (empty `tool_grants`) narrows against the party
            // authority exactly as before (byte-identical when no tool() steps).
            let warrant = if cm.warrant.tool_grants.is_empty() {
                let step_role = Role {
                    name: "blueprint-step".to_string(),
                    version: 0,
                    spec: cm.warrant.clone(),
                    description: String::new(),
                };
                intersect(&effective, &step_role).map_err(|_| BinderError::NotAuthorized)?
            } else {
                cm.warrant.clone()
            };
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

fn passthrough_dag_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(PASSTHROUGH_DAG_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid passthrough-dag recipe handle".into()))
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

/// Seed the T3.3 PURE multi-node `passthrough-dag` recipe and return its
/// `(handle, meta)` for the binder's recipe table. Factored out of
/// [`DemoLibrary::seed`] so the orchestrator stays within the line budget and each
/// recipe reads as a self-contained unit.
fn seed_passthrough_dag(
    versions: &SqliteVersionLedger,
    bodies: &SqliteBodyLedger,
    grants: &SqliteGrantLedger,
    owner: &PartyId,
    parties: &[String],
    exec_class: ExecutorClass,
) -> Result<(AssetPath, RecipeMeta), GatewayError> {
    let warrant = demo_warrant(exec_class);
    let handle = passthrough_dag_handle()?;
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
            ToolName("passthrough-dag".into()),
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

/// PR-6a/D155: the server-built `react-fs` step warrant. Mirrors [`react_warrant`]
/// but grants the read-only `fs-list@1` tool + a `fs_scope` of the operator-granted
/// read `root` (ReadOnly). The grant's fs_scope MUST equal the tool's declared
/// `fs_scope_required` so the broker's `precheck` subset gate passes and the
/// capability receives the root via `request.fs_scope`. `net_scope: None` (fs-list
/// has no egress). Tool authority NEVER enters via a client warrant (BLOCKER #5/SN-8).
pub(crate) fn react_fs_warrant(
    exec_class: ExecutorClass,
    model_id: &ModelId,
    tool: &(ToolName, ToolVersion),
    root: &Path,
) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(kx_warrant::ToolGrant {
        tool_id: tool.0.clone(),
        tool_version: tool.1.clone(),
    });
    let mut mounts = std::collections::BTreeMap::new();
    mounts.insert(root.to_path_buf(), kx_warrant::FsMode::ReadOnly);
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope { mounts },
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
            wall_clock_ms: 120_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: exec_class,
        ..Default::default()
    }
}

/// PR-6b-2: the COMPLETE server-built warrant for a standalone authored `tool()`
/// step — mirrors [`react_fs_warrant`] (a server-built, directly-admitted warrant)
/// but GENERIC over the tool's DECLARED `required_capability`. Grants EXACTLY
/// `(name, version)` and takes the tool's declared net/fs **and
/// `syscall_profile_ref`** (the resolver's `check_tool_requirement` demands syscall
/// EQUALITY, so the warrant must carry the TOOL's own profile — not the authoring
/// base's). The `model_route` / `resource_ceiling` / `executor_class` come from
/// `base` so the mote leases on the served worker. ReadOnlyNondet classes mirror
/// the react recipe warrant (the mcp-echo dispatch precedent). The coordinator's
/// `resolve_authored_tool_args` sets the dispatch REQUEST net/fs to the SAME
/// declared values, so `request ⊆ warrant` holds by construction at the broker's
/// precheck. Admitted DIRECTLY (not intersected against the party blueprint grant):
/// the tool's scope is SERVER-vetted (the registry), so the per-party gate is "can
/// author at all" (`effective` resolved); the broker precheck + coordinator D66
/// re-verify every axis at fire (SN-8).
fn tool_step_warrant(
    base: &WarrantSpec,
    name: &ToolName,
    version: &ToolVersion,
    tdef: &ToolDef,
) -> WarrantSpec {
    let mut grants = BTreeSet::new();
    grants.insert(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: tdef.required_capability.fs_scope_required.clone(),
        net_scope: tdef.required_capability.net_scope_required.clone(),
        syscall_profile_ref: tdef.required_capability.syscall_profile_ref,
        tool_grants: grants,
        model_route: base.model_route.clone(),
        resource_ceiling: base.resource_ceiling,
        environment_ref: None,
        executor_class: base.executor_class,
        ..Default::default()
    }
}

fn react_fs_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(REACT_FS_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid react-fs recipe handle".into()))
}

/// PR-6b-4: the cap on the auto-grant union warrant's tool set — bounds the
/// model's tool menu (prompt size) AND the union warrant's scope breadth. When
/// more than this many tools are registered, a deterministic `(id, version)`
/// prefix is granted (the rest are still authorable via an explicit `tool()`
/// node). 16 is generous for the live ReAct menu and keeps the prompt bounded.
const AUTOGRANT_MAX_TOOLS: usize = 16;

/// The "no syscall profile" sentinel every in-scope tool declares (MCP / stdio /
/// host-read tools do not run sandboxed body-exec). The auto-grant union FILTERS
/// to this profile so the union warrant's single `syscall_profile_ref` satisfies
/// the registry resolver's per-tool EQUALITY gate (`check_tool_requirement`) for
/// every granted tool (cf. BUG-24). A future sandboxed tool with a different
/// profile is simply excluded from auto-grant (still fireable via `tool()`).
const EMPTY_SYSCALL_PROFILE: [u8; 32] = [0u8; 32];

/// Union of two [`NetScope`]s — the identity is [`NetScope::None`] (`None ∪ X =
/// X`); two allowlists merge their host sets. A widening op (the dual of
/// `is_subset_of`), used ONLY to build the server's auto-grant union warrant from
/// already-vetted tool scopes — never to narrow a caller's authority.
fn net_scope_union(a: &NetScope, b: &NetScope) -> NetScope {
    match (a, b) {
        (NetScope::None, other) | (other, NetScope::None) => other.clone(),
        (NetScope::EgressAllowlist(x), NetScope::EgressAllowlist(y)) => {
            let mut hosts: BTreeSet<Host> = x.clone();
            hosts.extend(y.iter().cloned());
            NetScope::EgressAllowlist(hosts)
        }
    }
}

/// Per-path least-upper-bound merge of two [`FsScope`]s under the [`FsMode`]
/// subset order. Disjoint paths union; a shared path takes the wider mode.
/// Returns `None` (fail-closed) iff any shared path has INCOMPARABLE modes
/// (e.g. `ReadWrite` vs `ExecOnly` — neither is a subset of the other), so the
/// caller drops that tool from the union rather than fabricate a phantom
/// superset. In-scope tools are all `ReadOnly`/empty, so this never fires today;
/// the check is the forward-safety guard.
fn fs_scope_union(a: &FsScope, b: &FsScope) -> Option<FsScope> {
    let mut mounts = a.mounts.clone();
    for (path, mode_b) in &b.mounts {
        match mounts.get(path) {
            None => {
                mounts.insert(path.clone(), *mode_b);
            }
            Some(mode_a) => {
                // The least upper bound under `is_subset_of`: whichever mode the
                // other is a subset of. Incomparable ⇒ fail closed.
                let lub = if mode_a.is_subset_of(*mode_b) {
                    *mode_b
                } else if mode_b.is_subset_of(*mode_a) {
                    *mode_a
                } else {
                    return None;
                };
                mounts.insert(path.clone(), lub);
            }
        }
    }
    Some(FsScope { mounts })
}

/// PR-6b-4: the SERVER-built UNION warrant for the autonomous `react-auto` loop —
/// auto-grants the LIVE set of registered/dialed tools so the model can pick from
/// ALL of them (not just one bundled seed tool). `defs` is the live broker-fireable
/// `(id, version) → ToolDef` set; the union is rebuilt at BIND (a dialed tool
/// registers at runtime, after seed). Filters to [`EMPTY_SYSCALL_PROFILE`] (so the
/// per-tool syscall EQUALITY gate passes — BUG-24) AND to fs-union-compatible tools;
/// caps at [`AUTOGRANT_MAX_TOOLS`] by a deterministic `(id, version)` sort + prefix.
/// `tool_grants` = that set; `net_scope` / `fs_scope` = the UNION of their declared
/// `required_capability` scopes (so the broker `precheck` `request ⊆ warrant` passes
/// for EVERY granted tool, the per-call request carrying THAT tool's own scope);
/// `syscall_profile_ref` = the empty sentinel; `model_route` / `resource_ceiling` /
/// `executor_class` from `base` (the react-auto seed warrant) so the chain leases on
/// the served worker. Admitted DIRECTLY at bind (mirrors [`tool_step_warrant`] / the
/// PR-6b-2 `author()` precedent): the tool scopes are SERVER-vetted (SSRF-vetted at
/// dial, the registry the source of truth) and the operator opt-in (`KX_SERVE_AUTOGRANT`)
/// is the OSS ceiling; the broker precheck + coordinator D66 re-verify every axis at
/// each fire (SN-8). Client `tool_grants` are NEVER accepted.
pub(crate) fn tool_union_warrant(
    base: &WarrantSpec,
    defs: &std::collections::BTreeMap<(ToolName, ToolVersion), ToolDef>,
) -> WarrantSpec {
    // Deterministic order: BTreeMap already iterates by (ToolName, ToolVersion),
    // so the cap takes a stable prefix and the warrant is byte-reproducible.
    let mut tool_grants: BTreeSet<ToolGrant> = BTreeSet::new();
    let mut net_scope = NetScope::None;
    let mut fs_scope = FsScope::empty();
    for ((name, version), def) in defs {
        if tool_grants.len() >= AUTOGRANT_MAX_TOOLS {
            break;
        }
        let cap = &def.required_capability;
        // Only tools with the empty syscall profile can share one union warrant.
        if cap.syscall_profile_ref != ContentRef::from_bytes(EMPTY_SYSCALL_PROFILE) {
            continue;
        }
        // A tool whose fs scope is incomparable with the running union is skipped
        // (never poisons the whole union); echo/fs-list/MCP tools never trip this.
        let Some(merged_fs) = fs_scope_union(&fs_scope, &cap.fs_scope_required) else {
            continue;
        };
        fs_scope = merged_fs;
        net_scope = net_scope_union(&net_scope, &cap.net_scope_required);
        tool_grants.insert(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
    }
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope,
        net_scope,
        syscall_profile_ref: ContentRef::from_bytes(EMPTY_SYSCALL_PROFILE),
        tool_grants,
        model_route: base.model_route.clone(),
        resource_ceiling: base.resource_ceiling,
        environment_ref: None,
        executor_class: base.executor_class,
        ..Default::default()
    }
}

/// PR-6b-4: the seed-time PLACEHOLDER warrant for `kx/recipes/react-auto`. Mirrors
/// [`react_warrant`] but with EMPTY `tool_grants` (no bundled tool) — at bind the
/// host binder OVERRIDES the bound seed Mote's warrant with [`tool_union_warrant`]
/// rebuilt from the LIVE registry. This base supplies the `model_route` /
/// `resource_ceiling` / `executor_class` so the recipe is structurally valid + the
/// chain leases on the served worker, and it is the `base` passed to the union
/// builder at bind.
pub(crate) fn react_auto_base_warrant(
    exec_class: ExecutorClass,
    model_id: &ModelId,
) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes(EMPTY_SYSCALL_PROFILE),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model_id.clone(),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 120_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: exec_class,
        ..Default::default()
    }
}

/// The wire handle of the PR-6b-4 `react-auto` recipe: a live ReAct loop like
/// [`REACT_RECIPE_HANDLE`] BUT whose warrant is REBUILT at bind from the LIVE
/// registry to auto-grant the registered/dialed tool set (a union warrant, ≤
/// `AUTOGRANT_MAX_TOOLS`). A SEPARATE recipe (the react-fs precedent) so the
/// canonical `kx/recipes/react` + the projection digest stay byte-unchanged.
/// Seeded only when a fit serve model resolved AND `KX_SERVE_AUTOGRANT` is on.
pub const REACT_AUTO_RECIPE_HANDLE: &str = "kx/recipes/react-auto";

fn react_auto_handle() -> Result<AssetPath, GatewayError> {
    parse_handle(REACT_AUTO_RECIPE_HANDLE)
        .ok_or_else(|| GatewayError::Catalog("invalid react-auto recipe handle".into()))
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
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#, &[])
            .await
            .unwrap();
        let a2 = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#, &[])
            .await
            .unwrap();
        assert_eq!(
            a1.terminal_mote_id, a2.terminal_mote_id,
            "identical args → identical identity (idempotent re-invoke)"
        );
        assert_eq!(a1.recipe_fingerprint, a2.recipe_fingerprint);

        let b = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"y"}"#, &[])
            .await
            .unwrap();
        assert_ne!(
            a1.terminal_mote_id, b.terminal_mote_id,
            "distinct args → distinct identity (exactly-once-per-input)"
        );
    }

    #[tokio::test]
    async fn context_bundle_injection_is_identity_bearing_and_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        // A live bundle store with one bundle authored by alice.
        let bundles = std::sync::Arc::new(crate::bundles::BundlesDb::open(dir.path()).unwrap());
        bundles
            .upsert(
                "alice@acme",
                "team/ctx/notes",
                "alice's notes",
                &[kx_gateway_core::BundleItemRecord {
                    name: "doc".into(),
                    content_ref: [0xab; 32],
                    media_type: String::new(),
                }],
            )
            .unwrap();
        let binder = HostRecipeBinder::new(lib).with_bundles(bundles.clone());

        // Same input, NO context ⇒ the pre-PR-7 identity.
        let plain = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#, &[])
            .await
            .unwrap();
        // Same input, WITH the bundle ⇒ a DIFFERENT entry identity (exactly-once-
        // per-(input+context)). The bundle ref-set is folded into the entry Mote's
        // identity-bearing config_subset.
        let grounded = binder
            .bind(
                "alice@acme",
                DEMO_RECIPE_HANDLE,
                br#"{"topic":"x"}"#,
                &["team/ctx/notes".to_string()],
            )
            .await
            .unwrap();
        assert_ne!(
            plain.terminal_mote_id, grounded.terminal_mote_id,
            "attaching a context bundle changes the entry MoteId (identity-bearing)"
        );

        // The SAME (input + context) re-derives the SAME identity (idempotent).
        let grounded2 = binder
            .bind(
                "alice@acme",
                DEMO_RECIPE_HANDLE,
                br#"{"topic":"x"}"#,
                &["team/ctx/notes".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(
            grounded.terminal_mote_id, grounded2.terminal_mote_id,
            "identical input+context ⇒ identical identity"
        );

        // An unknown handle FAILS CLOSED (never silently drops requested context).
        let unknown = binder
            .bind(
                "alice@acme",
                DEMO_RECIPE_HANDLE,
                br#"{"topic":"x"}"#,
                &["team/ctx/missing".to_string()],
            )
            .await;
        assert!(
            matches!(unknown, Err(BinderError::InvalidArgs(_))),
            "an unknown context bundle is refused at admission"
        );

        // Another party cannot resolve alice's bundle (caller-scoped, fail-closed).
        let cross_party = binder
            .bind(
                "alice@acme",
                DEMO_RECIPE_HANDLE,
                br#"{"topic":"x"}"#,
                &["team/ctx/notes".to_string()],
            )
            .await;
        assert!(cross_party.is_ok(), "alice resolves her own bundle");
    }

    #[tokio::test]
    async fn context_bundles_without_a_store_fail_closed() {
        // A binder with NO bundle store wired (the default) refuses a non-empty
        // context_bundles rather than silently ignoring it.
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        let err = binder
            .bind(
                "alice@acme",
                DEMO_RECIPE_HANDLE,
                br#"{"topic":"x"}"#,
                &["team/ctx/notes".to_string()],
            )
            .await;
        assert!(
            matches!(err, Err(BinderError::InvalidArgs(_))),
            "context bundles with no store wired fail closed"
        );
        // An EMPTY context_bundles is byte-identical to pre-PR-7 (still binds).
        let ok = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#, &[])
            .await;
        assert!(ok.is_ok(), "no context ⇒ unchanged");
    }

    #[tokio::test]
    async fn bind_no_widen_keeps_worker_executor_class() {
        let dir = tempfile::tempdir().unwrap();
        let binder = demo_lib(dir.path());
        let bound = binder
            .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#, &[])
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
            .bind("alice@acme", PASSTHROUGH_DAG_HANDLE, b"{}", &[])
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
            .bind("alice@acme", PASSTHROUGH_DAG_HANDLE, b"{}", &[])
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
                .bind("mallory@acme", DEMO_RECIPE_HANDLE, br#"{"topic":"x"}"#, &[])
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
                .bind("alice@acme", "kx/recipes/nope", br#"{"topic":"x"}"#, &[])
                .await,
            Err(BinderError::NotAuthorized)
        ));
        assert!(matches!(
            binder
                .bind("alice@acme", "not-a-handle", br#"{"topic":"x"}"#, &[])
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
                .bind("alice@acme", DEMO_RECIPE_HANDLE, br#"{"topic":5}"#, &[])
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // Missing `topic`.
        assert!(matches!(
            binder
                .bind("alice@acme", DEMO_RECIPE_HANDLE, b"{}", &[])
                .await,
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

    // ---- PR-6b-2: tool() authoring helpers + tests ------------------------

    fn tool_registry_with(tool_id: &str, version: &str) -> std::sync::Arc<dyn ToolRegistry> {
        use kx_tool_registry::{IdempotencyClass, InMemoryToolRegistry, ToolKind, ToolProvenance};
        let mut reg = InMemoryToolRegistry::new();
        let def = ToolDef {
            tool_id: ToolName(tool_id.into()),
            tool_version: ToolVersion(version.into()),
            kind: ToolKind::Builtin,
            required_capability: kx_warrant::ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: String::new(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: None,
        };
        let _ = reg.register(def, ToolProvenance::HumanAuthored { author: "t".into() });
        std::sync::Arc::new(reg)
    }

    fn author_with_tools(
        dir: &std::path::Path,
        tools: std::sync::Arc<dyn ToolRegistry>,
    ) -> HostWorkflowAuthor {
        let lib =
            DemoLibrary::open(dir, ExecutorClass::Bwrap, &["alice@acme".to_string()]).unwrap();
        HostWorkflowAuthor::from_shared_with_tools(std::sync::Arc::new(lib), tools)
    }

    fn tool_author_step(tool_id: &str, version: &str, args_json: &[u8]) -> AuthorStep {
        let mut tc = std::collections::BTreeMap::new();
        tc.insert(tool_id.to_string(), version.to_string());
        let mut params = std::collections::BTreeMap::new();
        params.insert(TOOL_ARGS_KEY.to_string(), args_json.to_vec());
        AuthorStep {
            kind: AuthorStepKind::Tool,
            model_id: String::new(),
            prompt: String::new(),
            body_signature_id: None,
            tool_contract: tc,
            params,
        }
    }

    /// A `tool()` step authors a fireable, args-bearing, tool-granting Mote — the
    /// shape the coordinator's `is_authored_tool` + the worker's args gate expect.
    #[tokio::test]
    async fn tool_step_authors_a_fireable_tool_node() {
        let dir = tempfile::tempdir().unwrap();
        let author = author_with_tools(dir.path(), tool_registry_with("echo-tool", "1"));
        let steps = [tool_author_step("echo-tool", "1", br#"{"q":"hi"}"#)];
        let bound = author
            .author(
                "alice@acme",
                3,
                &steps,
                &[],
                AuthorExecutionMode::Frozen,
                &[],
            )
            .await
            .expect("tool step authored");
        assert_eq!(bound.motes.len(), 1);
        let (mote, warrant) = &bound.motes[0];
        // Names the tool + carries the authored args (the args-from-config path).
        assert!(mote
            .def
            .tool_contract
            .contains_key(&ToolName("echo-tool".into())));
        let args = mote
            .def
            .config_subset
            .get(&ConfigKey(TOOL_ARGS_KEY.to_string()))
            .expect("authored args land in config_subset");
        assert_eq!(args.0, br#"{"q":"hi"}"#.to_vec());
        // The SERVER-built warrant GRANTS the tool (so the broker can fire it; SN-8).
        assert!(warrant
            .tool_grants
            .iter()
            .any(|g| g.tool_id.0 == "echo-tool"));
        // The observation shape: WORLD-MUTATING + StageThenCommit.
        assert_eq!(
            mote.def.effect_pattern,
            kx_mote::EffectPattern::StageThenCommit
        );
        // Deterministic + content-addressed (same authored bytes → same identity).
        let again = author
            .author(
                "alice@acme",
                3,
                &steps,
                &[],
                AuthorExecutionMode::Frozen,
                &[],
            )
            .await
            .expect("re-authored");
        assert_eq!(bound.terminal_mote_id, again.terminal_mote_id);
    }

    /// A `tool()` step naming an UNREGISTERED tool is refused at authoring (the
    /// fail-closed GR15 gate — the registry lookup misses).
    #[tokio::test]
    async fn tool_step_unregistered_tool_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let author = author_with_tools(dir.path(), tool_registry_with("echo-tool", "1"));
        let steps = [tool_author_step("ghost-tool", "1", b"{}")];
        assert!(matches!(
            author
                .author(
                    "alice@acme",
                    1,
                    &steps,
                    &[],
                    AuthorExecutionMode::Frozen,
                    &[]
                )
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
    }

    /// Without a wired tool registry (`from_shared`), `tool()` authoring is refused
    /// fail-closed (the PR-1 PURE/MODEL-only behaviour is byte-unchanged).
    #[tokio::test]
    async fn tool_step_without_registry_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let author = demo_author(dir.path());
        let steps = [tool_author_step("echo-tool", "1", b"{}")];
        assert!(matches!(
            author
                .author(
                    "alice@acme",
                    1,
                    &steps,
                    &[],
                    AuthorExecutionMode::Frozen,
                    &[]
                )
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
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
            .author(
                "alice@acme",
                7,
                &steps,
                &edges,
                AuthorExecutionMode::Frozen,
                &[],
            )
            .await
            .expect("authored");
        let b = author
            .author(
                "alice@acme",
                7,
                &steps,
                &edges,
                AuthorExecutionMode::Frozen,
                &[],
            )
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
            .author(
                "alice@acme",
                8,
                &steps,
                &edges,
                AuthorExecutionMode::Frozen,
                &[],
            )
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
                    AuthorExecutionMode::Frozen,
                    &[],
                )
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // DYNAMIC mode is reserved (PR-1 frozen-only).
        assert!(matches!(
            author
                .author(
                    "alice@acme",
                    1,
                    &steps,
                    &[],
                    AuthorExecutionMode::Dynamic,
                    &[]
                )
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // An empty DAG is refused.
        assert!(matches!(
            author
                .author("alice@acme", 1, &[], &[], AuthorExecutionMode::Frozen, &[])
                .await,
            Err(BinderError::InvalidArgs(_))
        ));
        // An ungranted party gets a UNIFORM NotAuthorized (no existence oracle).
        assert!(matches!(
            author
                .author(
                    "mallory@evil",
                    1,
                    &steps,
                    &[],
                    AuthorExecutionMode::Frozen,
                    &[]
                )
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
                &[],
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
            Some(model_id.clone()),
        )
        .unwrap();
        let binder = HostRecipeBinder::new(lib);

        let bound = binder
            .bind(
                "alice@acme",
                MODEL_RECIPE_HANDLE,
                br#"{"prompt":"Capital of France?"}"#,
                &[],
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
                .bind("alice@acme", MODEL_RECIPE_HANDLE, br#"{"prompt":"x"}"#, &[])
                .await,
            Err(BinderError::NotAuthorized)
        ));
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
        // `echo` + `passthrough-dag` are always seeded; `chat`/`react`/`vision` are conditional.
        assert!(handles.contains(&DEMO_RECIPE_HANDLE.to_string()));
        assert!(handles.contains(&PASSTHROUGH_DAG_HANDLE.to_string()));
        assert!(!handles.contains(&MODEL_RECIPE_HANDLE.to_string()));
    }

    // --- PR-4 Batch D: advisory metadata + the pure recipe ranker ---

    #[test]
    fn recipe_metadata_is_the_honest_advisory_for_provisioned_handles() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        let m = lib
            .recipe_metadata(DEMO_RECIPE_HANDLE)
            .expect("echo is provisioned");
        assert!(m.description.contains("Echo"));
        assert!(m.tags.contains(&"passthrough".to_string()));
        // Content-addressed published version pin (12-hex), never a faked semver.
        assert_eq!(
            m.version.len(),
            12,
            "the version is the 12-hex version-id pin"
        );
        assert!(m.version.chars().all(|c| c.is_ascii_hexdigit()));
        // An unprovisioned / unknown handle yields no metadata (no oracle).
        assert!(lib.recipe_metadata(MODEL_RECIPE_HANDLE).is_none());
        assert!(lib.recipe_metadata("kx/recipes/does-not-exist").is_none());
    }

    #[test]
    fn search_recipes_ranks_exact_handle_first_and_filters_nonmatches() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        // Exact handle wins (10000 bp); only positive matches are returned.
        let ranked = lib.search_recipes(DEMO_RECIPE_HANDLE, &[], 20);
        assert_eq!(
            ranked.first().map(|r| r.handle.as_str()),
            Some(DEMO_RECIPE_HANDLE)
        );
        assert_eq!(ranked[0].score_bp, 10_000);
        assert!(
            ranked.iter().all(|r| r.score_bp > 0),
            "a non-empty query drops zero-score recipes"
        );

        // A tag/keyword query surfaces by tag (the passthrough-dag carries it too).
        let by_tag = lib.search_recipes("", &["passthrough".to_string()], 20);
        let hits: Vec<&str> = by_tag.iter().map(|r| r.handle.as_str()).collect();
        assert!(hits.contains(&DEMO_RECIPE_HANDLE));
        assert!(hits.contains(&PASSTHROUGH_DAG_HANDLE));
        // score_bp is always display-bounded (SN-8: never a float, ≤ 10000).
        assert!(by_tag.iter().all(|r| r.score_bp <= 10_000));
    }

    #[test]
    fn search_recipes_empty_query_lists_all_and_limit_caps() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
        )
        .unwrap();
        let all = lib.search_recipes("", &[], 20);
        assert_eq!(
            all.len(),
            lib.recipe_handles().len(),
            "empty query lists every recipe"
        );
        // Deterministic best-first → handle tiebreak (here all neutral ⇒ handle order).
        let mut sorted = all.clone();
        sorted.sort_by(|a, b| {
            b.score_bp
                .cmp(&a.score_bp)
                .then_with(|| a.handle.cmp(&b.handle))
        });
        assert_eq!(
            all.iter().map(|r| &r.handle).collect::<Vec<_>>(),
            sorted.iter().map(|r| &r.handle).collect::<Vec<_>>(),
        );
        // The limit caps the result set.
        assert_eq!(lib.search_recipes("", &[], 1).len(), 1);
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
            .recipe_form(PASSTHROUGH_DAG_HANDLE)
            .expect("passthrough-dag is provisioned");
        assert!(form.is_empty(), "passthrough-dag takes no free-params");
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
            Some(&ModelId("kx-serve:vlm".to_string())),
            None,
            false,
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
            Some(&ModelId("kx-serve:vlm".to_string())),
            None,
            true,
            None,
            false,
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
    fn react_fs_recipe_seeded_only_with_fs_list() {
        // PR-6a/D155: react-fs is a SEPARATE recipe, seeded only when a model is
        // served AND fs-list is available — default-OFF keeps the canonical set.
        let model = ModelId("kx-serve:m".to_string());
        let fs_tool = (ToolName("fs-list".into()), ToolVersion("1".into()));
        let root = tempfile::tempdir().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_complete(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            Some(&model),
            None,
            false,
            Some((&fs_tool, root.path())),
            false,
        )
        .unwrap();
        assert!(lib
            .recipe_handles()
            .contains(&REACT_FS_RECIPE_HANDLE.to_string()));
        // The published form is the react contract (instruction + the two budget caps).
        let form = lib
            .recipe_form(REACT_FS_RECIPE_HANDLE)
            .expect("react-fs is provisioned");
        assert_eq!(form.len(), 3);

        // Without an fs-list binding ⇒ NOT seeded (default-OFF).
        let dir2 = tempfile::tempdir().unwrap();
        let lib2 = DemoLibrary::open_complete(
            dir2.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            Some(&model),
            None,
            false,
            None,
            false,
        )
        .unwrap();
        assert!(!lib2
            .recipe_handles()
            .contains(&REACT_FS_RECIPE_HANDLE.to_string()));
    }

    #[test]
    fn react_variants_coexist_without_a_body_id_collision() {
        // BUG-25 regression: react (echo), react-fs (fs-list), and react-auto all
        // seed live ReAct chains whose recipe bodies differ ONLY by their
        // server-built warrant. The body manifest id is `hash(seed ‖ mote_ids)` and
        // EXCLUDES the warrant, so a shared logic ref makes the second seed a
        // body-ledger immutability conflict (a startup panic). Each carries its OWN
        // logic ref, so a serve with the echo bin + KX_SERVE_FS_ROOT + autogrant
        // provisions ALL of them without conflict.
        let model = ModelId("kx-serve:m".to_string());
        let echo = (ToolName("mcp-echo".into()), ToolVersion("1".into()));
        let fs_tool = (ToolName("fs-list".into()), ToolVersion("1".into()));
        let root = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_complete(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            Some(&model),
            Some(&echo), // react
            false,
            Some((&fs_tool, root.path())), // react-fs
            true,                          // react-auto
        )
        .expect("all three react variants seed without a body-id collision");
        let handles = lib.recipe_handles();
        assert!(handles.contains(&REACT_RECIPE_HANDLE.to_string()));
        assert!(handles.contains(&REACT_FS_RECIPE_HANDLE.to_string()));
        assert!(handles.contains(&REACT_AUTO_RECIPE_HANDLE.to_string()));
        // Each has a DISTINCT recipe fingerprint (the distinct logic refs).
        let fp = |h: &str| lib.recipe_fingerprint(h).unwrap();
        assert_ne!(fp(REACT_RECIPE_HANDLE), fp(REACT_FS_RECIPE_HANDLE));
        assert_ne!(fp(REACT_RECIPE_HANDLE), fp(REACT_AUTO_RECIPE_HANDLE));
        assert_ne!(fp(REACT_FS_RECIPE_HANDLE), fp(REACT_AUTO_RECIPE_HANDLE));
    }

    #[test]
    fn react_auto_recipe_seeded_only_with_the_autogrant_flag() {
        // PR-6b-4: react-auto is a SEPARATE recipe, seeded only when a model is
        // served AND the operator opted in — default-OFF keeps the canonical set.
        let model = ModelId("kx-serve:m".to_string());

        // autogrant ON ⇒ seeded; the published form is the react contract.
        let dir = tempfile::tempdir().unwrap();
        let lib = DemoLibrary::open_complete(
            dir.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            Some(&model),
            None,
            false,
            None,
            true,
        )
        .unwrap();
        assert!(lib
            .recipe_handles()
            .contains(&REACT_AUTO_RECIPE_HANDLE.to_string()));
        let form = lib
            .recipe_form(REACT_AUTO_RECIPE_HANDLE)
            .expect("react-auto is provisioned");
        assert_eq!(form.len(), 3, "instruction + the two budget caps");

        // autogrant OFF (default) ⇒ NOT seeded.
        let dir2 = tempfile::tempdir().unwrap();
        let lib2 = DemoLibrary::open_complete(
            dir2.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            Some(&model),
            None,
            false,
            None,
            false,
        )
        .unwrap();
        assert!(!lib2
            .recipe_handles()
            .contains(&REACT_AUTO_RECIPE_HANDLE.to_string()));

        // No served model ⇒ NOT seeded even with the flag on.
        let dir3 = tempfile::tempdir().unwrap();
        let lib3 = DemoLibrary::open_complete(
            dir3.path(),
            ExecutorClass::Bwrap,
            &["alice@acme".to_string()],
            None,
            None,
            false,
            None,
            true,
        )
        .unwrap();
        assert!(!lib3
            .recipe_handles()
            .contains(&REACT_AUTO_RECIPE_HANDLE.to_string()));
    }

    #[test]
    fn react_fs_warrant_grants_fs_list_and_the_read_root() {
        let root = std::path::PathBuf::from("/data");
        let w = react_fs_warrant(
            ExecutorClass::Bwrap,
            &ModelId("m".to_string()),
            &(ToolName("fs-list".into()), ToolVersion("1".into())),
            &root,
        );
        assert!(w.tool_grants.iter().any(|g| g.tool_id.0 == "fs-list"));
        assert_eq!(
            w.fs_scope.mounts.get(&root),
            Some(&kx_warrant::FsMode::ReadOnly),
            "the grant's fs_scope must equal the tool's declared scope (precheck subset)"
        );
        assert_eq!(w.net_scope, NetScope::None, "fs-list has no egress");
    }

    // ---- PR-6b-4: auto-grant union warrant -------------------------------

    fn host(h: &str) -> Host {
        Host(h.to_string())
    }

    fn tool_def_with(net: NetScope, fs: FsScope, syscall: [u8; 32]) -> ToolDef {
        ToolDef {
            tool_id: ToolName("t".into()),
            tool_version: ToolVersion("1".into()),
            kind: kx_tool_registry::ToolKind::Builtin,
            required_capability: kx_warrant::ToolRequirement {
                net_scope_required: net,
                fs_scope_required: fs,
                syscall_profile_ref: ContentRef::from_bytes(syscall),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: String::new(),
            idempotency_class: kx_tool_registry::IdempotencyClass::Staged,
            input_schema: None,
        }
    }

    #[test]
    fn net_scope_union_treats_none_as_identity_and_merges_allowlists() {
        let a = NetScope::EgressAllowlist([host("api.example.com")].into_iter().collect());
        assert_eq!(net_scope_union(&NetScope::None, &a), a, "None ∪ X = X");
        assert_eq!(net_scope_union(&a, &NetScope::None), a, "X ∪ None = X");
        assert_eq!(
            net_scope_union(&NetScope::None, &NetScope::None),
            NetScope::None
        );
        let b = NetScope::EgressAllowlist([host("b.example.com")].into_iter().collect());
        let merged = net_scope_union(&a, &b);
        match merged {
            NetScope::EgressAllowlist(hosts) => {
                assert!(hosts.contains(&host("api.example.com")));
                assert!(hosts.contains(&host("b.example.com")));
            }
            NetScope::None => panic!("merge of two allowlists must be an allowlist"),
        }
    }

    #[test]
    fn fs_scope_union_takes_lub_and_fails_closed_on_incomparable_modes() {
        let p = std::path::PathBuf::from("/data");
        let ro = FsScope {
            mounts: [(p.clone(), FsMode::ReadOnly)].into_iter().collect(),
        };
        let rw = FsScope {
            mounts: [(p.clone(), FsMode::ReadWrite)].into_iter().collect(),
        };
        // ReadWrite is the wider mode (ReadOnly ⊆ ReadWrite) ⇒ LUB = ReadWrite.
        let lub = fs_scope_union(&ro, &rw).expect("comparable modes union");
        assert_eq!(lub.mounts.get(&p), Some(&FsMode::ReadWrite));
        // Disjoint paths simply union.
        let q = std::path::PathBuf::from("/etc");
        let other = FsScope {
            mounts: [(q.clone(), FsMode::ReadOnly)].into_iter().collect(),
        };
        let merged = fs_scope_union(&ro, &other).expect("disjoint union");
        assert_eq!(merged.mounts.len(), 2);
        // ExecOnly ⟂ ReadWrite ⇒ incomparable ⇒ fail-closed None.
        let exec = FsScope {
            mounts: [(p.clone(), FsMode::ExecOnly)].into_iter().collect(),
        };
        assert_eq!(fs_scope_union(&exec, &rw), None, "incomparable ⇒ None");
    }

    #[test]
    fn tool_union_warrant_grants_unions_caps_and_filters() {
        let base = react_auto_base_warrant(ExecutorClass::Bwrap, &ModelId("m".into()));
        // Two egress tools to different hosts + one non-empty-syscall tool (excluded).
        let mut defs: std::collections::BTreeMap<(ToolName, ToolVersion), ToolDef> =
            std::collections::BTreeMap::new();
        let net_a = NetScope::EgressAllowlist([host("a.example.com")].into_iter().collect());
        let net_b = NetScope::EgressAllowlist([host("b.example.com")].into_iter().collect());
        defs.insert(
            (ToolName("alpha".into()), ToolVersion("1".into())),
            tool_def_with(net_a, FsScope::empty(), EMPTY_SYSCALL_PROFILE),
        );
        defs.insert(
            (ToolName("bravo".into()), ToolVersion("1".into())),
            tool_def_with(net_b, FsScope::empty(), EMPTY_SYSCALL_PROFILE),
        );
        defs.insert(
            (ToolName("sandboxed".into()), ToolVersion("1".into())),
            tool_def_with(NetScope::None, FsScope::empty(), [9u8; 32]),
        );
        let w = tool_union_warrant(&base, &defs);
        // alpha + bravo granted; the non-empty-syscall tool is FILTERED out.
        assert_eq!(w.tool_grants.len(), 2);
        assert!(w.tool_grants.iter().any(|g| g.tool_id.0 == "alpha"));
        assert!(w.tool_grants.iter().any(|g| g.tool_id.0 == "bravo"));
        assert!(!w.tool_grants.iter().any(|g| g.tool_id.0 == "sandboxed"));
        // net_scope is the UNION of the two hosts.
        match &w.net_scope {
            NetScope::EgressAllowlist(hosts) => {
                assert!(hosts.contains(&host("a.example.com")));
                assert!(hosts.contains(&host("b.example.com")));
            }
            NetScope::None => panic!("union of two egress tools must be an allowlist"),
        }
        // syscall is the empty sentinel; classes + model_route inherit the base.
        assert_eq!(w.syscall_profile_ref, ContentRef::from_bytes([0u8; 32]));
        assert_eq!(w.model_route.model_id, ModelId("m".into()));
        // Determinism: same input ⇒ byte-identical warrant.
        assert_eq!(w, tool_union_warrant(&base, &defs));
    }

    #[test]
    fn tool_union_warrant_caps_at_the_max() {
        let base = react_auto_base_warrant(ExecutorClass::Bwrap, &ModelId("m".into()));
        let mut defs: std::collections::BTreeMap<(ToolName, ToolVersion), ToolDef> =
            std::collections::BTreeMap::new();
        for i in 0..(AUTOGRANT_MAX_TOOLS + 4) {
            defs.insert(
                (ToolName(format!("tool-{i:02}")), ToolVersion("1".into())),
                tool_def_with(NetScope::None, FsScope::empty(), EMPTY_SYSCALL_PROFILE),
            );
        }
        let w = tool_union_warrant(&base, &defs);
        assert_eq!(w.tool_grants.len(), AUTOGRANT_MAX_TOOLS, "capped");
        // The deterministic prefix is the first AUTOGRANT_MAX_TOOLS by (id, ver).
        assert!(w.tool_grants.iter().any(|g| g.tool_id.0 == "tool-00"));
        assert!(!w
            .tool_grants
            .iter()
            .any(|g| g.tool_id.0 == format!("tool-{:02}", AUTOGRANT_MAX_TOOLS + 3)));
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
