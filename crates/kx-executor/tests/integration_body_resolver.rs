//! PR 9a-hardening-5 integration tests for the `BodyResolver` trait +
//! `ContentStoreBodyResolver` impl.
//!
//! Structural tests run cross-platform (verify the materialization
//! contract + chmod +x); the real-spawn variant is opt-in (macOS for
//! now) and proves the executor can spawn a body whose path was
//! resolved from `logic_ref` at run time, not pre-configured via
//! `with_body(path)`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::sync::Arc;

use kx_content::{ContentStore, InMemoryContentStore};
use kx_executor::{BodyResolver, BodyResolverError, ContentStoreBodyResolver, MaterializedBody};
use kx_mote::LogicRef;

// ============================================================================
// Cross-platform structural tests
// ============================================================================

#[test]
fn resolve_returns_materialized_body_pointing_at_the_bytes() {
    let bytes = b"#!/bin/sh\necho hello\n".to_vec();
    let store = InMemoryContentStore::new();
    let content_ref = store.put(&bytes).expect("put");
    let logic_ref = LogicRef::from_bytes(*content_ref.as_bytes());

    let resolver = ContentStoreBodyResolver::new(store);
    let materialized: MaterializedBody = resolver.resolve(&logic_ref).expect("resolve");

    // The materialized file's bytes should equal the input bytes.
    let on_disk = std::fs::read(materialized.path()).expect("read materialized");
    assert_eq!(on_disk, bytes);
}

#[test]
fn resolve_missing_logic_ref_returns_not_in_store() {
    let store = InMemoryContentStore::new();
    let resolver = ContentStoreBodyResolver::new(store);

    let unknown = LogicRef::from_bytes([0xAB; 32]);
    let err = resolver
        .resolve(&unknown)
        .expect_err("missing logic_ref must err");
    assert!(matches!(err, BodyResolverError::NotInStore { .. }));
}

#[cfg(unix)]
#[test]
fn materialized_body_has_executable_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = b"#!/bin/sh\necho hi\n".to_vec();
    let store = InMemoryContentStore::new();
    let content_ref = store.put(&bytes).expect("put");
    let logic_ref = LogicRef::from_bytes(*content_ref.as_bytes());

    let resolver = ContentStoreBodyResolver::new(store);
    let materialized = resolver.resolve(&logic_ref).expect("resolve");

    let mode = std::fs::metadata(materialized.path())
        .expect("metadata")
        .permissions()
        .mode();
    // 0o755 = owner rwx, group rx, other rx. We assert at least the
    // owner-x bit is set (0o100) — the resolver's chmod target.
    assert_ne!(mode & 0o100, 0, "owner-x must be set; got mode {mode:o}");
}

#[test]
fn dropping_materialized_body_removes_the_tempfile() {
    let bytes = b"transient".to_vec();
    let store = InMemoryContentStore::new();
    let content_ref = store.put(&bytes).expect("put");
    let logic_ref = LogicRef::from_bytes(*content_ref.as_bytes());

    let resolver = ContentStoreBodyResolver::new(store);
    let materialized = resolver.resolve(&logic_ref).expect("resolve");
    let path = materialized.path().to_path_buf();
    assert!(
        path.exists(),
        "tempfile should exist while MaterializedBody is alive"
    );

    drop(materialized);
    assert!(
        !path.exists(),
        "tempfile MUST be removed on MaterializedBody Drop"
    );
}

#[test]
fn resolver_object_safety_holds_via_arc_dyn() {
    // `Arc<dyn BodyResolver>` MUST compile (object-safe trait).
    let store = InMemoryContentStore::new();
    let resolver = ContentStoreBodyResolver::new(store);
    let _arc: Arc<dyn BodyResolver> = Arc::new(resolver);
}

