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
    SignatureEntry, SqliteBodyLedger, SqliteGrantLedger, SqliteVersionLedger, TaskSignatureHash,
    VersionLedger, VersionedContent,
};
use kx_content::ContentRef;
use kx_gateway_core::{
    BinderError, BoundRecipe, CatalogSeamError, RecipeBinder, RegisteredSignature,
    SignatureCatalog, SignatureSummaryEntry,
};
use kx_invoke::{bind_snapshot, InvokeError, UseWarrantResolver};
use kx_mote::{ConfigKey, ConfigVal, LogicRef, ToolName};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling, Role,
    WarrantSpec,
};
use kx_workflow::{transform, WorkflowDef};

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

/// The content-ref of the demo recipe's single typed free-param (`topic`).
const TOPIC_SCHEMA_REF: [u8; 32] = [0x2b; 32];

/// A server-provisioned recipe library backing the `Invoke` path, over the
/// durable G1a SQLite ledgers (so a registered recipe survives restart). R2b
/// seeds ONE PURE demo recipe whose step warrant uses the embedded worker's
/// `executor_class` — otherwise a bound run would never lease and `Invoke` would
/// hang. WORLD-MUTATING recipes need the capability path (a later wave).
pub struct DemoLibrary {
    versions: SqliteVersionLedger,
    bodies: SqliteBodyLedger,
    grants: SqliteGrantLedger,
    /// Per-handle binding metadata — one entry per seeded recipe (the demo
    /// `echo` always; the PR-9b real-exec `exec-demo` when a body was located).
    /// `bind` looks the handle up here for its owner-root warrant + free-param
    /// contract; an unknown handle is a uniform `NotAuthorized` (no oracle).
    recipes: Vec<(AssetPath, RecipeMeta)>,
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
        Self::seed(dir, exec_class, parties, None)
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
        Self::seed(dir, exec_class, parties, real_body_ref)
    }

    /// Open the durable ledgers under `dir` and idempotently seed the demo `echo`
    /// recipe (always) plus the real-exec `exec-demo` recipe (when `real_body_ref`
    /// is `Some`). Re-opening on restart is a no-op (content-addressed bodies +
    /// guarded version publish + idempotent grants).
    fn seed(
        dir: &Path,
        exec_class: ExecutorClass,
        parties: &[String],
        real_body_ref: Option<ContentRef>,
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

        Ok(Self {
            versions,
            bodies,
            grants,
            recipes,
        })
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
    lib: DemoLibrary,
}

impl HostRecipeBinder {
    /// Wrap a provisioned [`DemoLibrary`].
    pub fn new(lib: DemoLibrary) -> Self {
        Self { lib }
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
        let bound = bind_snapshot(
            &self.lib.versions,
            &self.lib.bodies,
            &resolver,
            &party_id,
            &asset_path,
            &meta.free_params,
            &DemoSchemaResolver,
            args,
        )
        .map_err(map_invoke_err)?;
        Ok(BoundRecipe {
            recipe_fingerprint: bound.recipe_fingerprint,
            motes: bound.motes,
            terminal_mote_id: bound.terminal_mote_id,
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
/// Exactly three non-empty segments; anything else ⇒ `None`.
fn parse_handle(handle: &str) -> Option<AssetPath> {
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

/// Resolves the demo recipe's `topic` schema-ref to a `Str` param schema.
struct DemoSchemaResolver;

impl SchemaResolver for DemoSchemaResolver {
    fn resolve_schema(&self, schema_ref: &[u8; 32]) -> Option<Vec<u8>> {
        (*schema_ref == TOPIC_SCHEMA_REF)
            .then(|| encode_param_schema(&ParamType::Str { max_len: 4096 }))
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
}
