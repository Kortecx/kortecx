//! PR 9a-hardening-4 wall-clock-budget integration test.
//!
//! The parent-side watcher thread (introduced in `crate::spawn::spawn_body`'s
//! `wall_clock_ms` parameter) MUST SIGKILL the child after the configured
//! budget elapses. This integration test spawns the `pure_body` example
//! with `--sleep 60000` (60 s) + a `wall_clock_ms: 500` warrant; the
//! executor's `run()` MUST return `MoteExecutorError::WallClockTimedOut
//! { budget_ms: 500 }` within a few seconds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Instant;

    use kx_content::ContentRef;
    use kx_executor::{MacOsSandboxExecutor, MoteExecutor, MoteExecutorError};
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

    fn warrant_with_wall_clock(
        body_path: &std::path::Path,
        input_dir: &std::path::Path,
        wall_clock_ms: u64,
    ) -> WarrantSpec {
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
                wall_clock_ms,
                fd_count: 0,
                disk_bytes: 0,
            },
            environment_ref: None,
            executor_class: ExecutorClass::MacOsSandbox,
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

    #[test]
    #[ignore = "real fork+spawn with --sleep + wall-clock watcher; requires `cargo build --example pure_body -p kx-executor` first; opt in with `cargo test -- --ignored`"]
    fn body_exceeding_wall_clock_ms_is_sigkilled_by_watcher() {
        let body_path = pure_body_binary_path().expect("pure_body example not built");

        let mut input_file = tempfile::NamedTempFile::new().expect("tempfile");
        input_file
            .write_all(b"input bytes")
            .expect("write input bytes");
        input_file.flush().expect("flush");
        let input_path = input_file.path().to_path_buf();
        let input_dir = input_path.parent().expect("input parent dir").to_path_buf();

        // 500ms wall-clock budget; body sleeps 60s. The watcher MUST kill
        // the body well before the 60s sleep completes.
        let warrant = warrant_with_wall_clock(&body_path, &input_dir, 500);
        let mote = build_pure_mote();

        // Construct the executor with the body + the `--sleep 60000` arg.
        // We need a custom MacOsSandboxExecutor that passes the sleep arg;
        // for PR 9a-hardening-4 we extend the existing constructor pattern
        // to accept additional argv. As a stopgap, the test below uses the
        // public `with_body` + `with_input_file` API and relies on the
        // executor's argv builder to pass the input path as argv[1]. To
        // get `--sleep 60000` into the body's argv, we would need a
        // `with_extra_args` constructor on MacOsSandboxExecutor.
        //
        // For PR 9a-hardening-4 we ADD that constructor below. Until the
        // constructor lands, this test asserts the watcher API contract
        // at the SHAPE level (the wall_clock_ms field is read from the
        // warrant + plumbed through to spawn_body's new parameter).
        //
        // The shape verification: construct the executor, call run with
        // a warrant whose wall_clock_ms is 500, observe that the executor
        // attempted to spawn (and returned either Success or
        // WallClockTimedOut depending on whether the body finished in
        // time). For the pure_body without --sleep, 500ms is plenty; the
        // body finishes successfully. To exercise the actual TIMEOUT
        // path, see the `with_extra_args` test that follows once the
        // constructor lands.

        let started = Instant::now();
        let executor = MacOsSandboxExecutor::with_body(body_path.clone())
            .with_input_file(input_path.clone())
            .with_extra_args(vec!["--sleep".into(), "60000".into()]);
        let result = executor.run(&mote, &warrant, None);
        let elapsed = started.elapsed();

        // The watcher MUST fire within ~1 second of the budget elapsing;
        // the parent reaps the SIGKILLed child + returns WallClockTimedOut.
        // We accept up to 5s elapsed for system noise.
        assert!(
            elapsed.as_secs() < 5,
            "wall-clock test should complete in <5s; observed {elapsed:?}"
        );
        match result {
            Err(MoteExecutorError::WallClockTimedOut { budget_ms }) => {
                assert_eq!(budget_ms, 500, "budget_ms should round-trip");
            }
            other => panic!("expected WallClockTimedOut, got {other:?}"),
        }
    }
}

// ============================================================================
// Linux wall-clock variant — PR 9a-hardening-6
// ============================================================================
//
// Mirrors `macos::body_exceeding_wall_clock_ms_is_sigkilled_by_watcher` at
// the `BwrapExecutor` layer. The watcher thread + SIGKILL path are
// cross-platform; the only Linux-specific bit is the `bwrap` invocation.
// Test runtime-skips if `bwrap` is not installed on the runner.

#[cfg(target_os = "linux")]
#[allow(clippy::disallowed_methods, clippy::disallowed_types)]
mod linux {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Instant;

    use kx_content::ContentRef;
    use kx_executor::{BwrapExecutor, MoteExecutor, MoteExecutorError};
    use kx_mote::{
        EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
        PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_warrant::{
        ExecutorClass, FsMode, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling,
        WarrantSpec,
    };
    use smallvec::SmallVec;

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

    fn warrant_with_wall_clock(
        body_dir: &std::path::Path,
        input_dir: &std::path::Path,
        wall_clock_ms: u64,
    ) -> WarrantSpec {
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
                wall_clock_ms,
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

    #[test]
    #[ignore = "real fork+execvp(bwrap)+spawn with --sleep + wall-clock watcher; runtime-skips if bwrap absent; opt in with `cargo test -- --ignored`"]
    fn body_exceeding_wall_clock_ms_is_sigkilled_by_watcher_under_bwrap() {
        let Some(bwrap_path) = which_bwrap() else {
            eprintln!("skipping: bwrap not installed on this runner");
            return;
        };
        let Some(body_path) = pure_body_binary_path() else {
            panic!(
                "pure_body example not built — run `cargo build --example pure_body -p kx-executor`"
            );
        };
        let body_dir = body_path.parent().expect("body parent dir").to_path_buf();

        let mut input_file = tempfile::NamedTempFile::new().expect("tempfile");
        input_file.write_all(b"input").expect("write");
        input_file.flush().expect("flush");
        let input_path = input_file.path().to_path_buf();
        let input_dir = input_path.parent().expect("input parent dir").to_path_buf();

        let warrant = warrant_with_wall_clock(&body_dir, &input_dir, 500);
        let mote = build_pure_mote();
        let executor = BwrapExecutor::with_body(body_path.clone())
            .with_input_file(input_path)
            .with_bwrap_path(bwrap_path)
            .with_extra_args(vec!["--sleep".into(), "60000".into()]);

        let started = Instant::now();
        let result = executor.run(&mote, &warrant, None);
        let elapsed = started.elapsed();

        // The watcher fires at 500ms; the parent reaps the SIGKILLed
        // child + returns WallClockTimedOut. Accept up to 5s for system
        // noise + bwrap setup overhead.
        assert!(
            elapsed.as_secs() < 5,
            "wall-clock test should complete in <5s; observed {elapsed:?}"
        );
        match result {
            Err(MoteExecutorError::WallClockTimedOut { budget_ms }) => {
                assert_eq!(budget_ms, 500);
            }
            other => panic!("expected WallClockTimedOut, got {other:?}"),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[test]
fn integration_wall_clock_unix_only_placeholder() {
    // Wall-clock SIGKILL via nix::sys::signal::kill is Unix-only;
    // non-Unix targets compile this file as a no-op.
}
