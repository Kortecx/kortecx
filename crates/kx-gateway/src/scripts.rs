//! The first-class **script** primitive: declare it, register it, run it in a
//! sandbox under the caller's own grants.
//!
//! A script is a named, versioned program the operator registers once and the
//! runtime can then fire as an ordinary tool. It is declarative in the same sense
//! every other tool here is: the registration says what the script *is* and what
//! access it *wants*; it never says what it is allowed to do. Authority is
//! decided at fire time, by the runtime, against the caller's warrant.
//!
//! ## Registration grants no authority
//!
//! Registering a script mints a [`ToolDef`] whose `required_capability` is the
//! script's declared **wish** — the mounts, egress and resource ceiling it says
//! it needs. That declaration is carried, not honoured:
//!
//! 1. the coordinator resolves the tool and puts the declared wish on the
//!    dispatch as `EffectRequest.fs_scope` / `net_scope`;
//! 2. the broker refuses the dispatch unless that wish is a **subset** of the
//!    granting warrant (`CapabilityExceedsWarrant`);
//! 3. only then does [`ScriptCapability::invoke`] run, and it builds the sandbox
//!    profile from the request it was handed — which is, by (2), already within
//!    the caller's grants.
//!
//! So a script that wants more than its caller has is refused before it runs, and
//! the same script under a sufficient grant fires. The client never supplies a
//! warrant; there is no field in which it could.
//!
//! ## What the model controls, and what it does not
//!
//! A model calling a script controls exactly one thing: the `input` string, which
//! arrives on the script's stdin. `argv` and the environment are fixed at
//! registration. That asymmetry is deliberate — argv and env are where a shell
//! script is easiest to subvert, and neither needs to be model-driven for a
//! script to be useful.
//!
//! ## Trust boundary
//!
//! A script's stdout is **untrusted input to the agent**, exactly like connector
//! output or a retrieved document. Nothing here treats it as instructions; it is
//! returned as opaque bytes for the broker to content-address, and whatever reads
//! it downstream is responsible for continuing to treat it as data.
//!
//! ## Execution
//!
//! Scripts never run on the host. Every dispatch goes through
//! [`crate::real_exec::spawn_body_in_sandbox`] — bwrap on Linux, sandbox-exec on
//! macOS — with the bundled shim as the body binary. If the sandbox cannot run,
//! or the shim was never provisioned, the dispatch **refuses**. There is no
//! configuration in which a script runs unsandboxed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_script_runner::{hex32, result_ref_bytes, ScriptDescriptor};
use kx_tool_registry::{
    IdempotencyClass, InputSchema, ParamSpec, ParamType, RegistrationError, SqliteToolRegistry,
    ToolDef, ToolKind, ToolProvenance,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    SecretScope, ToolRequirement, WarrantSpec,
};
use serde::Deserialize;

use crate::real_exec::{bundled_binary_path, run_script_body, ScriptPlumbing};

/// The bundled sandbox shim's binary name (`KX_SCRIPT_RUNNER_PATH` overrides).
const SHIM_BIN: &str = "kx-script-runner";

/// Ceiling on a registered script's source, and on the `input` a caller may pass.
const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
/// Default ceiling on a script's output. Exceeding it REFUSES the call — a
/// truncated result would read as a complete answer.
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
/// Default wall-clock budget for one script run.
const DEFAULT_WALL_CLOCK_MS: u64 = 30_000;

/// A script stages an intent the autonomy approval gate can hold before it
/// commits — the same posture every other world-facing tool takes here.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

// ---------------------------------------------------------------------------
// interpreters
// ---------------------------------------------------------------------------

/// The closed set of interpreters a script may declare.
///
/// Closed on purpose. An open "run this program" field is not a script
/// primitive — it is arbitrary host execution with extra steps, and it would put
/// the choice of what gets mounted execute-only in the caller's hands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Interpreter {
    /// POSIX shell.
    Sh,
    /// CPython 3.
    Python3,
    /// Node.js.
    Node,
}

