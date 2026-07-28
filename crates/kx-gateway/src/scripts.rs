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
//! the sandbox seam in `real_exec` — bwrap on Linux, sandbox-exec on
//! macOS — with the bundled shim as the body binary. If the sandbox cannot run,
//! or the shim was never provisioned, the dispatch **refuses**. There is no
//! configuration in which a script runs unsandboxed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_gateway_core::{RegisteredScriptEntry, ScriptAdmin, ScriptAdminError, ScriptRegistration};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_script_runner::{hex32, result_ref_bytes, ScriptDescriptor};
use kx_tool_registry::{
    IdempotencyClass, InputSchema, ParamSpec, ParamType, RegistrationError, SqliteToolRegistry,
    ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    SecretScope, ToolRequirement, WarrantSpec,
};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::real_exec::{bundled_binary_path, run_script_body, ScriptPlumbing};

/// The bundled sandbox shim's binary name (`KX_SCRIPT_RUNNER_PATH` overrides).
const SHIM_BIN: &str = "kx-script-runner";

/// Ceiling on a registered script's source, and on the `input` a caller may pass.
const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
/// Default ceiling on a script's output. Exceeding it REFUSES the call — a
/// truncated result would read as a complete answer.
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
/// Budget for an interpreter probe at registration — short, because a probe that
/// hangs must not hang the registration.
const PROBE_BUDGET_MS: u64 = 10_000;
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

    /// A trivial script that prints `ok`, used to prove a candidate actually runs.
    const fn probe_source(self) -> &'static str {
        match self {
            Self::Sh => "printf ok",
            Self::Python3 => "print('ok', end='')",
            Self::Node => "process.stdout.write('ok')",
        }
    }

    /// Every candidate that exists on this host, canonical, in priority order:
    /// the operator override, then `PATH`, then the well-known absolute paths.
    ///
    /// **`PATH` is searched, not just the fixed list.** Version managers — nvm,
    /// pyenv, asdf, conda — install outside every system prefix, and they are the
    /// normal case on a developer machine, not an exotic one. A fixed list alone
    /// tells someone with a working `node` that node is not installed.
    ///
    /// Existence only; whether a candidate actually WORKS is settled by probing
    /// it, because on this platform some of them do not.
    ///
    /// Canonical because the macOS sandbox matches `subpath` rules against the
    /// kernel's resolved path: a rule written for a symlinked path silently never
    /// matches, and the exec fails with no indication of why.
    fn resolved_candidates(self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let push = |path: PathBuf, out: &mut Vec<PathBuf>| {
            if path.is_file() {
                if let Ok(canonical) = std::fs::canonicalize(path) {
                    if !out.contains(&canonical) {
                        out.push(canonical);
                    }
                }
            }
        };
        if let Some(over) = std::env::var_os(self.path_env()) {
            push(PathBuf::from(over), &mut out);
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                push(dir.join(self.as_str()), &mut out);
            }
        }
        for candidate in self.candidates() {
            push(PathBuf::from(candidate), &mut out);
        }
        out
    }

    /// The first candidate that exists. Kept for callers that only need a path
    /// (display, tests); registration uses the PROBED resolution instead.
    pub fn resolve(self) -> Option<PathBuf> {
        self.resolved_candidates().into_iter().next()
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
    /// Wall-clock budget in milliseconds (0 ⇒ the host default, 30s).
    pub wall_clock_ms: u64,
    /// Memory ceiling in bytes (0 ⇒ unset, the platform default applies).
    pub mem_bytes: u64,
    /// Output ceiling in bytes (0 ⇒ the host default, 1 MiB).
    pub max_output_bytes: u64,
}

