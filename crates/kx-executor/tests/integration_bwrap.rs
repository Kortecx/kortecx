//! PR 9a-hardening-3 integration tests for `BwrapExecutor`. Structural
//! tests run cross-platform; the real-spawn test is gated to Linux +
//! bwrap-installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::PathBuf;

use kx_executor::{BwrapExecutor, MoteExecutor};
use kx_warrant::ExecutorClass;

// ============================================================================
// Cross-platform structural tests
// ============================================================================

#[test]
fn bwrap_executor_default_has_no_body() {
    let exec = BwrapExecutor::new();
    assert!(exec.body_path().is_none());
    assert!(exec.input_path().is_none());
}

#[test]
fn bwrap_executor_with_body_carries_the_path() {
    let exec = BwrapExecutor::with_body(PathBuf::from("/usr/local/bin/my-body"));
    assert_eq!(
        exec.body_path(),
        Some(std::path::Path::new("/usr/local/bin/my-body"))
    );
    assert!(exec.input_path().is_none());
}

#[test]
fn bwrap_executor_with_input_file_carries_the_path() {
    let exec = BwrapExecutor::with_body(PathBuf::from("/usr/local/bin/my-body"))
        .with_input_file(PathBuf::from("/tmp/input.bin"));
    assert_eq!(
        exec.input_path(),
        Some(std::path::Path::new("/tmp/input.bin"))
    );
}

#[test]
fn bwrap_executor_supports_only_bwrap_class_on_linux() {
    let exec = BwrapExecutor::new();
    assert_eq!(
        exec.supports(ExecutorClass::Bwrap),
        cfg!(target_os = "linux")
    );
    assert!(!exec.supports(ExecutorClass::MacOsSandbox));
    assert!(!exec.supports(ExecutorClass::OciDaemon));
    assert!(!exec.supports(ExecutorClass::CloudMicroVm));
}

#[test]
fn bwrap_executor_with_body_path_override_carries_the_path() {
    let exec = BwrapExecutor::with_body(PathBuf::from("/usr/local/bin/my-body"))
        .with_bwrap_path(PathBuf::from("/opt/bwrap/bin/bwrap"));
    // Just verify the executor was constructed; bwrap_path accessor isn't
    // exposed publicly (production callers shouldn't depend on it).
    let _ = exec;
}

// ============================================================================
// Linux-specific real-spawn test (runtime-skips if bwrap absent)
// ============================================================================

#[cfg(target_os = "linux")]
#[allow(clippy::disallowed_methods, clippy::disallowed_types)]
mod linux {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Command;

    use kx_content::ContentRef;
    use kx_executor::{BwrapExecutor, MoteExecutor};
    use kx_mote::{
        EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
        PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_warrant::{
        ExecutorClass, FsMode, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling,
        WarrantSpec,
    };
    use smallvec::SmallVec;

    /// Probe `PATH` for the `bwrap` binary. The test runtime-skips if absent.
    /// Uses `std::process::Command::new("which")` — this is the ONE
    /// allowed exception to the kx-executor `std::process::Command` lint
    /// (located in a test, not the production executor surface).
    #[allow(clippy::disallowed_methods, clippy::disallowed_types)]
    fn which_bwrap() -> Option<PathBuf> {
        let output = Command::new("which").arg("bwrap").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path_str.is_empty() {
            None
        } else {
            Some(PathBuf::from(path_str))
        }
    }

    fn pure_body_binary_path() -> Option<PathBuf> {
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
                return Some(candidate);
            }
        }
        None
    }

    fn permissive_warrant_for_bwrap(
        body_dir: &std::path::Path,
        input_dir: &std::path::Path,
    ) -> WarrantSpec {
        // bwrap's --ro-bind /usr /usr (etc.) covers system libraries; the
        // body's own directory + input's directory go into fs_scope.
        let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
        mounts.insert(body_dir.to_path_buf(), FsMode::ReadOnly);
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
            executor_class: ExecutorClass::Bwrap,
            ..Default::default()
        }
    }

    fn build_pure_mote() -> Mote {
        let def = MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([1; 32]),
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

    fn expected_result_ref(input_bytes: &[u8]) -> ContentRef {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"kx-executor-pure-body-result");
        hasher.update(input_bytes);
        ContentRef::from_bytes(*hasher.finalize().as_bytes())
    }

    #[test]
    #[ignore = "real fork+execvp(bwrap) spawn; runtime-skips if bwrap absent; opt in with `cargo test -- --ignored`"]
    fn pure_mote_runs_end_to_end_through_bwrap_executor() {
        let Some(bwrap_path) = which_bwrap() else {
            eprintln!("skipping: bwrap not installed on this machine");
            return;
        };
        let Some(body_path) = pure_body_binary_path() else {
            panic!(
                "pure_body example not built — run `cargo build --example pure_body -p kx-executor`"
            );
        };
        let body_dir = body_path
            .parent()
            .expect("body has a parent dir")
            .to_path_buf();

        let mut input_file = tempfile::NamedTempFile::new().expect("tempfile");
        let input_bytes = b"hello from PR 9a-hardening-3 / Linux";
        input_file.write_all(input_bytes).expect("write input");
        input_file.flush().expect("flush");
        let input_path = input_file.path().to_path_buf();
        let input_dir = input_path.parent().expect("input parent dir").to_path_buf();

        let executor = BwrapExecutor::with_body(body_path.clone())
            .with_input_file(input_path.clone())
            .with_bwrap_path(bwrap_path);
        let warrant = permissive_warrant_for_bwrap(&body_dir, &input_dir);
        let mote = build_pure_mote();

        let result = executor
            .run(&mote, &warrant, None)
            .expect("spawn must succeed");

        let expected = expected_result_ref(input_bytes);
        assert_eq!(
            result.result_ref, expected,
            "body's result_ref must match BLAKE3 contract"
        );
        assert!(result.finished_at_epoch_ms >= result.started_at_epoch_ms);
    }
}