impl Interpreter {
    /// Parse the wire token. Unknown ⇒ `None` (fail-closed at admission).
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "sh" => Some(Self::Sh),
            "python3" => Some(Self::Python3),
            "node" => Some(Self::Node),
            _ => None,
        }
    }

    /// The wire token, for display and round-tripping.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sh => "sh",
            Self::Python3 => "python3",
            Self::Node => "node",
        }
    }

    /// Every interpreter this build accepts — the admission error names them, so
    /// an operator does not have to guess.
    pub const ALL: &'static [Self] = &[Self::Sh, Self::Python3, Self::Node];

    /// The operator override naming this interpreter's absolute path.
    const fn path_env(self) -> &'static str {
        match self {
            Self::Sh => "KX_SCRIPT_SH_PATH",
            Self::Python3 => "KX_SCRIPT_PYTHON3_PATH",
            Self::Node => "KX_SCRIPT_NODE_PATH",
        }
    }

    /// Well-known absolute paths, tried in order.
    const fn candidates(self) -> &'static [&'static str] {
        match self {
            Self::Sh => &["/bin/sh"],
            Self::Python3 => &[
                "/usr/bin/python3",
                "/usr/local/bin/python3",
                "/opt/homebrew/bin/python3",
            ],
            Self::Node => &[
                "/usr/bin/node",
                "/usr/local/bin/node",
                "/opt/homebrew/bin/node",
            ],
        }
    }

    /// Resolve to an absolute, canonical path on this host, or `None`.
    ///
    /// Canonical because the macOS sandbox matches `subpath` rules against the
    /// kernel's resolved path: a rule written for a symlinked path silently never
    /// matches, and the exec fails with no indication of why.
    pub fn resolve(self) -> Option<PathBuf> {
        if let Some(over) = std::env::var_os(self.path_env()) {
            let path = PathBuf::from(over);
            if path.is_file() {
                return std::fs::canonicalize(path).ok();
            }
        }
        self.candidates()
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .and_then(|p| std::fs::canonicalize(p).ok())
    }

    /// The directory that must be execute-only for the interpreter to launch.
    ///
    /// **The interpreter's DIRECTORY, never the binary itself.** The macOS
    /// profile builder renders every mount as an SBPL `subpath` rule, and
    /// `subpath` matches a directory prefix — so a mount naming the binary
    /// (`/bin/sh`) expands to "everything beneath the directory /bin/sh", which
    /// matches nothing. The grant silently has no effect and the nested exec dies
    /// with a bare `EPERM` that names no rule. Mounting `/bin` is what actually
    /// authorizes running `/bin/sh`.
    ///
    /// The widening this implies is bounded: it permits *executing* other
    /// binaries in the interpreter's own directory, which any shell script could
    /// already invoke through its interpreter. It grants no additional reach into
    /// data — that is the caller's warrant, and nothing here touches it.
    fn exec_dir(resolved: &Path) -> Option<PathBuf> {
        resolved.parent().map(Path::to_path_buf)
    }

    /// Read-only roots the interpreter needs beyond its own binary — its standard
    /// library and shared objects.
    ///
    /// Derived from the resolved binary's installation prefix (`/usr/bin/python3`
    /// ⇒ `/usr`) rather than hard-coded, so a homebrew and a system install both
    /// work. Nonexistent candidates are dropped, keeping the mount set minimal.
    ///
    /// The filesystem root is deliberately excluded. `/bin/sh` has prefix `/`, and
    /// mounting that read-only would hand every script a read of the entire host
    /// — a confidentiality hole arrived at by arithmetic rather than by decision.
    /// A shell needs no stdlib root anyway.
    ///
    /// On Linux this is largely redundant: the bwrap argv already binds `/usr`,
    /// `/lib`, `/lib64` and `/etc` read-only. macOS needs it spelled out, because
    /// SBPL starts from `(deny default)`.
    fn read_roots(resolved: &Path) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(prefix) = resolved.parent().and_then(Path::parent) {
            if prefix.parent().is_some() {
                roots.push(prefix.to_path_buf());
            }
        }
        if let Some(over) = std::env::var_os("KX_SCRIPT_READ_ROOTS") {
            for part in over.to_string_lossy().split(':').filter(|p| !p.is_empty()) {
                roots.push(PathBuf::from(part));
            }
        }
        roots.retain(|p| p.exists() && p.parent().is_some());
        roots.sort();
        roots.dedup();
        roots
    }
}