impl ScriptDecl {
    /// The egress scope this declaration compiles to — exposed so a test can
    /// build the matching caller grant without duplicating the mapping (a second
    /// copy would let the two drift and quietly stop testing the same thing).
    #[must_use]
    pub fn wish_net_scope_for_test(&self) -> NetScope {
        self.wish.net_scope()
    }
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
    /// The interpreter is allowed but no candidate on this host could run a
    /// script under the sandbox. Carries WHY per candidate: "not installed" is a
    /// misleading thing to tell someone who has it installed and whose real
    /// problem is that it will not start confined.
    #[error("no usable {interpreter} on this host: {detail}")]
    InterpreterUnavailable {
        /// Which interpreter was asked for.
        interpreter: &'static str,
        /// One line per candidate tried, with the reason it was rejected.
        detail: String,
    },
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
    /// The declaration names a ceiling this host's sandbox cannot apply.
    ///
    /// Fail closed rather than accept it. A ceiling that is declared, carried through
    /// the registry, shown in every listing, and then ignored by the spawn is worse than
    /// one that was never offered: it reads as a constraint everywhere it appears. This
    /// was the state of an egress allowlist on Linux — the script registered, ran, and
    /// had the network wide open, and nothing anywhere said so.
    #[error("this host cannot enforce a ceiling the script declared: {0}")]
    UnenforceableCeiling(String),
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
        interpreter_plumbing(
            &self.interpreter_path,
            &self.interpreter_read_roots,
            src_dir,
            out_dir,
        )
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

