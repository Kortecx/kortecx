//! `kx-executor` — single-Mote executor + `MoteExecutor` trait + four backend
//! impl/stubs + `LocalResourceManager` + submission-time refusal predicates +
//! fact-zero protocol. **PR 9a skeleton scope.**
//!
//! # PR 9a vs PR 9b vs PR 9a-hardening
//!
//! PR 9a ships:
//! - The `MoteExecutor` trait + four backend types: `BwrapExecutor` (Linux),
//!   `MacOsSandboxExecutor` (macOS), `OciDaemonExecutor` (stub),
//!   `CloudMicroVmExecutor` (refusal). All four backends are SKELETONS —
//!   `run()` returns `BackendUnsupported`. The trait surface is real.
//! - `default_executor()` factory (platform-conditional via `target_os` cfgs).
//! - `LocalResourceManager` skeleton (in-memory accounting of acquired slots;
//!   real cgroup v2 / `setrlimit` ships in PR 9a-hardening).
//! - REAL submission-time refusal predicates: R-1, R-2, R-3, R-4, R-5, R-6,
//!   R-7, R-8, R-8b, R-9, `ValidatorTypeError`, `AttemptedWiden`. R-10..R-13
//!   are reserved for PR 9b.
//! - REAL fact-zero protocol (D34): `write_fact_zero` writes the synthetic
//!   Committed entry as the first journal write of a run.
//! - PURE-Mote lifecycle orchestration with a `TestMoteExecutor` test
//!   backend that exercises the seams without depending on bwrap/sandbox-
//!   exec being installed.
//!
//! PR 9a-hardening adds:
//! - Real bwrap argv builder + `execvp` on Linux.
//! - Real `posix_spawn` + `sandbox_init`-equivalent on macOS.
//! - Real cgroup v2 file I/O + `setrlimit` + wall-clock timer.
//! - `crates/kx-executor/src/backends/macos_sandbox_template.sb`
//!   `include_str!`-embedded.
//! - A small `kx-executor-pure-body` example binary exercised by the
//!   PURE-Mote integration test.
//!
//! PR 9b adds:
//! - The EffectStaged-then-Committed commit protocol (D38 §2b).
//! - R-10..R-13 refusal predicates.
//! - 9-cell cross-product recovery integration tests at the executor layer.
//! - Test A re-use from `kx-content` + Test B (executor commit-protocol
//!   trust).
//! - WORLD-MUTATING Mote crash-recovery end-to-end.
//!
//! # Architecture
//!
//! Two non-overlapping seams (per `docs/design/capability-broker.md` §3 and
//! `docs/design/resource-manager.md` §3):
//! - **Capability broker** (D24, `kx-capability`) — workflow-declared
//!   effects. The executor invokes `CapabilityBroker::dispatch` for every
//!   tool call. NOT consumed in PR 9a (PURE Motes have no effects).
//! - **Resource manager** (D25, this crate) — runtime self-management.
//!   `LocalResourceManager::acquire`/`release` bracket every Mote execution.
//!
//! The `MoteExecutor` trait is a third seam at a lower layer: the per-Mote
//! process-environment fence. Not a generic "executor" in the threading-pool
//! sense; specifically the sandboxing apparatus.
//!
//! # `std::process::Command` forbidden
//!
//! Per `02-crate-specs.md` §`kx-executor` DoD: NO `std::process::Command`
//! shell-outs in this crate. The `crates/kx-executor/clippy.toml` enforces
//! this at compile time (PR 9a-hardening will wire the lint; for PR 9a the
//! grep audit is the manual check).
//!
//! # Examples
//!
//! Construct the platform default executor + a resource manager:
//!
//! ```
//! use kx_executor::{default_executor, LocalResourceManager};
//!
//! let executor = default_executor();
//! let _rm = LocalResourceManager::dev_defaults();
//! // PR 9a's `executor.run(...)` returns BackendUnsupported (skeleton);
//! // PR 9a-hardening's spawn path makes it functional.
//! let _ = executor;
//! ```
//!
//! Validate a workflow submission for R-1..R-9 + R-8b refusal:
//!
//! ```
//! use kx_executor::{validate_submission, WorkflowSubmission};
//! use std::collections::BTreeMap;
//!
//! let submission = WorkflowSubmission {
//!     run_id: [0u8; 32],
//!     master_warrant: example_warrant(),
//!     motes: BTreeMap::new(),
//!     accept_at_least_once: BTreeMap::new(),
//! };
//! // Empty submission has no R-* triggers; returns Ok.
//! assert!(validate_submission(&submission).is_ok());
//!
//! # fn example_warrant() -> kx_warrant::WarrantSpec {
//! #     use std::collections::BTreeSet;
//! #     kx_warrant::WarrantSpec {
//! #         mote_class: kx_warrant::MoteClass::Pure,
//! #         nd_class: kx_warrant::MoteClass::Pure,
//! #         fs_scope: kx_warrant::FsScope::empty(),
//! #         net_scope: kx_warrant::NetScope::None,
//! #         syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
//! #         tool_grants: BTreeSet::new(),
//! #         model_route: kx_warrant::ModelRoute {
//! #             model_id: kx_mote::ModelId("local".into()),
//! #             max_input_tokens: 0, max_output_tokens: 0, max_calls: 0,
//! #         },
//! #         resource_ceiling: kx_warrant::ResourceCeiling {
//! #             cpu_milli: 0, mem_bytes: 0, wall_clock_ms: 0,
//! #             fd_count: 0, disk_bytes: 0,
//! #         },
//! #         environment_ref: None,
//! #         executor_class: kx_warrant::ExecutorClass::Bwrap,
//! #     }
//! # }
//! ```
//!
//! Write fact-zero (D34) and obtain the synthetic seed `MoteId`:
//!
//! ```
//! use kx_content::{ContentRef, InMemoryContentStore};
//! use kx_executor::{write_fact_zero, SeedPayload};
//! use kx_journal::InMemoryJournal;
//!
//! let store = InMemoryContentStore::new();
//! let journal = InMemoryJournal::new();
//! let seed = SeedPayload {
//!     run_id: [1u8; 16],
//!     task: "demo".into(),
//!     system_prompt: None,
//!     workflow_def_ref: ContentRef::from_bytes([0; 32]),
//!     submitted_at_ms: 0,
//! };
//! # let warrant = example_warrant();
//! let mote_id = write_fact_zero(&store, &journal, &seed, &warrant).expect("seed write");
//! assert_eq!(mote_id, kx_executor::seed_mote_id(&seed.run_id));
//!
//! # fn example_warrant() -> kx_warrant::WarrantSpec {
//! #     use std::collections::BTreeSet;
//! #     kx_warrant::WarrantSpec {
//! #         mote_class: kx_warrant::MoteClass::Pure,
//! #         nd_class: kx_warrant::MoteClass::Pure,
//! #         fs_scope: kx_warrant::FsScope::empty(),
//! #         net_scope: kx_warrant::NetScope::None,
//! #         syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
//! #         tool_grants: BTreeSet::new(),
//! #         model_route: kx_warrant::ModelRoute {
//! #             model_id: kx_mote::ModelId("local".into()),
//! #             max_input_tokens: 0, max_output_tokens: 0, max_calls: 0,
//! #         },
//! #         resource_ceiling: kx_warrant::ResourceCeiling {
//! #             cpu_milli: 0, mem_bytes: 0, wall_clock_ms: 0,
//! #             fd_count: 0, disk_bytes: 0,
//! #         },
//! #         environment_ref: None,
//! #         executor_class: kx_warrant::ExecutorClass::Bwrap,
//! #     }
//! # }
//! ```
//!
//! Run a PURE Mote end-to-end via the test executor:
//!
//! ```
//! use kx_content::InMemoryContentStore;
//! use kx_executor::{LocalResourceManager, TestMoteExecutor, run_pure_mote};
//! use kx_journal::InMemoryJournal;
//! use kx_mote::{ConfigKey, ConfigVal, EffectPattern, GraphPosition, InputDataId,
//!     LogicRef, ModelId, Mote, MoteDef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION};
//! use smallvec::SmallVec;
//! use std::collections::BTreeMap;
//!
//! let store = InMemoryContentStore::new();
//! let journal = InMemoryJournal::new();
//! let rm = LocalResourceManager::dev_defaults();
//! let executor = TestMoteExecutor::deterministic();
//!
//! let def = MoteDef {
//!     logic_ref: LogicRef::from_bytes([1; 32]),
//!     model_id: ModelId("local".into()),
//!     prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
//!     tool_contract: BTreeMap::new(),
//!     nd_class: NdClass::Pure,
//!     config_subset: BTreeMap::new(),
//!     effect_pattern: EffectPattern::IdempotentByConstruction,
//!     critic_for: None,
//!     is_topology_shaper: false,
//!     inference_params: kx_mote::InferenceParams::default(),
//!     critic_check: None,
//!     schema_version: MOTE_DEF_SCHEMA_VERSION,
//! };
//! let mote = Mote::new(
//!     def,
//!     InputDataId::from_bytes([0; 32]),
//!     GraphPosition(b"root".to_vec()),
//!     SmallVec::new(),
//! );
//!
//! # let warrant = example_warrant();
//! let commit = run_pure_mote(&mote, &warrant, &journal, &rm, &executor).expect("run");
//! assert_eq!(commit.mote_id, mote.id);
//! let _ = store;
//!
//! # fn example_warrant() -> kx_warrant::WarrantSpec {
//! #     use std::collections::BTreeSet;
//! #     kx_warrant::WarrantSpec {
//! #         mote_class: kx_warrant::MoteClass::Pure,
//! #         nd_class: kx_warrant::MoteClass::Pure,
//! #         fs_scope: kx_warrant::FsScope::empty(),
//! #         net_scope: kx_warrant::NetScope::None,
//! #         syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
//! #         tool_grants: BTreeSet::new(),
//! #         model_route: kx_warrant::ModelRoute {
//! #             model_id: kx_mote::ModelId("local".into()),
//! #             max_input_tokens: 0, max_output_tokens: 0, max_calls: 0,
//! #         },
//! #         resource_ceiling: kx_warrant::ResourceCeiling {
//! #             cpu_milli: 0, mem_bytes: 0, wall_clock_ms: 0,
//! #             fd_count: 0, disk_bytes: 0,
//! #         },
//! #         environment_ref: None,
//! #         executor_class: kx_warrant::ExecutorClass::Bwrap,
//! #     }
//! # }
//! ```
//!
//! Pick a backend by `ExecutorClass`:
//!
//! ```
//! use kx_executor::{executor_for_class};
//! use kx_warrant::ExecutorClass;
//!
//! // Construction is always allowed; per-platform refusal lands at `run()`.
//! let _bwrap = executor_for_class(ExecutorClass::Bwrap);
//! let _mac = executor_for_class(ExecutorClass::MacOsSandbox);
//! let _oci = executor_for_class(ExecutorClass::OciDaemon);
//! let _cloud = executor_for_class(ExecutorClass::CloudMicroVm);
//! ```