// ---------------------------------------------------------------------------
// declaration
// ---------------------------------------------------------------------------

/// What a script says it needs. Carried into the [`ToolDef`], enforced by the
/// broker as a subset of the caller's warrant — never granted by being declared.
#[derive(Debug, Clone, Default)]
pub struct ScriptWish {
    /// Filesystem the script wants to reach, beyond the runtime's own scratch.
    pub fs_mounts: BTreeMap<PathBuf, FsMode>,
    /// Hosts the script wants to reach. Empty ⇒ no egress at all.
    pub net_hosts: BTreeSet<Host>,
    /// Wall-clock budget in milliseconds (0 ⇒ [`DEFAULT_WALL_CLOCK_MS`]).
    pub wall_clock_ms: u64,
    /// Memory ceiling in bytes (0 ⇒ unset, the platform default applies).
    pub mem_bytes: u64,
    /// Output ceiling in bytes (0 ⇒ [`DEFAULT_MAX_OUTPUT_BYTES`]).
    pub max_output_bytes: u64,
}

impl ScriptWish {
    fn net_scope(&self) -> NetScope {
        if self.net_hosts.is_empty() {
            NetScope::None
        } else {
            NetScope::EgressAllowlist(self.net_hosts.clone())
        }
    }

    fn ceiling(&self) -> ResourceCeiling {
        ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: self.mem_bytes,
            wall_clock_ms: if self.wall_clock_ms == 0 {
                DEFAULT_WALL_CLOCK_MS
            } else {
                self.wall_clock_ms
            },
            fd_count: 0,
            disk_bytes: 0,
        }
    }

    fn output_cap(&self) -> u64 {
        if self.max_output_bytes == 0 {
            DEFAULT_MAX_OUTPUT_BYTES
        } else {
            self.max_output_bytes
        }
    }
}

/// A script as declared by an operator, before the runtime has resolved anything.
#[derive(Debug, Clone)]
pub struct ScriptDecl {
    /// Identity half — the grant-set key.
    pub name: ToolName,
    /// Identity half.
    pub version: ToolVersion,
    /// Which interpreter runs the source.
    pub interpreter: Interpreter,
    /// The script's source bytes.
    pub source: Vec<u8>,
    /// Free-form; shown to the model in the tool menu, never parsed for
    /// enforcement.
    pub description: String,
    /// Free-form author identifier for the audit trail. Never enforcement.
    pub author: String,
    /// Fixed arguments appended after the script path. NOT model-controlled.
    pub argv: Vec<String>,
    /// Fixed environment. NOT model-controlled; empty ⇒ the script runs with no
    /// environment at all.
    pub env: Vec<(String, String)>,
    /// The declared wish.
    pub wish: ScriptWish,
}

/// Why a script could not be registered. Every variant is a refusal at admission,
/// so an unusable script never reaches the registry.
#[derive(Debug, thiserror::Error)]
pub enum ScriptAdmissionError {
    /// The interpreter token is not in the closed allowlist.
    #[error("unknown interpreter {got:?}; this build accepts {allowed}")]
    UnknownInterpreter {
        /// What was asked for.
        got: String,
        /// The accepted tokens, comma-separated.
        allowed: String,
    },
    /// The interpreter is allowed but not installed on this host.
    #[error("interpreter {0} is not installed on this host")]
    InterpreterUnavailable(&'static str),
    /// The sandbox shim is not present, so nothing could run the script safely.
    #[error(
        "the sandbox shim ({SHIM_BIN}) is not available, so scripts cannot run sandboxed \
         and will not be registered"
    )]
    ShimUnavailable,
    /// Empty or oversized source.
    #[error("script source is {got} bytes; must be 1..={max}")]
    BadSource {
        /// The size given.
        got: usize,
        /// The ceiling.
        max: usize,
    },
    /// Identity halves must be non-empty.
    #[error("script name and version must both be non-empty")]
    BadIdentity,
    /// The content store or registry refused the write.
    #[error("could not store the script: {0}")]
    Storage(String),
    /// The registry refused the registration.
    #[error("could not register the script: {0}")]
    Registration(#[from] RegistrationError),
}