        // Canonical paths in the descriptor: the sandbox matches rules against the
        // kernel's resolved path, and an interpreter resolves whatever it is
        // handed. Passing `/var/...` when the rules say `/private/var/...` makes
        // the interpreter stat a path no rule covers.
        let descriptor = ScriptDescriptor {
            interpreter_path: self.interpreter_path.to_string_lossy().into_owned(),
            script_path: canonical_or(&script_path).to_string_lossy().into_owned(),
            out_path: canonical_or(out_dir.path())
                .join("result")
                .to_string_lossy()
                .into_owned(),
            argv: self.argv.clone(),
            stdin_bytes: input.into_bytes(),
            env: self.env.clone(),
            // The ceiling travels to the shim, which is the interpreter's direct
            // parent and can therefore stop precisely what overran. The host keeps
            // its own outer deadline as a backstop for a wedged shim.
            wall_clock_ms: self.ceiling.wall_clock_ms,
            mem_bytes: self.ceiling.mem_bytes,
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
    let args: ScriptArgs =
        serde_json::from_slice(payload).map_err(|e| fail(&format!("bad args: {e}")))?;
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

/// The mounts an interpreter needs, shared by a real dispatch and the registration
/// probe so the probe proves the same conditions the run will get.
///
/// The interpreter's PREFIX is execute-as-well-as-readable, not merely readable: a
/// framework-packaged interpreter re-execs a bundled binary inside its own prefix
/// during startup, so read alone gets `posix_spawn: Undefined error: 0` — a
/// message that names neither the path nor the permission.
fn interpreter_plumbing(
    interpreter_path: &Path,
    read_roots: &[PathBuf],
    src_dir: &Path,
    out_dir: &Path,
) -> ScriptPlumbing {
    let mut exec_dirs = Vec::new();
    if let Some(dir) = Interpreter::exec_dir(interpreter_path) {
        exec_dirs.push(dir);
    }
    exec_dirs.extend(read_roots.iter().cloned());
    // Ancestors of the SCRIPT and the OUTPUT too, not just the interpreter. An
    // interpreter resolves its main module to a real path before running it, and
    // that walk stats every component above the script — which lives in a scratch
    // directory whose ancestors nothing else had a reason to grant.
    let mut metadata_paths = ancestors_of(interpreter_path);
    for dir in [src_dir, out_dir] {
        for ancestor in ancestors_of(&canonical_or(dir).join("x")) {
            if !metadata_paths.contains(&ancestor) {
                metadata_paths.push(ancestor);
            }
        }
    }
    ScriptPlumbing {
        exec_dirs,
        read_dirs: vec![src_dir.to_path_buf()],
        metadata_paths,
        write_dir: out_dir.to_path_buf(),
    }
}

/// Canonicalize, falling back to the path as given when it cannot be resolved.
fn canonical_or(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Every directory above `path`, excluding the filesystem root.
///
/// The root is excluded deliberately: it needs no grant, and naming it would put
/// a rule about `/` in a profile whose whole point is that nothing is granted by
/// default.
fn ancestors_of(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir.parent().is_none() {
            break;
        }
        out.push(dir.to_path_buf());
        current = dir.parent();
    }
    out
}

/// Find an interpreter that actually runs a script under this platform's sandbox.
///
/// Returns the working path and its read roots, or `None` when no candidate
/// survives — which registration turns into a refusal, because a script whose
/// interpreter cannot run is not a script the runtime should offer a model.
fn probe_interpreter(
    interpreter: Interpreter,
    shim_ref: ContentRef,
    store: &LocalFsContentStore,
    exec_class: ExecutorClass,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let candidates = interpreter.resolved_candidates();
    if candidates.is_empty() {
        return Err("no candidate found on PATH or in the well-known locations".into());
    }
    let mut rejected = Vec::new();
    for candidate in candidates {
        let read_roots = Interpreter::read_roots(&candidate);
        match run_probe(
            &candidate,
            &read_roots,
            interpreter,
            shim_ref,
            store,
            exec_class,
        ) {
            Ok(()) => return Ok((candidate, read_roots)),
            Err(reason) => {
                tracing::info!(
                    interpreter = interpreter.as_str(),
                    candidate = %candidate.display(),
                    %reason,
                    "interpreter candidate did not run under the sandbox; trying the next"
                );
                rejected.push(format!("{}: {reason}", candidate.display()));
            }
        }
    }
    Err(rejected.join("; "))
}

/// Run `probe_source` through the real sandbox and require `ok` back.
fn run_probe(
    interpreter_path: &Path,
    read_roots: &[PathBuf],
    interpreter: Interpreter,
    shim_ref: ContentRef,
    store: &LocalFsContentStore,
    exec_class: ExecutorClass,
) -> Result<(), String> {
    let src_dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out_dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let script_path = src_dir.path().join("probe");
    std::fs::write(&script_path, interpreter.probe_source()).map_err(|e| e.to_string())?;
    let out_path = out_dir.path().join("result");

    let descriptor = ScriptDescriptor {
        interpreter_path: interpreter_path.to_string_lossy().into_owned(),
        script_path: canonical_or(&script_path).to_string_lossy().into_owned(),
        out_path: canonical_or(out_dir.path())
            .join("result")
            .to_string_lossy()
            .into_owned(),
        argv: Vec::new(),
        stdin_bytes: Vec::new(),
        env: Vec::new(),
        // A probe that hangs must not hang the registration.
        wall_clock_ms: PROBE_BUDGET_MS,
        mem_bytes: 0,
        max_output_bytes: 64,
    };
    let plumbing =
        interpreter_plumbing(interpreter_path, read_roots, src_dir.path(), out_dir.path());
    let warrant = probe_warrant(exec_class);
    let mote = probe_mote(shim_ref);
    run_script_body(
        store,
        exec_class,
        shim_ref,
        &mote,
        &warrant,
        &descriptor.encode(),
        &plumbing,
    )
    .map_err(|e| e.to_string())?;
    let produced = std::fs::read(&out_path).map_err(|e| e.to_string())?;
    if produced == b"ok" {
        Ok(())
    } else {
        Err(format!(
            "the probe returned {:?} rather than \"ok\"",
            String::from_utf8_lossy(&produced)
        ))
    }
}

/// The probe's warrant grants nothing: it needs no caller data, so anything it
/// could reach would be a bug in the plumbing rather than a requirement.
fn probe_warrant(exec_class: ExecutorClass) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
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
            wall_clock_ms: PROBE_BUDGET_MS,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: exec_class,
        secret_scope: SecretScope::None,
        ..Default::default()
    }
}

