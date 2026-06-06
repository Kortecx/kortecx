//! Real, sandboxed Mote body-execution for the embedded `kx serve` worker (PR-9b).
//!
//! R1 wired the embedded worker to a *deterministic* content-storing executor
//! (`server::storing_executor`) — a faithful demo of the durability
//! spine, but it never spawns a real process. PR-9b closes that gap WITHOUT
//! touching the frozen trio (`kx-executor`/`kx-scheduler`/`kx-inference`): it
//! composes the EXISTING public `kx-executor` surface
//! ([`BwrapExecutor`]/[`MacOsSandboxExecutor`] + [`ContentStoreBodyResolver`])
//! behind a router that the gateway binary owns.
//!
//! `RouterExecutor` dispatches per leased Mote:
//! - a Mote whose `def.logic_ref` is the registered **real body** → run it inside
//!   the platform sandbox (bwrap on Linux, sandbox-exec/Seatbelt on macOS), then
//!   **reconcile** its result bytes into the shared content store so the
//!   coordinator's D55 phantom-ref guard passes at commit;
//! - any other (the bodyless PURE demo `echo`) Mote → the deterministic
//!   `storing` fallback — the exact R1 behavior, untouched.
//!
//! **Fail-closed (Golden Rule 9).** When the sandbox cannot run (no `bwrap`,
//! blocked user-namespaces, non-matching platform), the executor returns
//! [`MoteExecutorError`] and the worker backs off; it NEVER falls back to
//! un-sandboxed host execution. The demo path and the canonical-digest engine
//! path (`kx run`, a separate `TestMoteExecutor::deterministic()`) are untouched.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_executor::{
    BodyResolver, BwrapExecutor, ContentStoreBodyResolver, MacOsSandboxExecutor,
    MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs,
};
use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

/// The content-prefix half of the `pure_body` contract
/// (`kx-executor/examples/pure_body.rs`): the body prints
/// `result_ref = BLAKE3(PURE_BODY_PREFIX ‖ input)` on stdout. By the
/// content-addressing identity, the object whose ref that IS equals exactly
/// `PURE_BODY_PREFIX ‖ input` — so the gateway reconstructs + `put`s it to
/// satisfy the coordinator's D55 ref-existence guard. Kept in lock-step with
/// the example's `b"kx-executor-pure-body-result"` literal.
const PURE_BODY_PREFIX: &[u8] = b"kx-executor-pure-body-result";

/// A [`MoteExecutor`] the gateway binary composes from the public `kx-executor`
/// surface. See the module docs. Holds the shared content store (for the body
/// resolver + the result reconciliation), the registered real-body ref to route
/// on, the embedded worker's executor class, and the deterministic fallback.
pub(crate) struct RouterExecutor {
    /// The shared content store — clones feed [`ContentStoreBodyResolver`] (body
    /// materialization) and back the result reconciliation `put` (D55).
    store: LocalFsContentStore,
    /// The content-ref of the registered real body (== its `logic_ref` bytes). A
    /// leased Mote carrying this `logic_ref` is dispatched to the sandbox. `None`
    /// when no body binary was located at startup (the image ran without it) — the
    /// router then behaves exactly like the R1 storing executor.
    real_body_ref: Option<ContentRef>,
    /// The platform executor class the embedded worker registered as
    /// ([`crate::server::default_executor_class`]).
    exec_class: ExecutorClass,
    /// The deterministic content-storing fallback for bodyless PURE Motes (the
    /// demo `echo`) — the unchanged R1 `server::storing_executor`.
    fallback: Arc<dyn MoteExecutor>,
}

impl RouterExecutor {
    /// Compose the router. `real_body_ref` is the ref the gateway `put` the body
    /// bytes under at startup (`None` ⇒ pure fallback behavior).
    pub(crate) fn new(
        store: LocalFsContentStore,
        real_body_ref: Option<ContentRef>,
        exec_class: ExecutorClass,
        fallback: Arc<dyn MoteExecutor>,
    ) -> Self {
        Self {
            store,
            real_body_ref,
            exec_class,
            fallback,
        }
    }