// ---------------------------------------------------------------------------
// the tool definition
// ---------------------------------------------------------------------------

/// Build the [`ToolDef`] a registered script is fired through.
///
/// `kind` is [`ToolKind::LocalScript`], carrying the content ref of the source —
/// so the registry row names the exact bytes that will run, and a changed script
/// is a different registration rather than a silent substitution.
pub fn script_tool_def(decl: &ScriptDecl, script_ref: ContentRef) -> ToolDef {
    ToolDef {
        tool_id: decl.name.clone(),
        tool_version: decl.version.clone(),
        kind: ToolKind::LocalScript { script_ref },
        required_capability: ToolRequirement {
            net_scope_required: decl.wish.net_scope(),
            fs_scope_required: FsScope {
                mounts: decl.wish.fs_mounts.clone(),
            },
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: decl.wish.ceiling(),
        },
        description: decl.description.clone(),
        // World-facing by default: the run stages an intent the approval gate can
        // hold for review before it commits.
        idempotency_class: IdempotencyClass::Staged,
        input_schema: Some(InputSchema {
            params: vec![ParamSpec {
                name: "input".into(),
                ty: ParamType::Str {
                    max_len: MAX_SCRIPT_BYTES,
                },
                required: false,
            }],
            deny_unknown: true,
        }),
    }
}

/// The model-supplied half of a script call. One field, by design.
#[derive(Deserialize)]
struct ScriptArgs {
    #[serde(default)]
    input: String,
}

// ---------------------------------------------------------------------------
// the capability
// ---------------------------------------------------------------------------

/// Fires one registered script, in the platform sandbox, under the caller's
/// grants.
pub struct ScriptCapability {
    name: ToolName,
    version: ToolVersion,
    /// The source bytes' content ref — materialized per call from the store.
    script_ref: ContentRef,
    /// The shim's content ref, used as the body Mote's `logic_ref` so the
    /// executor's body resolver materializes it.
    shim_ref: ContentRef,
    /// Absolute canonical interpreter path, mounted execute-only.
    interpreter_path: PathBuf,
    /// Read-only roots the interpreter needs (its stdlib / shared objects).
    interpreter_read_roots: Vec<PathBuf>,
    /// Fixed argv — never model-controlled.
    argv: Vec<String>,
    /// Fixed environment — never model-controlled.
    env: Vec<(String, String)>,
    ceiling: ResourceCeiling,
    max_output_bytes: u64,
    exec_class: ExecutorClass,
    store: LocalFsContentStore,
}

impl ScriptCapability {
    /// The sandbox warrant for one dispatch: **exactly** what the caller granted.
    ///
    /// `request.fs_scope` / `net_scope` come straight through. The broker's
    /// precheck has already proven both are subsets of the granting warrant, so
    /// copying them is what keeps this layer from widening anything — and it means
    /// reading the warrant answers "what can this script reach" with nothing else
    /// mixed in. The directories the runtime itself needs travel separately, as
    /// [`ScriptPlumbing`].
    fn sandbox_warrant(&self, request: &EffectRequest) -> WarrantSpec {
        WarrantSpec {
            mote_class: MoteClass::Pure,
            nd_class: MoteClass::Pure,
            fs_scope: request.fs_scope.clone(),
            net_scope: request.net_scope.clone(),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            tool_grants: BTreeSet::new(),
            model_route: ModelRoute {
                model_id: ModelId("local".into()),
                max_input_tokens: 0,
                max_output_tokens: 0,
                max_calls: 0,
            },
            resource_ceiling: self.ceiling,
            environment_ref: None,
            executor_class: self.exec_class,
            secret_scope: SecretScope::None,
            ..Default::default()
        }
    }