// PR 9a-hardening-2 lifts the previous `#![forbid(unsafe_code)]` to a
// per-block discipline (matching the kx-llamacpp precedent): the `spawn`
// submodule contains the post-fork pre-exec systems calls (sandbox_init,
// execvp, dup2, _exit) — each unsafe block carries a `// SAFETY:` comment
// naming the invariant it relies on; the rest of the crate stays
// unsafe-free at the source level. `unsafe_op_in_unsafe_fn` is denied so
// every unsafe operation inside an `unsafe fn` still requires its own
// `unsafe { }` block (clippy-belt-and-suspenders).
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::pedantic))]
// PR 9a documented-infallible production sites (mirrors kx-mote / kx-warrant /
// kx-tool-registry / kx-capability `#![allow(clippy::expect_used)]` precedent):
//   - `fact_zero::SeedPayload::{identity_bytes, full_bytes}` call
//     `bincode::serde::encode_to_vec(...).expect(...)` on `Serialize` types
//     containing only owned `[u8; N]` / `String` / `Option<String>` /
//     `ContentRef` — infallible to encode (no floats, no non-encodable types).
//   - `LocalResourceManager` carries `.lock().map_err(...)` — no `expect()`
//     in production paths; the allow scopes the bincode sites only.
// TODO(PR 9a-hardening cleanup): migrate the bincode sites to a workspace-
// shared encode helper returning a typed error; or to `try_*` constructors.
#![allow(clippy::expect_used)]
// PR 9a doc-discipline carve-outs. The crate carries substantial public-API
// docs (≥6 doctests on the public surface) + module-level error/panic prose;
// the per-site `# Errors` / `# Panics` sections add little value at this
// skeleton stage. The PR 9a-hardening review converts these to per-site
// sections + intra-doc links to error variants (matches the kx-capability
// `dispatch` doc shape).
// TODO(PR 9a-hardening): replace each allow below with per-site `# Errors`
// / `# Panics` sections + intra-doc links to error variants.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::type_complexity,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value
)]

