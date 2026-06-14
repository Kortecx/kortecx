//! Real, sandboxed Mote body-execution SEAM for the embedded `kx serve` worker.
//!
//! Composes the EXISTING public `kx-executor` surface
//! ([`BwrapExecutor`]/[`MacOsSandboxExecutor`] + [`ContentStoreBodyResolver`])
//! behind a router the gateway binary owns — WITHOUT touching the frozen trio
//! (`kx-executor`/`kx-scheduler`/`kx-inference`).
//!
//! `RouterExecutor` dispatches per leased Mote:
//! - a Mote whose `def.logic_ref` is a registered **real body** → run it inside
//!   the platform sandbox (bwrap on Linux, sandbox-exec/Seatbelt on macOS), then
//!   **reconcile** its result bytes into the shared content store so the
//!   coordinator's D55 phantom-ref guard passes at commit;
//! - any other (a bodyless PURE Mote, e.g. `echo`) → the honest passthrough
//!   fallback (GR15 — it commits the Mote's real input, never a placeholder).
//!
//! In the OSS serve path the router is wired with `real_body_ref = None` (no body
//! binary is provisioned — script/tool execution is OSS-scoped-out, D141.4), so
//! every Mote takes the passthrough fallback. The sandbox-routing machinery is
//! retained as a stable seam: a later tools/scripts batch re-enables body
//! registration with ZERO change here.
//!
//! **Fail-closed (Golden Rule 9).** When the sandbox cannot run (no `bwrap`,
//! blocked user-namespaces, non-matching platform), the executor returns
//! [`MoteExecutorError`] and the worker backs off; it NEVER falls back to
//! un-sandboxed host execution. The canonical-digest engine path (`kx run`, a
//! separate `TestMoteExecutor::deterministic()`) is untouched.

use std::io::Write as _;
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
    /// The content-ref of a registered real body (== its `logic_ref` bytes). A
    /// leased Mote carrying this `logic_ref` is dispatched to the sandbox. `None`
    /// in the OSS serve path (no body provisioned) — every Mote then takes the
    /// honest passthrough fallback.
    real_body_ref: Option<ContentRef>,
    /// The platform executor class the embedded worker registered as
    /// ([`crate::server::default_executor_class`]).
    exec_class: ExecutorClass,
    /// The honest passthrough fallback for bodyless PURE Motes (e.g. `echo`) —
    /// commits the Mote's real input (GR15), never a fabricated placeholder.
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
/// (deterministic ⇒ exactly-once-per-input). Used by [`RouterExecutor`] when a real
/// body is registered.
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