    /// The directories the mechanism needs — never the caller's authority.
    ///
    /// The interpreter's DIRECTORY, not its binary: the profile builder renders
    /// every mount as an SBPL `subpath`, which matches a directory prefix, so a
    /// rule naming `/bin/sh` expands to "everything under the directory /bin/sh"
    /// and matches nothing at all.
    fn plumbing(&self, src_dir: &Path, out_dir: &Path) -> ScriptPlumbing {
        let mut exec_dirs = Vec::new();
        if let Some(dir) = Interpreter::exec_dir(&self.interpreter_path) {
            exec_dirs.push(dir);
        }
        let mut read_dirs = self.interpreter_read_roots.clone();
        read_dirs.push(src_dir.to_path_buf());
        ScriptPlumbing {
            exec_dirs,
            read_dirs,
            write_dir: out_dir.to_path_buf(),
        }
    }

    /// The synthetic Mote that carries the shim as its body.
    ///
    /// The executor resolves a body from `def.logic_ref`; this is the only field
    /// that matters here. The Mote is never journalled — it exists for the length
    /// of one sandbox spawn.
    fn body_mote(&self) -> Mote {
        let def = MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes(*self.shim_ref.as_bytes()),
            model_id: ModelId("local".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([0; 32]),
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
            InputDataId::from_bytes(*self.script_ref.as_bytes()),
            GraphPosition(b"script".to_vec()),
            smallvec::SmallVec::new(),
        )
    }
}


impl Capability for ScriptCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn version(&self) -> &ToolVersion {
        &self.version
    }

    fn supported_patterns(&self) -> &[EffectPattern] {
        PATTERNS
    }

    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        let input = parse_input_arg(&request.payload)?;

        // Two scratch directories, not one: the script and the descriptor are
        // mounted read-only so a script cannot rewrite itself or its own
        // descriptor mid-run, while the result directory is the single writable
        // mount.
        let src_dir = tempfile::tempdir().map_err(|e| fail(&format!("scratch dir: {e}")))?;
        let out_dir = tempfile::tempdir().map_err(|e| fail(&format!("output dir: {e}")))?;

        let source = self
            .store
            .get(&self.script_ref)
            .map_err(|e| fail(&format!("could not read the script's source: {e}")))?;
        let script_path = src_dir.path().join("script");
        std::fs::write(&script_path, &source)
            .map_err(|e| fail(&format!("could not materialize the script: {e}")))?;
        let out_path = out_dir.path().join("result");

        let descriptor = ScriptDescriptor {
            interpreter_path: self.interpreter_path.to_string_lossy().into_owned(),
            script_path: script_path.to_string_lossy().into_owned(),
            out_path: out_path.to_string_lossy().into_owned(),
            argv: self.argv.clone(),
            stdin_bytes: input.into_bytes(),
            env: self.env.clone(),
            max_output_bytes: self.max_output_bytes,
        };

        let warrant = self.sandbox_warrant(request);
        let plumbing = self.plumbing(src_dir.path(), out_dir.path());
        let mote = self.body_mote();
        // Fail-closed: a sandbox that cannot run is an error, never a fallback to
        // host execution.
        let printed = run_script_body(
            &self.store,
            self.exec_class,
            self.shim_ref,
            &mote,
            &warrant,
            &descriptor.encode(),
            &plumbing,
        )
        .map_err(|e| fail(&format!("sandboxed run failed: {e}")))?;

        // The shim printed a ref; read what it actually wrote and prove the two
        // agree. A truncated, substituted or partially written result cannot
        // survive this, so the result's integrity does not depend on the shim
        // being correct.
        let output = std::fs::read(&out_path)
            .map_err(|e| fail(&format!("the sandboxed run left no readable result: {e}")))?;
        if hex32(&result_ref_bytes(&output)) != printed {
            return Err(fail(
                "the script's result does not match the ref its run reported (rejected)",
            ));
        }
        Ok(output)
    }
}

/// Decode the model's `input` arg. An absent payload is a script called with no
/// input, which is legitimate.
fn parse_input_arg(payload: &[u8]) -> Result<String, CapabilityFailureReason> {
    if payload.is_empty() {
        return Ok(String::new());
    }
    let args: ScriptArgs = serde_json::from_slice(payload)
        .map_err(|e| fail(&format!("bad args: {e}")))?;
    if args.input.len() > MAX_SCRIPT_BYTES {
        return Err(fail(&format!(
            "input is {} bytes, over the {MAX_SCRIPT_BYTES}-byte cap",
            args.input.len()
        )));
    }
    Ok(args.input)
}