// ============================================================================
// Real-spawn variant — opt-in (macOS only for now; Linux equivalent ships
// alongside the Linux wall-clock variant in integration_wall_clock_linux)
// ============================================================================

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;

    use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
    use kx_executor::{ContentStoreBodyResolver, MacOsSandboxExecutor, MoteExecutor};
    use kx_mote::{
        EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
        PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_warrant::{
        ExecutorClass, FsMode, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling,
        WarrantSpec,
    };
    use smallvec::SmallVec;

    fn pure_body_bytes() -> Option<Vec<u8>> {
        let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")?;
        let manifest_path = PathBuf::from(&manifest_dir);
        let workspace_root = manifest_path.parent()?.parent()?;
        for profile in ["debug", "release"] {
            let candidate = workspace_root
                .join("target")
                .join(profile)
                .join("examples")
                .join("pure_body");
            if candidate.exists() {
                return std::fs::read(&candidate).ok();
            }
        }
        None
    }

    fn permissive_warrant(input_dir: &std::path::Path, tempdir: &std::path::Path) -> WarrantSpec {
        // The resolver materializes the body into a tempfile under
        // `tempdir`. Granting `/` ReadOnly covers dyld + libsystem +
        // tempfile reads; the tempdir needs ExecOnly so SBPL's
        // process-exec rule permits running the materialized binary.
        let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
        mounts.insert(PathBuf::from("/"), FsMode::ReadOnly);
        mounts.insert(input_dir.to_path_buf(), FsMode::ReadOnly);
        mounts.insert(tempdir.to_path_buf(), FsMode::ExecOnly);
        WarrantSpec {
            mote_class: MoteClass::Pure,
            nd_class: MoteClass::Pure,
            fs_scope: FsScope { mounts },
            net_scope: NetScope::None,
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            tool_grants: BTreeSet::new(),
            model_route: ModelRoute {
                model_id: ModelId("local".into()),
                max_input_tokens: 0,
                max_output_tokens: 0,
                max_calls: 0,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 30_000,
                fd_count: 0,
                disk_bytes: 0,
            },
            environment_ref: None,
            executor_class: ExecutorClass::MacOsSandbox,
        }
    }

    fn build_pure_mote(logic_ref: LogicRef) -> Mote {
        let def = MoteDef {
            logic_ref,
            model_id: ModelId("local".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: kx_mote::InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([0; 32]),
            GraphPosition(b"root".to_vec()),
            SmallVec::new(),
        )
    }

    #[test]
    #[ignore = "real fork+sandbox_init+execvp with BodyResolver tempfile; requires `cargo build --example pure_body` first; opt in with `cargo test -- --ignored`"]
    fn pure_mote_runs_with_body_resolved_from_content_store() {
        let bytes = pure_body_bytes().expect(
            "pure_body example not built — run `cargo build --example pure_body -p kx-executor`",
        );

        // Put the body bytes into an InMemoryContentStore + capture the
        // logic_ref. The store's ContentRef IS the BLAKE3 hash of the
        // bytes; we use it as the logic_ref.
        let store = InMemoryContentStore::new();
        let content_ref = store.put(&bytes).expect("put bytes");
        let logic_ref = LogicRef::from_bytes(*content_ref.as_bytes());

        // Build the resolver, wrap in Arc<dyn BodyResolver>, give it to
        // the executor.
        let resolver: Arc<dyn kx_executor::BodyResolver> =
            Arc::new(ContentStoreBodyResolver::new(store));

        // Input file the body reads as argv[1].
        let mut input_file = tempfile::NamedTempFile::new().expect("input tempfile");
        let input_bytes = b"hello from hardening-5 BodyResolver";
        input_file.write_all(input_bytes).expect("write input");
        input_file.flush().expect("flush");
        let input_path = input_file.path().to_path_buf();
        let input_dir = input_path.parent().expect("parent").to_path_buf();

        let executor = MacOsSandboxExecutor::new()
            .with_body_resolver(Arc::clone(&resolver))
            .with_input_file(input_path);
        // `std::env::temp_dir()` reports `/var/folders/...` on macOS but
        // the kernel resolves the canonical path to `/private/var/folders/
        // ...` (a symlink). sandbox-exec's `subpath` matcher uses the
        // canonical path, so we must canonicalize before granting
        // ExecOnly — otherwise the SBPL allow rule misses + execvp
        // returns 71 (execvp returned, the marker exit code).
        let tempdir = std::fs::canonicalize(std::env::temp_dir()).expect("canonicalize tempdir");
        let warrant = permissive_warrant(&input_dir, &tempdir);
        let mote = build_pure_mote(logic_ref);

        let result = executor.run(&mote, &warrant, None).expect("run");

        // pure_body computes BLAKE3("kx-executor-pure-body-result" ||
        // input_bytes); compare.
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"kx-executor-pure-body-result");
        hasher.update(input_bytes);
        let expected = ContentRef::from_bytes(*hasher.finalize().as_bytes());
        assert_eq!(result.result_ref, expected);
    }
}