/// The synthetic body Mote the probe runs under.
fn probe_mote(shim_ref: ContentRef) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes(*shim_ref.as_bytes()),
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
        InputDataId::from_bytes([0; 32]),
        GraphPosition(b"probe".to_vec()),
        smallvec::SmallVec::new(),
    )
}

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
    // Refuse a ceiling this host cannot keep, BEFORE anything is stored. Checked here
    // rather than at the spawn because the spawn is too late to be honest: by then the
    // script is registered, listed with its declared scope, and every surface above it
    // has been told the constraint exists.
    if let Some(why) = crate::sandbox_probe::unenforceable_wish(
        exec_class,
        decl.wish.net_hosts.iter().map(|h| h.0.as_str()),
        decl.wish.mem_bytes > 0,
    ) {
        return Err(ScriptAdmissionError::UnenforceableCeiling(why));
    }
    // PROBE, do not assume. A candidate that exists is not a candidate that works:
    // `/usr/bin/python3` on macOS is a developer-tools trampoline that answers
    // `--version` happily from a normal shell and then fails inside the sandbox,
    // where it cannot reach what it wants to delegate to. Checking `is_file()`
    // would register a script whose first fire is its first failure — so each
    // candidate is run, sandboxed, exactly as a real script will be, and the first
    // that comes back is the one used.
    let (interpreter_path, interpreter_read_roots) =
        probe_interpreter(decl.interpreter, shim_ref, store, exec_class).map_err(|detail| {
            ScriptAdmissionError::InterpreterUnavailable {
                interpreter: decl.interpreter.as_str(),
                detail,
            }
        })?;

    let source_ref = store
        .put(&decl.source)
        .map_err(|e| ScriptAdmissionError::Storage(e.to_string()))?;
    // The registry row points at the RECORD, which pins the source by ref along
    // with the interpreter and fixed arguments. See `ScriptRecord`.
    let record = ScriptRecord {
        interpreter: decl.interpreter.as_str().to_string(),
        source_ref: *source_ref.as_bytes(),
        argv: decl.argv.clone(),
        env: decl.env.clone(),
        max_output_bytes: decl.wish.output_cap(),
    };
    let record_ref = store
        .put(&record.encode()?)
        .map_err(|e| ScriptAdmissionError::Storage(e.to_string()))?;

    let def = script_tool_def(decl, record_ref);
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
        script_ref: source_ref,
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
    Ok(source_ref)
}