fn fail(reason: &str) -> CapabilityFailureReason {
    CapabilityFailureReason::Other(format!("script: {reason}"))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// The shim's content ref, `put` into the store so the executor's body resolver
/// can materialize it. `None` when the binary is not present — in which case no
/// script registers, rather than registering something that cannot run safely.
pub fn provision_shim(store: &LocalFsContentStore) -> Option<ContentRef> {
    let path = bundled_binary_path(SHIM_BIN, "KX_SCRIPT_RUNNER_PATH")?;
    let bytes = std::fs::read(&path)
        .map_err(|error| {
            tracing::warn!(%error, bin = %path.display(), "could not read the sandbox shim");
        })
        .ok()?;
    match store.put(&bytes) {
        Ok(shim_ref) => {
            tracing::info!(bin = %path.display(), "sandbox script shim provisioned");
            Some(shim_ref)
        }
        Err(error) => {
            tracing::warn!(%error, "could not store the sandbox shim");
            None
        }
    }
}

/// Register a declared script: store its source, mint its [`ToolDef`] into the
/// durable registry, and register its firing capability on the broker.
///
/// Refuses — rather than degrading — when the interpreter is unknown or absent,
/// the shim is missing, or the source is empty or oversized. A script that cannot
/// run sandboxed must not appear in the registry at all, because appearing there
/// is what makes it offerable to a model.
pub fn register_script<S: ContentStore + Send + Sync>(
    decl: &ScriptDecl,
    shim_ref: Option<ContentRef>,
    store: &LocalFsContentStore,
    registry: &SqliteToolRegistry,
    broker: &LocalCapabilityBroker<S>,
    exec_class: ExecutorClass,
) -> Result<ContentRef, ScriptAdmissionError> {
    if decl.name.0.trim().is_empty() || decl.version.0.trim().is_empty() {
        return Err(ScriptAdmissionError::BadIdentity);
    }
    if decl.source.is_empty() || decl.source.len() > MAX_SCRIPT_BYTES {
        return Err(ScriptAdmissionError::BadSource {
            got: decl.source.len(),
            max: MAX_SCRIPT_BYTES,
        });
    }
    let shim_ref = shim_ref.ok_or(ScriptAdmissionError::ShimUnavailable)?;
    let interpreter_path = decl
        .interpreter
        .resolve()
        .ok_or(ScriptAdmissionError::InterpreterUnavailable(
            decl.interpreter.as_str(),
        ))?;
    let interpreter_read_roots = Interpreter::read_roots(&interpreter_path);

    let script_ref = store
        .put(&decl.source)
        .map_err(|e| ScriptAdmissionError::Storage(e.to_string()))?;

    let def = script_tool_def(decl, script_ref);
    // Always `HumanAuthored`: a script arrives through an operator-facing RPC, and
    // nothing on this path may self-assert `SelfGenerated` to launder lineage past
    // the review a generated tool owes.
    registry.register_durable(
        def,
        ToolProvenance::HumanAuthored {
            author: decl.author.clone(),
        },
        None,
    )?;

    broker.register_capability(Box::new(ScriptCapability {
        name: decl.name.clone(),
        version: decl.version.clone(),
        script_ref,
        shim_ref,
        interpreter_path,
        interpreter_read_roots,
        argv: decl.argv.clone(),
        env: decl.env.clone(),
        ceiling: decl.wish.ceiling(),
        max_output_bytes: decl.wish.output_cap(),
        exec_class,
        store: store.clone(),
    }));
    tracing::info!(
        script = %decl.name.0,
        version = %decl.version.0,
        interpreter = decl.interpreter.as_str(),
        "script registered (sandboxed; authority decided per call against the caller's warrant)"
    );
    Ok(script_ref)
}

/// The accepted interpreter tokens, for an admission error's `allowed` list.
pub fn allowed_interpreters() -> String {
    Interpreter::ALL
        .iter()
        .map(|i| i.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
