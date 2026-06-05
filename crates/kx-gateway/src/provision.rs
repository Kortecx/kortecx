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
    /// The owner's base warrant the grant fold narrows from (== the recipe step
    /// warrant, so the bound run keeps the worker's `executor_class` and the
    /// `intersect` chain never attempts a widen).
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
        let cat = |e: String| GatewayError::Catalog(e);
        let versions =
            SqliteVersionLedger::open(dir.join("versions.db")).map_err(|e| cat(e.to_string()))?;
        let bodies =
            SqliteBodyLedger::open(dir.join("bodies.db")).map_err(|e| cat(e.to_string()))?;
        let grants =
            SqliteGrantLedger::open(dir.join("grants.db")).map_err(|e| cat(e.to_string()))?;

        let warrant = demo_warrant(exec_class);
        let owner = PartyId::new("kx-gateway");
        let handle = demo_handle()?;
        let asset = AssetRef::Path(handle.clone());

        // (1) Own the asset (idempotent on re-open with the same owner).
        grants
            .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
            .map_err(|e| cat(e.to_string()))?;

        // (2) Publish the executable body (content-addressed, idempotent) + move
        //     the handle to it (guarded so a restart re-seed is a no-op).
        let (manifest_id, _) = bodies
            .publish_body(recipe_body(&warrant))
            .map_err(|e| cat(e.to_string()))?;
        if versions.resolve(&handle).is_none() {
            versions
                .publish(AssetVersion::root(
                    handle.clone(),
                    VersionedContent::Workflow(manifest_id),
                    owner.clone(),
                    Provenance::from_recipe(manifest_id.0),
                ))
                .map_err(|e| cat(e.to_string()))?;
        }

        // (3) Grant Use+Read to every configured party (and the dev principal),
        //     under a runtime scope == the recipe warrant.
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

        Ok(Self {
            versions,
            bodies,
            grants,
            owner_root: warrant,
            free_params: topic_contract(),
        })
    }
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
        let party_id = PartyId::new(party);
        let resolver = HostUseResolver {
            grants: &self.lib.grants,
            owner_root: self.lib.owner_root.clone(),
        };
        let bound = bind_snapshot(
            &self.lib.versions,
            &self.lib.bodies,
            &resolver,
            &party_id,
            &asset_path,
            &self.lib.free_params,
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

/// The PURE demo recipe body: a single content-addressed step that declares a
/// `topic` variable slot (so a bound free-param can overwrite it). PURE so the
/// embedded worker's deterministic content-storing executor runs it.
fn recipe_body(warrant: &WarrantSpec) -> WorkflowDef {
    let mut wf = WorkflowDef::new(0x2b2b_2b2b);
    let mut step = transform(
        LogicRef::from_bytes([0x2b; 32]),
        warrant.model_route.model_id.clone(),
        warrant.clone(),
        ToolName("demo".into()),
    );
    step.config_subset
        .insert(ConfigKey("topic".into()), ConfigVal(Vec::new()));
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
}
