//! PR 9a-hardening-2 end-to-end integration test: spawn the `pure_body`
//! example binary through `MacOsSandboxExecutor` (fork + `sandbox_init` +
//! execvp + pipe + waitpid). On Linux this test currently routes through
//! `BwrapExecutor` once that backend's real spawn path ships; for now the
//! Linux path returns `BackendUnsupported` (PR 9a-hardening-2 covers macOS;
//! Linux real-spawn follows in PR 9a-hardening-3).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::path::PathBuf;

    use kx_content::ContentRef;
    use kx_executor::{MacOsSandboxExecutor, MoteExecutor};
    use kx_mote::{
        EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
        PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_warrant::{
        ExecutorClass, FsMode, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling,
        WarrantSpec,
    };
    use smallvec::SmallVec;

    fn pure_body_binary_path() -> Option<PathBuf> {
        // Walk up from CARGO_MANIFEST_DIR (crates/kx-executor) to the
        // workspace root + look for the built example. The example is built
        // by `cargo build --example pure_body` before running this test.
        let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")?;
        let manifest_path = PathBuf::from(&manifest_dir);
        let workspace_root = manifest_path.parent()?.parent()?; // crates/ -> root
        for profile in ["debug", "release"] {
            let candidate = workspace_root
                .join("target")
                .join(profile)
                .join("examples")
                .join("pure_body");
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    fn permissive_warrant_with_paths(
        body_path: &std::path::Path,
        input_dir: &std::path::Path,
    ) -> WarrantSpec {
        // Grant ReadOnly on the system root (so dyld can load libsystem +
        // friends) + ExecOnly on the body binary + ReadOnly on the input
        // file's directory. This is permissive for the test environment;
        // real workflows would narrow each axis aggressively.
        let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
        mounts.insert(PathBuf::from("/"), FsMode::ReadOnly);
        mounts.insert(body_path.to_path_buf(), FsMode::ExecOnly);
        mounts.insert(input_dir.to_path_buf(), FsMode::ReadOnly);

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

    fn build_pure_mote() -> Mote {
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([1; 32]),
            model_id: ModelId("local".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([0; 32]),
            GraphPosition(b"root".to_vec()),
            SmallVec::new(),
        )
    }

    fn expected_result_ref(input_bytes: &[u8]) -> ContentRef {
        // Mirror pure_body's hash formula: BLAKE3("kx-executor-pure-body-
        // result" || input_bytes).
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"kx-executor-pure-body-result");
        hasher.update(input_bytes);
        ContentRef::from_bytes(*hasher.finalize().as_bytes())
    }

    #[test]
    #[ignore = "real fork+sandbox_init+execvp spawn; requires the `pure_body` example built first via `cargo build --example pure_body`; opt in with `cargo test -- --ignored`"]
    fn pure_mote_runs_end_to_end_through_macos_sandbox_executor() {
        let body_path = pure_body_binary_path().expect(
            "pure_body example not built — run `cargo build --example pure_body -p kx-executor`",
        );

        // Write the Mote's input bytes to a tempfile the body will read.
        let mut input_file = tempfile::NamedTempFile::new().expect("tempfile");
        let input_bytes = b"hello from PR 9a-hardening-2";
        input_file.write_all(input_bytes).expect("write input");
        input_file.flush().expect("flush");
        let input_path = input_file.path().to_path_buf();
        let input_dir = input_path.parent().expect("parent dir").to_path_buf();

        // Construct the executor + warrant.
        let executor =
            MacOsSandboxExecutor::with_body(body_path.clone()).with_input_file(input_path.clone());
        let warrant = permissive_warrant_with_paths(&body_path, &input_dir);
        let mote = build_pure_mote();

        // Run.
        let result = executor
            .run(&mote, &warrant, None)
            .expect("spawn must succeed");

        // The body computed BLAKE3("kx-executor-pure-body-result" || input);
        // the executor parsed 64 hex chars; the result_ref MUST match.
        let expected = expected_result_ref(input_bytes);
        assert_eq!(
            result.result_ref, expected,
            "body's result_ref must match BLAKE3 contract"
        );
        assert!(result.finished_at_epoch_ms >= result.started_at_epoch_ms);
    }
}

// Non-macOS targets compile this file as a no-op (the test functions are
// gated under `#[cfg(target_os = "macos")]`). This keeps `cargo test`
// behavior uniform across platforms.
#[cfg(not(target_os = "macos"))]
#[test]
fn integration_real_spawn_macos_only_placeholder() {
    // PR 9a-hardening-2 ships the macOS real-spawn path; the Linux real-spawn
    // path (fork + execvp(bwrap) + waitpid) lands in PR 9a-hardening-3 along
    // with bwrap-binary probing + a bwrap-installed CI gate.
}