/// The accepted interpreter tokens, for an admission error's `allowed` list.
pub fn allowed_interpreters() -> String {
    Interpreter::ALL
        .iter()
        .map(|i| i.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// the durable record
// ---------------------------------------------------------------------------

/// What the registry row points at: everything needed to reconstruct a script's
/// capability, with the source itself pinned by reference.
///
/// The registry has columns for a tool, not for a script — no interpreter, no
/// argv, no output cap. Rather than smuggle those into a text field, the row's
/// content ref names THIS record, and the record names the source. So the
/// registry still pins the exact thing that will run, and now pins the
/// interpreter and fixed arguments too: changing any of them is a different
/// record, a different ref, and therefore a different registration.
///
/// It also makes a script survive a restart. The broker is in-memory; the
/// registry is durable. Without a record the runtime could read back a row it
/// had no way to make fireable again, and the tool would resolve and then fail
/// at dispatch with nothing to explain it. [`rehydrate`] walks these records at
/// startup and re-registers each capability.
#[derive(Serialize, Deserialize)]
struct ScriptRecord {
    interpreter: String,
    source_ref: [u8; 32],
    argv: Vec<String>,
    env: Vec<(String, String)>,
    max_output_bytes: u64,
}

impl ScriptRecord {
    /// Canonical JSON — a pure function of the fields, so the same declaration
    /// always lands on the same ref.
    fn encode(&self) -> Result<Vec<u8>, ScriptAdmissionError> {
        serde_json::to_vec(self).map_err(|e| ScriptAdmissionError::Storage(e.to_string()))
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

// ---------------------------------------------------------------------------
// the host admin seam
// ---------------------------------------------------------------------------

/// The [`ScriptAdmin`] host impl.
///
/// Holds the same durable registry, content store and broker the serve path
/// uses, so a script registered over the RPC is immediately fireable by the
/// running loop — there is no second inventory that could disagree with the one
/// authority is decided against.
pub struct HostScriptRegistry<S: ContentStore + Send + Sync + 'static> {
    registry: Arc<SqliteToolRegistry>,
    store: LocalFsContentStore,
    broker: Arc<LocalCapabilityBroker<S>>,
    /// `None` when no sandbox shim shipped with this serve. Every registration
    /// then refuses, rather than admitting a script the runtime could only run on
    /// the host.
    shim_ref: Option<ContentRef>,
    exec_class: ExecutorClass,
}

impl<S: ContentStore + Send + Sync + 'static> HostScriptRegistry<S> {
    /// Compose the admin over the live serve objects.
    pub fn new(
        registry: Arc<SqliteToolRegistry>,
        store: LocalFsContentStore,
        broker: Arc<LocalCapabilityBroker<S>>,
        shim_ref: Option<ContentRef>,
        exec_class: ExecutorClass,
    ) -> Self {
        Self {
            registry,
            store,
            broker,
            shim_ref,
            exec_class,
        }
    }

    /// Read one script's row, its record and its source. `None` when the
    /// `(name, version)` is absent, or names a tool that is not a script —
    /// reporting one as the other would let a caller believe it can read a
    /// source that does not exist.
    fn row(&self, name: &str, version: &str) -> Option<(RegisteredScriptEntry, Vec<u8>)> {
        let def = self.registry.lookup(
            &ToolName(name.to_string()),
            &ToolVersion(version.to_string()),
        )?;
        let ToolKind::LocalScript { script_ref } = def.kind else {
            return None;
        };
        let record = ScriptRecord::decode(self.store.get(&script_ref).ok()?.as_ref())?;
        let source = self
            .store
            .get(&ContentRef::from_bytes(record.source_ref))
            .ok()?
            .as_ref()
            .to_vec();
        Some((entry_from(&def, &record), source))
    }

    /// Re-register every durably recorded script's capability on the broker.
    ///
    /// The registry survives a restart; the broker does not. Without this a
    /// restarted serve would resolve a script's tool and then fail at dispatch
    /// with an unknown capability — a row that looks live and is not. Each
    /// failure is logged and skipped rather than aborting startup: one script
    /// whose interpreter has since been uninstalled must not stop a serve.
    pub fn rehydrate(&self) -> usize {
        let Ok(rows) = self.registry.discover(usize::MAX, None) else {
            return 0;
        };
        let mut live = 0;
        for row in rows {
            let ToolKind::LocalScript { script_ref } = row.def.kind else {
                continue;
            };
            match self.reinstate(&row.def.tool_id, &row.def.tool_version, script_ref) {
                Ok(()) => live += 1,
                Err(error) => tracing::warn!(
                    script = %row.def.tool_id.0,
                    %error,
                    "a recorded script could not be made fireable and was skipped"
                ),
            }
        }
        if live > 0 {
            tracing::info!(count = live, "recorded scripts restored");
        }
        live
    }

    /// Rebuild one capability from its durable record.
    fn reinstate(
        &self,
        name: &ToolName,
        version: &ToolVersion,
        script_ref: ContentRef,
    ) -> Result<(), ScriptAdmissionError> {
        let shim_ref = self.shim_ref.ok_or(ScriptAdmissionError::ShimUnavailable)?;
        let bytes = self
            .store
            .get(&script_ref)
            .map_err(|e| ScriptAdmissionError::Storage(e.to_string()))?;
        let record = ScriptRecord::decode(bytes.as_ref())
            .ok_or_else(|| ScriptAdmissionError::Storage("unreadable script record".into()))?;
        let interpreter = Interpreter::parse(&record.interpreter).ok_or_else(|| {
            ScriptAdmissionError::UnknownInterpreter {
                got: record.interpreter.clone(),
                allowed: allowed_interpreters(),
            }
        })?;
        let interpreter_path =
            interpreter
                .resolve()
                .ok_or(ScriptAdmissionError::InterpreterUnavailable {
                    interpreter: interpreter.as_str(),
                    detail: "no candidate resolved during restore".into(),
                })?;
        let def = self
            .registry
            .lookup(name, version)
            .ok_or_else(|| ScriptAdmissionError::Storage("row vanished".into()))?;
        self.broker.register_capability(Box::new(ScriptCapability {
            name: name.clone(),
            version: version.clone(),
            script_ref: ContentRef::from_bytes(record.source_ref),
            shim_ref,
            interpreter_read_roots: Interpreter::read_roots(&interpreter_path),
            interpreter_path,
            argv: record.argv,
            env: record.env,
            ceiling: def.required_capability.min_resource_ceiling,
            max_output_bytes: record.max_output_bytes,
            exec_class: self.exec_class,
            store: self.store.clone(),
        }));
        Ok(())
    }
}

/// Project a registry row + its record into the admin seam's wire row.
/// The wire row shows the SOURCE's ref — what an operator wants to see and
/// diff. The record's own ref is an implementation detail of how the runtime
/// finds it again, so it is deliberately not surfaced.
fn entry_from(def: &ToolDef, record: &ScriptRecord) -> RegisteredScriptEntry {
    let req = &def.required_capability;
    RegisteredScriptEntry {
        script_id: script_id_of(&def.tool_id, &def.tool_version),
        script_name: def.tool_id.0.clone(),
        script_version: def.tool_version.0.clone(),
        interpreter: record.interpreter.clone(),
        description: def.description.clone(),
        source_ref_hex: hex32(&record.source_ref),
        fs_scope_summary: summarize_fs(&req.fs_scope_required),
        net_scope_summary: summarize_net(&req.net_scope_required),
        wall_clock_ms: req.min_resource_ceiling.wall_clock_ms,
        max_output_bytes: record.max_output_bytes,
    }
}

/// Display summary of a declared filesystem wish.
fn summarize_fs(scope: &FsScope) -> String {
    if scope.mounts.is_empty() {
        return "none".to_string();
    }
    scope
        .mounts
        .iter()
        .map(|(path, mode)| {
            let tag = match mode {
                FsMode::ReadOnly => "ro",
                FsMode::ReadWrite => "rw",
                FsMode::ExecOnly => "exec",
            };
            format!("{tag}:{}", path.display())
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Display summary of a declared egress wish.
fn summarize_net(scope: &NetScope) -> String {
    match scope {
        NetScope::None => "none".to_string(),
        NetScope::EgressAllowlist(hosts) if hosts.is_empty() => "none".to_string(),
        NetScope::EgressAllowlist(hosts) => format!(
            "egress:{}",
            hosts
                .iter()
                .map(|h| h.0.clone())
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// The 16-byte server-derived id, from the identity halves the grant key uses.
/// Deterministic, so the same script reports the same id across restarts.
fn script_id_of(name: &ToolName, version: &ToolVersion) -> [u8; 16] {
    // Length-delimited so ("ab","c") and ("a","bc") cannot collide.
    let mut seed = Vec::with_capacity(name.0.len() + version.0.len() + 16);
    seed.extend_from_slice(b"kx-script-id");
    seed.extend_from_slice(&(name.0.len() as u64).to_le_bytes());
    seed.extend_from_slice(name.0.as_bytes());
    seed.extend_from_slice(version.0.as_bytes());
    let full = ContentRef::of(&seed);
    let mut id = [0u8; 16];
    id.copy_from_slice(&full.as_bytes()[..16]);
    id
}

impl<S: ContentStore + Send + Sync + 'static> ScriptAdmin for HostScriptRegistry<S> {
    fn register(&self, reg: ScriptRegistration) -> Result<[u8; 16], ScriptAdminError> {
        let interpreter = Interpreter::parse(&reg.interpreter).ok_or_else(|| {
            ScriptAdminError::InvalidArgument(format!(
                "unknown interpreter {:?}; this build accepts {}",
                reg.interpreter,
                allowed_interpreters()
            ))
        })?;
        let mut fs_mounts = BTreeMap::new();
        for mount in &reg.fs_mounts {
            let mode = match mount.mode.as_str() {
                "ro" => FsMode::ReadOnly,
                "rw" => FsMode::ReadWrite,
                "exec" => FsMode::ExecOnly,
                other => {
                    return Err(ScriptAdminError::InvalidArgument(format!(
                        "unknown mount mode {other:?}; expected ro, rw or exec"
                    )))
                }
            };
            let path = PathBuf::from(&mount.path);
            if !path.is_absolute() {
                return Err(ScriptAdminError::InvalidArgument(format!(
                    "mount {:?} must be an absolute path",
                    mount.path
                )));
            }
            fs_mounts.insert(path, mode);
        }
        let net_hosts = reg.net_hosts.iter().map(|h| Host(h.clone())).collect();

        let decl = ScriptDecl {
            name: ToolName(reg.script_name),
            version: ToolVersion(reg.script_version),
            interpreter,
            source: reg.source,
            description: reg.description,
            // The RPC has no author field; the registration is operator-driven by
            // construction (a client that reached this seam is an authenticated
            // party), and nothing downstream reads it for enforcement.
            author: "operator".to_string(),
            argv: reg.argv,
            env: reg
                .env
                .into_iter()
                .map(|pair| (pair.key, pair.value))
                .collect(),
            wish: ScriptWish {
                fs_mounts,
                net_hosts,
                wall_clock_ms: reg.wall_clock_ms,
                mem_bytes: reg.mem_bytes,
                max_output_bytes: reg.max_output_bytes,
            },
        };
        let id = script_id_of(&decl.name, &decl.version);
        register_script(
            &decl,
            self.shim_ref,
            &self.store,
            &self.registry,
            &self.broker,
            self.exec_class,
        )
        .map_err(|e| admission_status(&e))?;
        Ok(id)
    }

    fn deregister(
        &self,
        script_name: &str,
        script_version: &str,
    ) -> Result<bool, kx_gateway_core::GatewayError> {
        let name = ToolName(script_name.to_string());
        let version = ToolVersion(script_version.to_string());
        // Only a script may be deregistered here: routing a tool through the
        // script surface would let a caller remove one by naming it as the other.
        if !matches!(
            self.registry.lookup(&name, &version).map(|d| d.kind),
            Some(ToolKind::LocalScript { .. })
        ) {
            return Ok(false);
        }
        // Removing the registry row is what withdraws authority: a dispatch needs
        // the tool in BOTH the caller's warrant grants and the Mote's contract,
        // and neither can be minted for a tool that no longer resolves. The
        // in-memory capability outliving the row is therefore inert.
        self.registry
            .deregister(&name, &version)
            .map_err(|e| kx_gateway_core::GatewayError::Internal(e.to_string()))
    }

    fn list(
        &self,
        limit: usize,
        after: Option<(String, String)>,
    ) -> Result<(Vec<RegisteredScriptEntry>, bool), kx_gateway_core::GatewayError> {
        let cursor = after
            .as_ref()
            .map(|(name, version)| (name.as_str(), version.as_str()));
        // Ask for one more than the page so `has_more` is observed rather than
        // guessed. Scripts are a subset of the registry, so the over-read is
        // filtered afterwards and the page is refilled until it is full or the
        // registry is exhausted.
        let rows = self
            .registry
            .discover(usize::MAX, cursor)
            .map_err(|e| kx_gateway_core::GatewayError::Internal(e.to_string()))?;
        let mut out = Vec::new();
        let mut has_more = false;
        for row in rows {
            let ToolKind::LocalScript { script_ref } = row.def.kind else {
                continue;
            };
            if out.len() == limit {
                has_more = true;
                break;
            }
            let Ok(bytes) = self.store.get(&script_ref) else {
                continue;
            };
            let Some(record) = ScriptRecord::decode(bytes.as_ref()) else {
                continue;
            };
            out.push(entry_from(&row.def, &record));
        }
        Ok((out, has_more))
    }

    fn get(
        &self,
        script_name: &str,
        script_version: &str,
    ) -> Result<Option<(RegisteredScriptEntry, Vec<u8>)>, kx_gateway_core::GatewayError> {
        Ok(self.row(script_name, script_version))
    }
}

/// Map an admission refusal onto the seam's error vocabulary.
///
/// The split matters at the RPC edge: a bad field is the caller's to fix, while a
/// missing shim or interpreter is the SERVE's limitation — the request was
/// well-formed and this host simply cannot honour it, which is not something a
/// client can correct by retrying with different arguments.
fn admission_status(err: &ScriptAdmissionError) -> ScriptAdminError {
    match err {
        ScriptAdmissionError::UnknownInterpreter { .. }
        | ScriptAdmissionError::BadSource { .. }
        | ScriptAdmissionError::BadIdentity => ScriptAdminError::InvalidArgument(err.to_string()),
        // An unenforceable ceiling belongs with the other HOST limitations, not with the
        // bad fields: the declaration is well-formed and would be honoured elsewhere.
        // What is missing is a capability of this machine, which no amount of retrying
        // with different arguments will supply — though narrowing the wish will.
        ScriptAdmissionError::ShimUnavailable
        | ScriptAdmissionError::InterpreterUnavailable { .. }
        | ScriptAdmissionError::UnenforceableCeiling(_) => {
            ScriptAdminError::Unavailable(err.to_string())
        }
        ScriptAdmissionError::Storage(_) | ScriptAdmissionError::Registration(_) => {
            ScriptAdminError::Storage(err.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// the bundled benchmark script
// ---------------------------------------------------------------------------

/// The bundled ORACLE script the live benchmark's `script` family drives.
///
/// Aggregates a CSV on stdin: one `name,amount` row per line, out comes the row
/// count and the total. Real data processing rather than a toy — a word count
/// over a five-word phrase is something a model answers from its own head, so it
/// measures the model and not the capability.
///
/// It is NOT fully underivable, and pretending otherwise would be worse than
/// saying so: the input reaches the model in the instruction, so a careful model
/// can sum the column itself. Making the answer truly unknowable would mean
/// giving the script a data file the model never sees, and that needs the
/// autonomous loop's warrant to carry filesystem scope — a far larger grant than
/// a benchmark should be the reason for. What this DOES buy is a multi-row
/// arithmetic the model gets wrong when it guesses, while `expected_tools` and
/// `tool_call_f1` measure firing directly, which is the metric for firing.
///
/// Pure shell builtins: the only executables a script gets are those in the
/// interpreter's own directory, so anything reaching into `/usr/bin` would not
/// run.
/// The `|| [ -n "$name" ]` is load-bearing, not defensive noise: `read` returns
/// non-zero when the final line has no trailing newline, so the plain loop drops
/// the last row and returns a total that is quietly, plausibly wrong. Real CSVs
/// arrive without a trailing newline all the time, and a single-line test never
/// shows it.
pub const BENCH_SCRIPT_SOURCE: &str = "rows=0\ntotal=0\n\
     while IFS=, read -r name amount || [ -n \"$name\" ]; do\n\
     case \"$amount\" in ''|*[!0-9]*) name=''; continue;; esac\n\
     rows=$((rows + 1))\n\
     total=$((total + amount))\n\
     name=''\n\
     done\nprintf 'ROWS=%s TOTAL=%s' \"$rows\" \"$total\"\n";

/// The bundled benchmark script's identity. The `<group>/<leaf>` shape mirrors
/// the bundled MCP tools, so a model emitting the bare leaf still resolves.
#[must_use]
pub fn bench_script_tool() -> (ToolName, ToolVersion) {
    (ToolName("script/csv-total".into()), ToolVersion("1".into()))
}

/// Register the bundled benchmark script. Fail-soft: a serve that cannot sandbox
/// simply does not get it, exactly like a missing bundled binary.
pub fn register_bench_script<S: ContentStore + Send + Sync>(
    shim_ref: Option<ContentRef>,
    store: &LocalFsContentStore,
    registry: &SqliteToolRegistry,
    broker: &LocalCapabilityBroker<S>,
    exec_class: ExecutorClass,
) -> Option<(ToolName, ToolVersion)> {
    let (name, version) = bench_script_tool();
    let decl = ScriptDecl {
        name: name.clone(),
        version: version.clone(),
        interpreter: Interpreter::Sh,
        source: BENCH_SCRIPT_SOURCE.as_bytes().to_vec(),
        description: "Aggregate a CSV of name,amount rows. Args: {\"input\": <csv text, \
                      one name,amount per line>}. Returns 'ROWS=<n> TOTAL=<sum>'. Runs \
                      sandboxed; no files, no network."
            .into(),
        author: "bundled".into(),
        argv: Vec::new(),
        env: Vec::new(),
        wish: ScriptWish::default(),
    };
    match register_script(&decl, shim_ref, store, registry, broker, exec_class) {
        Ok(_) => {
            tracing::info!(tool = %name.0, "bundled benchmark script registered");
            Some((name, version))
        }
        Err(error) => {
            tracing::info!(%error, "bundled benchmark script not registered");
            None
        }
    }
}