    /// Whether this Mote's `logic_ref` is the registered real body.
    fn is_real_body(&self, mote: &Mote) -> bool {
        self.real_body_ref.is_some_and(|registered| {
            ContentRef::from_bytes(*mote.def.logic_ref.as_bytes()) == registered
        })
    }

    /// Run the Mote's body inside the platform sandbox, then reconcile its result
    /// bytes into the store (delegates to [`run_body_in_sandbox`], shared with the
    /// startup probe).
    fn run_sandboxed(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        run_body_in_sandbox(&self.store, self.exec_class, mote, warrant, env)
    }
}

/// Run `mote`'s body in the platform sandbox under `warrant`, then reconcile its
/// output into `store`. The body program is materialized from `logic_ref` by
/// [`ContentStoreBodyResolver`]; its per-Mote input is the Mote's identity bytes
/// (deterministic ⇒ exactly-once-per-input). Shared by [`RouterExecutor`] and the
/// startup [`probe_sandbox`].
fn run_body_in_sandbox(
    store: &LocalFsContentStore,
    exec_class: ExecutorClass,
    mote: &Mote,
    warrant: &WarrantSpec,
    env: Option<Rootfs>,
) -> Result<MoteExecutionResult, MoteExecutorError> {
    // 1. Per-Mote input → a tempfile the sandboxed body reads as argv[1]. The
    //    NamedTempFile MUST outlive `run()` (the child reads it), so it stays in
    //    scope until the end of this function.
    let input_bytes = mote.id.as_bytes().to_vec();
    let mut input_file =
        tempfile::NamedTempFile::new().map_err(|e| internal(&format!("input tempfile: {e}")))?;
    input_file
        .write_all(&input_bytes)
        .map_err(|e| internal(&format!("write input: {e}")))?;
    input_file
        .flush()
        .map_err(|e| internal(&format!("flush input: {e}")))?;
    let input_path = input_file.path().to_path_buf();

    // 2. The body resolver materializes `logic_ref` → a chmod-+x tempfile.
    let resolver: Arc<dyn BodyResolver> = Arc::new(ContentStoreBodyResolver::new(store.clone()));

    // 3. The platform sandbox, constructed per-call (so each lease gets its own
    //    per-Mote input). Only the two real-spawn backends are wired into serve;
    //    anything else fails closed.
    let result = match exec_class {
        ExecutorClass::MacOsSandbox => MacOsSandboxExecutor::new()
            .with_body_resolver(resolver)
            .with_input_file(input_path)
            .run(mote, warrant, env),
        ExecutorClass::Bwrap => BwrapExecutor::new()
            .with_body_resolver(resolver)
            .with_input_file(input_path)
            .run(mote, warrant, env),
        other => Err(MoteExecutorError::BackendUnsupported {
            class: other,
            reason: "kx serve wires only the bwrap/macOS sandbox backends".into(),
        }),
    }?;

    // 4. Reconcile (D55): the result object IS `PURE_BODY_PREFIX ‖ input`. `put` it
    //    so the coordinator can verify the committed ref exists, then assert the
    //    body's printed ref matches (a mismatch ⇒ a phantom; fail closed).
    let mut object = Vec::with_capacity(PURE_BODY_PREFIX.len() + input_bytes.len());
    object.extend_from_slice(PURE_BODY_PREFIX);
    object.extend_from_slice(&input_bytes);
    let put_ref = store
        .put(&object)
        .map_err(|e| internal(&format!("reconcile put: {e}")))?;
    if put_ref != result.result_ref {
        return Err(internal(
            "sandbox result_ref != reconstructed object ref (phantom result rejected)",
        ));
    }

    // Keep the input tempfile alive until here (the sandboxed child read it).
    drop(input_file);
    Ok(result)
}