pub mod backends;
pub mod body_resolver;
#[cfg(target_os = "linux")]
pub mod cgroup_v2;
pub mod commit_protocol;
pub mod executor_trait;
pub mod fact_zero;
pub mod factory;
pub mod lifecycle;
pub mod native_critic;
pub mod resource_manager;
pub mod verify;
// The spawn module ships shared Unix primitives (fork + pipe + dup2 +
// execvp + waitpid + setrlimit pre-exec hook). Linux's `BwrapExecutor`
// (PR 9a-hardening-3) and macOS's `MacOsSandboxExecutor` (PR 9a-hardening-2)
// both consume it.
#[cfg(unix)]
pub(crate) mod spawn;

// Re-exports — the executor's stable public API.
pub use body_resolver::{
    BodyResolver, BodyResolverError, ContentStoreBodyResolver, MaterializedBody,
};
pub use commit_protocol::{
    CommitInput, CommitProtocol, CommitProtocolError, StandardCommitProtocol,
};
pub use executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
pub use fact_zero::{
    seed_idempotency_key, seed_mote_id, write_fact_zero, FactZeroError, SeedPayload,
};
pub use factory::{default_executor, executor_for_class};
pub use lifecycle::{
    redispatch_wm_mote, run_pure_mote, run_wm_mote, LifecycleCommit, LifecycleError,
    TestMoteExecutor, WmLifecycleCommit, WmRedispatchOracle,
};
pub use native_critic::run_native_critic_mote;
// Submission-time refusal vocabulary + predicates. Extracted to the `kx-refusal`
// leaf crate (M1.3) so the control-plane `kx-coordinator` can enforce refusals
// without linking this crate's inference stack; re-exported here so existing
// callers (`kx_executor::validate_submission`, etc.) and the lifecycle path are
// unchanged.
pub use kx_refusal::{
    refusal_from_narrowing, validate_mote_submission, validate_submission,
    validate_submission_with_idempotency, SubmissionRefusal, ToolResolution, WorkflowSubmission,
};
pub use resource_manager::{LocalResourceManager, ResourceError, ResourceManager, Slot};
pub use verify::{verify_pure_rerun, VerifyError, VerifyOutcome};

// Backend re-exports — callers may select a specific backend explicitly.
pub use backends::bwrap::BwrapExecutor;
pub use backends::cloud_microvm::CloudMicroVmExecutor;
pub use backends::macos_sandbox::{profile_from_warrant, MacOsSandboxExecutor, SbplProfile};
pub use backends::oci_daemon::OciDaemonExecutor;