impl MoteExecutor for RouterExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        if self.is_real_body(mote) {
            self.run_sandboxed(mote, warrant, env)
        } else {
            self.fallback.run(mote, warrant, env)
        }
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        // The embedded worker leases on a single class; both the real sandbox path
        // and the storing fallback serve it.
        executor_class == self.exec_class || self.fallback.supports(executor_class)
    }
}

/// A fail-closed [`MoteExecutorError::Internal`] from a `&str` diagnostic.
fn internal(reason: &str) -> MoteExecutorError {
    MoteExecutorError::Internal {
        reason: reason.to_string(),
    }
}

/// One-shot startup probe: run the registered body once through the platform
/// sandbox under the real-exec `warrant`. Returns `true` iff it succeeds
/// end-to-end. The gateway uses this to GATE provisioning of `exec-demo`: when the
/// sandbox cannot run (e.g. Docker's default seccomp blocks the unprivileged user
/// namespace bubblewrap needs, or rlimits are too tight), the recipe is NOT
/// advertised — so an `Invoke` gets a clean refusal instead of a worker that
/// re-leases a never-committable Mote forever. The durable spine + the `echo`
/// recipe are unaffected either way.
pub(crate) fn probe_sandbox(
    store: &LocalFsContentStore,
    body_ref: ContentRef,
    exec_class: ExecutorClass,
    warrant: &WarrantSpec,
) -> bool {
    match run_body_in_sandbox(store, exec_class, &probe_mote(body_ref), warrant, None) {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                %error,
                "PR-9b: sandbox probe failed — kx/recipes/exec-demo NOT provisioned \
                 (durable spine + echo recipe unaffected; enable real-exec per docker-compose.yml)"
            );
            false
        }
    }
}

/// A throwaway PURE Mote whose `logic_ref` is the registered body, used only by
/// [`probe_sandbox`] (its committed output is content-addressed + discarded).
fn probe_mote(body_ref: ContentRef) -> Mote {
    use kx_mote::{
        EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, MoteDef,
        NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use smallvec::SmallVec;
    use std::collections::BTreeMap;

    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(*body_ref.as_bytes()),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition(b"kx-exec-demo-probe".to_vec()),
        SmallVec::new(),
    )
}

/// Locate the sandbox demo-body binary and `put` its bytes into the content
/// store, returning the resulting ref (== the body's `logic_ref`). `None` when
/// no body is found — the gateway then provisions only the demo `echo` recipe
/// and the router behaves exactly like the R1 storing executor.
///
/// The body is the existing `kx-executor` `pure_body` example (the
/// `64-hex-result-on-stdout` contract); reusing it keeps the frozen trio's
/// source diff-empty.
pub(crate) fn register_demo_body(store: &LocalFsContentStore) -> Option<ContentRef> {
    let path = real_body_binary_path()?;
    let bytes = std::fs::read(&path).ok()?;
    match store.put(&bytes) {
        Ok(body_ref) => {
            tracing::info!(
                body_path = %path.display(),
                "PR-9b: real-exec demo body registered (kx/recipes/exec-demo is live)"
            );
            Some(body_ref)
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "PR-9b: could not register the real-exec demo body; exec-demo not provisioned"
            );
            None
        }
    }
}

/// Resolve the demo-body binary path: an explicit `KX_DEMO_BODY_PATH` override
/// first, then the fixed in-image path the Dockerfile `COPY`s it to, then a
/// dev/test convenience search up to the workspace `target/` dir. `None` ⇒ no
/// body available (the FFI-free prebuilt image, e.g.).
fn real_body_binary_path() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("KX_DEMO_BODY_PATH") {
        let path = PathBuf::from(over);
        if path.exists() {
            return Some(path);
        }
    }
    let in_image = PathBuf::from("/usr/local/libexec/kx/pure_body");
    if in_image.exists() {
        return Some(in_image);
    }
    // Dev/test: walk up from the running binary to a `target` dir + look for the
    // built example (`cargo build --example pure_body -p kx-executor`).
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join("examples").join("pure_body");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
