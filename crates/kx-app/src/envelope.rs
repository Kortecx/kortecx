//! The [`AppEnvelope`] type + its canonical (de)serialization and validation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The envelope schema/version tag for a **Functional** App. Readers fail closed on a mismatch.
pub const APP_SCHEMA: &str = "kortecx.app/v1";

/// The envelope schema/version tag for an **Experience** (hosted) App.
///
/// D213: an Experience App is a distinct TYPE — it carries no `blueprint`, so it can never be
/// scheduled/triggered/compiled the way a Functional App is. The schema tag itself is the honest
/// discriminator (no separate `kind` field is added to the wire), which keeps every existing
/// Functional envelope's canonical bytes — and thus `app_ref`/`app_digest` — byte-identical.
pub const EXPERIENCE_SCHEMA: &str = "kortecx.experience/v1";

/// Errors from envelope (de)serialization and validation.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The bytes were not valid JSON / did not match the envelope shape.
    #[error("invalid app envelope JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// `schema` was absent or not [`APP_SCHEMA`].
    #[error("unsupported app schema {got:?} (expected {expected:?})")]
    Schema {
        /// The schema tag found in the envelope.
        got: String,
        /// The schema tag this binary supports.
        expected: &'static str,
    },
    /// A structural / value-level validation failure (bad ref, URL userinfo, a float, …).
    #[error("invalid app envelope: {0}")]
    Invalid(String),
}

fn default_version() -> String {
    "1".to_string()
}

/// A by-reference pointer to a context item (carries `media_type` — the bind-time
/// codec drops it, so it MUST live at the envelope layer).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRef {
    /// Display label.
    pub name: String,
    /// 64-char lowercase-hex content-store ref.
    pub content_ref: String,
    /// Advisory MIME type; never identity-bearing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media_type: String,
}

/// A registry reference to a tool (id + version only — never a grant).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRef {
    /// Registered tool id, e.g. `mcp-echo/echo`.
    pub tool_id: String,
    /// Tool version, e.g. `1`.
    pub tool_version: String,
}

/// A reference to an external connection — a descriptor (no URL userinfo) and a
/// credential NAME (never the secret bytes; the server resolves it by name at dial).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRef {
    /// The connection descriptor (e.g. an MCP endpoint). MUST carry no URL userinfo.
    pub descriptor: String,
    /// The credential NAME the server resolves at dial (never the value).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub credential_ref: String,
}

/// A reference to a dataset and the content-store blobs it spans.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRef {
    /// The dataset handle/ref.
    pub dataset_ref: String,
    /// 64-hex content-store refs the dataset spans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cas_refs: Vec<String>,
}

/// Ceiling on the `cas_refs` one declared [`DatasetRef`] may carry for SELF-INGEST.
///
/// Lives here, beside [`DatasetRef`], because it is a property of the ENVELOPE CONTRACT —
/// not of whoever happens to enforce it. Two crates need the same answer: the gateway
/// enforces it on first run, and the CLI must warn at EXPORT that a corpus will not
/// self-ingest. A constant duplicated in both is a constant that drifts, and the two would
/// then disagree about the same question while both looking right.
pub const MAX_APP_CORPUS_REFS: usize = 4096;

/// Ceiling on the total corpus BYTES one declared [`DatasetRef`] may SELF-INGEST.
///
/// **The bound that matters:** every byte is chunked and synchronously EMBEDDED on first run,
/// so the cost is model-TIME, not disk. 64 MiB of prose is millions of tokens — well clear of
/// any realistic text corpus, while keeping a hand-rolled envelope from turning one run into
/// hours of embedding.
///
/// **This is NOT the bundle ceiling.** `kx-cli`'s `MAX_BUNDLE_CLOSURE_BYTES` (512 MiB) bounds
/// the WHOLE closure an export may carry — prompts, artifacts, attachments, every dataset. The
/// two are different scopes and 512 > 64 is not a contradiction: a bundle legitimately holds
/// far more than any single dataset's embeddable corpus. What they DO imply together is a real
/// hazard, which is why this constant is shared rather than duplicated: a corpus between 64 MiB
/// and 512 MiB **exports cleanly and then silently does not self-ingest**, leaving the App to
/// ground on nothing. The export path uses this constant to say so at authoring time, when the
/// user can still act — instead of leaving a `tracing::warn!` in a server log nobody reads.
pub const MAX_APP_CORPUS_BYTES: u64 = 64 * 1024 * 1024;

/// A named text artifact (prompt / rule / memory) stored in the content store.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Display label / role.
    pub name: String,
    /// 64-char lowercase-hex content-store ref to the artifact body.
    pub content_ref: String,
}

/// A skill: a named (instructions + tool SET) bundle ≈ a reusable Agent. The
/// `tools` map is a grant WISH (id → version), re-resolved by the server at bind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRef {
    /// Display label.
    pub name: String,
    /// 64-char lowercase-hex content-store ref to the instructions body.
    pub instructions_ref: String,
    /// The skill's tool wish set (id → version).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, String>,
}

/// A reference to another App in the SAME caller's catalog, by handle.
///
/// The declaration half of App composition: a step names this handle in its
/// `apps` list, and the runtime lowers that App's own blueprint into the run.
///
/// **Handle only, deliberately.** It carries no snapshot of the callee's name or
/// `delivers` (a copy would go stale the moment the callee is edited, and the console
/// already lists Apps), and it pins no `app_digest`. Composition resolves the callee as it
/// is AT AUTHOR TIME, so fixing a callee improves every App that calls it — which is the
/// point of making an App a capability rather than a copy. Determinism is not lost by
/// this: the composed DAG is what the run's `MoteId`s and recipe fingerprint are derived
/// from, so a changed callee is visibly a different run, not a silently different one.
///
/// Carries NO authority, like every other entry on this rail: the callee re-resolves its
/// own warrants from its own envelope, and the caller can neither widen nor narrow them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRef {
    /// The callee's `namespace/collection/name` catalog handle.
    pub handle: String,
}

/// The by-reference rail. Every field is by-ref or by-name; no inline bytes,
/// no authority.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct References {
    /// Context items (carry `media_type`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ContextRef>,
    /// Tool registry references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolRef>,
    /// External connection references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionRef>,
    /// Dataset references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datasets: Vec<DatasetRef>,
    /// Prompt artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<ArtifactRef>,
    /// Rule artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ArtifactRef>,
    /// Skill bundles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillRef>,
    /// Memory artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory: Vec<ArtifactRef>,
    /// Other Apps this App may run.
    ///
    /// The field means one thing per lane, and both are the same idea — "the Apps this one is
    /// allowed to invoke":
    /// - **Functional** — the composition rail. A step names a handle here and the runtime
    ///   lowers that App into the run (`app_run`); an undeclared name is refused.
    /// - **Experience (hosted)** — the page's runnable set. The supervisor mints the served
    ///   page a scoped token from exactly these handles, and `RunApp` refuses the page anything
    ///   outside them. A hosted app has no blueprint, so nothing is *composed*; the declaration
    ///   is instead what the browser-side SDK is permitted to call back and run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<AppRef>,
}

impl References {
    /// True when no reference is set (so the field is omitted from canonical bytes).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.context.is_empty()
            && self.tools.is_empty()
            && self.connections.is_empty()
            && self.datasets.is_empty()
            && self.prompts.is_empty()
            && self.rules.is_empty()
            && self.skills.is_empty()
            && self.memory.is_empty()
            && self.apps.is_empty()
    }
}

/// Axis 1 — model + routing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSteering {
    /// The requested model route (server may rebind; `""` ⇒ served model).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_route: String,
    /// Recipe free-params (string-valued, canonical-JSON safe).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub free_params: BTreeMap<String, String>,
}

impl ModelSteering {
    /// True when default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model_route.is_empty() && self.free_params.is_empty()
    }
}

/// How the tool wish is resolved against the caller's own tool authority.
///
/// `Explicit` (default) materialises only the declared wish (`requested_grants`
/// ∪ any skill wishes), intersected with the caller's resolvable tool ceiling —
/// the pre-existing behaviour, so a default-reach envelope is byte-identical.
/// `InheritPrincipal` sets the wish to the caller's WHOLE resolvable tool ceiling
/// (the tools the caller is registered + allowed to fire), so the materialised set
/// equals that ceiling — bounded, never unbounded. Either way the field only
/// STEERS resolution; it grants nothing (the server always intersects, never
/// unions — a union would widen past the ceiling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// Materialise only the declared wish (byte-identical to the pre-reach form).
    #[default]
    Explicit,
    /// Expand the wish to the caller's whole resolvable tool ceiling.
    InheritPrincipal,
}

impl Reach {
    /// True for the default (`Explicit`). The `skip_serializing_if` predicate that
    /// keeps a default-reach envelope byte-identical — the `reach` key is omitted.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        matches!(self, Reach::Explicit)
    }
}

/// Axis 2 — tools + scopes. `requested_grants` is a WISH the server intersects
/// with the importer's own grants ∩ the step warrant; it grants nothing. `reach`
/// selects the wish set (declared vs. the caller's whole ceiling).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsSteering {
    /// The tool wish set (id → version).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requested_grants: BTreeMap<String, String>,
    /// Requested egress scope (host patterns); re-vetted at bind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net_scope: Vec<String>,
    /// Requested filesystem scope (confined roots); re-vetted at bind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fs_scope: Vec<String>,
    /// How the wish is resolved (default `Explicit` ⇒ omitted from canonical bytes).
    #[serde(default, skip_serializing_if = "Reach::is_explicit")]
    pub reach: Reach,
}

impl ToolsSteering {
    /// True when default. `reach` participates: a lone `InheritPrincipal` must
    /// keep the whole `tools` axis emitted (else the axis silently drops).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requested_grants.is_empty()
            && self.net_scope.is_empty()
            && self.fs_scope.is_empty()
            && self.reach.is_explicit()
    }
}

/// Axis 3 — context + data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSteering {
    /// 64-hex content refs to fold at bind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_refs: Vec<String>,
    /// Context-bundle handles to attach at bind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundle_handles: Vec<String>,
    /// Dataset refs to ground over.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dataset_refs: Vec<String>,
}

impl ContextSteering {
    /// True when default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.context_refs.is_empty()
            && self.bundle_handles.is_empty()
            && self.dataset_refs.is_empty()
    }
}

/// Axis 4 — guards + budgets. `secret_scope` lists secret NAMES to expose by
/// name at bind (never values). `cost_ceiling` is reserved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guards {
    /// React loop turn budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// React loop tool-call budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    /// Require TLS for egress.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tls_required: bool,
    /// Secret NAMES to expose by name (never values).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_scope: Vec<String>,
    /// Reserved cost ceiling (integer units).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_ceiling: Option<u64>,
}

impl Guards {
    /// True when default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max_turns.is_none()
            && self.max_tool_calls.is_none()
            && !self.tls_required
            && self.secret_scope.is_empty()
            && self.cost_ceiling.is_none()
    }
}

// Signature fixed by serde's `skip_serializing_if = "is_false"` (must be `fn(&T) -> bool`).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// The four-axis steering config the server RE-RESOLVES at bind. Steers; never grants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteeringConfig {
    /// Axis 1.
    #[serde(default, skip_serializing_if = "ModelSteering::is_empty")]
    pub model: ModelSteering,
    /// Axis 2.
    #[serde(default, skip_serializing_if = "ToolsSteering::is_empty")]
    pub tools: ToolsSteering,
    /// Axis 3.
    #[serde(default, skip_serializing_if = "ContextSteering::is_empty")]
    pub context: ContextSteering,
    /// Axis 4.
    #[serde(default, skip_serializing_if = "Guards::is_empty")]
    pub guards: Guards,
}

impl SteeringConfig {
    /// True when every axis is default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model.is_empty()
            && self.tools.is_empty()
            && self.context.is_empty()
            && self.guards.is_empty()
    }
}

/// Per-step replay disposition (metadata at this layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    /// Re-bind the committed bytes (no re-inference).
    Frozen,
    /// Run fresh (new `instance_id` / `step_salt`; side effects re-fire).
    ReRun,
}

/// Per-step replay intent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replay {
    /// `step_id` → mode.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_step: BTreeMap<String, ReplayMode>,
}

impl Replay {
    /// True when no per-step disposition is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.per_step.is_empty()
    }
}

/// Which lane an App belongs to. Derived from the envelope `schema` tag — NOT a
/// serialized field, so a Functional envelope stays byte-identical (D213).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    /// A Functional App (`kortecx.app/v1`) — a schedulable/triggerable capability DAG.
    #[default]
    Functional,
    /// An Experience App (`kortecx.experience/v1`) — a hosted, code-bearing web app.
    Experience,
}

impl AppKind {
    /// The stable lowercase wire label (`"functional"` / `"experience"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AppKind::Functional => "functional",
            AppKind::Experience => "experience",
        }
    }
}

/// How a Functional App is authored — the second axis, orthogonal to [`AppKind`].
///
/// [`AppKind`] says which LANE an App belongs to (schedulable capability vs hosted web app);
/// this says what the artifact of a scheduled App actually IS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    /// A TEXT app that works by model steer: it carries prompt / rules / reference markdown,
    /// and the runtime hands that context to the model, which acts through the tools, skills
    /// and integrations it was granted. The default, and what every App authored before this
    /// field existed keeps doing.
    #[default]
    Contextual,
    /// The model authors the code and configuration the runtime needs in order to manage,
    /// orchestrate and run the App. The artifact is a real project, not prose.
    Codified,
}

impl AppMode {
    /// The stable lowercase wire label (`"contextual"` / `"codified"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AppMode::Contextual => "contextual",
            AppMode::Codified => "codified",
        }
    }

    /// Parse the envelope label. Anything unrecognized — including the empty string every App
    /// authored before this field existed carries — is [`AppMode::Contextual`].
    ///
    /// Degrading rather than failing is the SAFE direction: contextual authors markdown only,
    /// so an unreadable label can never make a reader write files it does not understand.
    /// [`AppEnvelope::validate`] is what refuses an unknown label outright, so a typo is an
    /// honest authoring error rather than a silent downgrade — this is the defensive read for
    /// anything that got past it.
    #[must_use]
    pub fn from_label(s: &str) -> Self {
        match s {
            "codified" => Self::Codified,
            _ => Self::Contextual,
        }
    }
}

/// The framework an Experience App is scaffolded and served as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedFramework {
    /// The model picks the framework from the prompt (resolved before scaffolding).
    #[default]
    Auto,
    /// A Vite + React single-page app (the simplest dev server to supervise).
    ViteReact,
    /// A Next.js app (choose only when SSR / route handlers / file routing are needed).
    NextJs,
    /// A Vite + Svelte single-page app (a lightweight React alternative; same dev-server
    /// shape as Vite-React, so the supervisor treats it identically).
    Svelte,
}

impl HostedFramework {
    /// The stable lowercase wire label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            HostedFramework::Auto => "auto",
            HostedFramework::ViteReact => "vite_react",
            HostedFramework::NextJs => "next_js",
            HostedFramework::Svelte => "svelte",
        }
    }
}

/// The hosted-lane configuration carried by an Experience envelope. The real project
/// file tree lives in the App's `branch_handle` (a CoW-on-CAS branch manifest); this
/// struct carries only the framework + optional advisory install/dev command overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedConfig {
    /// The framework this app is scaffolded/served as.
    #[serde(default)]
    pub framework: HostedFramework,
    /// Advisory install command override (default: the framework's `npm install`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub install_cmd: String,
    /// Advisory dev-server command override (default: the framework's `npm run dev`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dev_cmd: String,
    /// Which lane the supervisor serves this app on: `""`/`"dev"` (materialize → install
    /// → `npm run dev`, hot reload — the default) or `"production"` (… → `npm run build`
    /// → the framework's preview/start server).
    ///
    /// A property of the APP, not of one press of Start: whether this is a live-editing
    /// workspace or a built artifact is authored, not toggled per request. Carrying it
    /// here rather than on `StartHostedAppRequest` also costs zero wire surface — the
    /// envelope is opaque bytes to the gateway.
    ///
    /// Empty on every app authored before this field existed, and empty parses as `dev`,
    /// so no existing app silently changes lane.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serve_mode: String,
    /// Advisory build command override (default: the framework's `npm run build`).
    /// Ignored in dev mode, which never builds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub build_cmd: String,
}

/// A `kortecx.app/v1` (Functional) or `kortecx.experience/v1` (Experience) envelope: a
/// portable blueprint OR a hosted-app config, wrapped with references, a steering
/// config, replay intent, and an optional project branch handle. Carries NO authority.
/// See the crate docs for the full contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppEnvelope {
    /// The schema/version tag — always [`APP_SCHEMA`].
    pub schema: String,
    /// The App name (the human handle within the catalog).
    pub name: String,
    /// The App version (default `"1"`).
    #[serde(default = "default_version")]
    pub version: String,
    /// Free-form description.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What one run of this App PRODUCES, in one line — the half of the catalog record that
    /// makes an App pickable by another App.
    ///
    /// `description` says what an App *is*; `input_schema` says what it *needs*. Neither says
    /// what comes BACK, so nothing composing Apps could choose one on purpose. This is that
    /// third fact, and it is the line the composition menu renders.
    ///
    /// Advisory prose — NEVER parsed for enforcement (the `description` posture). It steers
    /// which App an author picks; it constrains nothing about the run.
    ///
    /// Empty on every App authored before this field existed, and `skip_serializing_if` means
    /// empty adds ZERO bytes — so every existing envelope canonicalizes byte-identically and
    /// its `app_ref` / `app_digest` are unchanged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delivers: String,
    /// Catalog tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional input schema (opaque JSON), e.g. a JSON-schema for `run` args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// The portable blueprint (a `DagSpec`) carried VERBATIM as opaque JSON. Present for a
    /// Functional App; ALWAYS absent for an Experience App (which carries `hosted` instead).
    /// `skip_serializing_if` keeps a Functional envelope's canonical bytes unchanged — a
    /// `Some(object)` serializes under `"blueprint"` byte-identically to the old bare `Value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<Value>,
    /// The hosted-lane config. Present for an Experience App; ALWAYS absent (omitted) for a
    /// Functional App, so this field adds ZERO bytes to any existing Functional envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosted: Option<HostedConfig>,
    /// The by-reference rail.
    #[serde(default, skip_serializing_if = "References::is_empty")]
    pub references: References,
    /// The four-axis steering config.
    #[serde(default, skip_serializing_if = "SteeringConfig::is_empty")]
    pub steering_config: SteeringConfig,
    /// Per-step replay intent.
    #[serde(default, skip_serializing_if = "Replay::is_empty")]
    pub replay: Replay,
    /// Optional per-App project branch handle (reserved; the scaffold creates it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch_handle: String,
    /// How a Functional App is authored: `""`/`"contextual"` (a text app steered by its own
    /// markdown — the default) or `"codified"` (the model authors the code and configuration
    /// the runtime orchestrates from). See [`AppMode`].
    ///
    /// A property of the APP, not of one run: whether the artifact is prose or a project is
    /// authored, not toggled per invocation. ALWAYS empty for an Experience App, which has a
    /// project by construction — [`AppEnvelope::validate`] refuses one that carries it.
    ///
    /// Empty on every App authored before this field existed, and `skip_serializing_if` means
    /// empty adds ZERO bytes — so every existing envelope canonicalizes byte-identically and
    /// its `app_ref` / `app_digest` are unchanged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
}

/// The envelope-derived summary the catalog projects (the host adds handle + `app_ref`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppSummary {
    /// App name.
    pub name: String,
    /// App version.
    pub version: String,
    /// Description.
    pub description: String,
    /// What one run of this App produces (the [`AppEnvelope::delivers`] line).
    ///
    /// Carried on the SUMMARY, not left to a second call, so one `ListApps` yields the whole
    /// composition registry: the deriving model needs every candidate's output line at once,
    /// and an N+1 manifest read per App would put a live registry read inside the authoring
    /// path.
    pub delivers: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Number of blueprint steps (display only; 0 for an Experience App).
    pub step_count: u32,
    /// Which lane the App belongs to (derived from the schema tag).
    pub kind: AppKind,
    /// How a Functional App is authored. Always [`AppMode::Contextual`] for an Experience App
    /// (which has no such axis — see [`AppEnvelope::validate`]).
    pub mode: AppMode,
}

impl AppEnvelope {
    /// A minimal Functional envelope wrapping `blueprint` under `name`, schema + version preset.
    #[must_use]
    pub fn new(name: impl Into<String>, blueprint: Value) -> Self {
        Self {
            schema: APP_SCHEMA.to_string(),
            name: name.into(),
            version: default_version(),
            description: String::new(),
            delivers: String::new(),
            tags: Vec::new(),
            input_schema: None,
            blueprint: Some(blueprint),
            hosted: None,
            references: References::default(),
            steering_config: SteeringConfig::default(),
            replay: Replay::default(),
            branch_handle: String::new(),
            mode: String::new(),
        }
    }

    /// A minimal Experience (hosted) envelope: no blueprint, a `hosted` config, and the
    /// `branch_handle` that will hold the generated project file tree.
    #[must_use]
    pub fn new_experience(
        name: impl Into<String>,
        hosted: HostedConfig,
        branch_handle: impl Into<String>,
    ) -> Self {
        Self {
            schema: EXPERIENCE_SCHEMA.to_string(),
            name: name.into(),
            version: default_version(),
            description: String::new(),
            delivers: String::new(),
            tags: Vec::new(),
            input_schema: None,
            blueprint: None,
            hosted: Some(hosted),
            references: References::default(),
            steering_config: SteeringConfig::default(),
            replay: Replay::default(),
            branch_handle: branch_handle.into(),
            mode: String::new(),
        }
    }

    /// Which lane this App belongs to, derived from the schema tag. An unknown schema
    /// resolves to [`AppKind::Functional`]; [`AppEnvelope::validate`] is what rejects it.
    #[must_use]
    pub fn kind(&self) -> AppKind {
        if self.schema == EXPERIENCE_SCHEMA {
            AppKind::Experience
        } else {
            AppKind::Functional
        }
    }

    /// How this App is authored — the parsed [`AppEnvelope::mode`] label.
    ///
    /// Always [`AppMode::Contextual`] for an Experience App: `validate` refuses one that
    /// carries a mode at all, so there is no second answer to derive.
    #[must_use]
    pub fn mode(&self) -> AppMode {
        AppMode::from_label(&self.mode)
    }

    /// Parse + validate an envelope from JSON bytes (any key order accepted).
    ///
    /// # Errors
    /// Returns [`AppError::Json`] if the bytes are not valid envelope JSON, or the
    /// [`AppError`] from [`AppEnvelope::validate`] if the parsed envelope is invalid.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, AppError> {
        let env: Self = serde_json::from_slice(bytes)?;
        env.validate()?;
        Ok(env)
    }

    /// Canonical bytes: keys sorted (via [`serde_json::Value`]), compact, no floats.
    /// This is the hashable + on-the-wire form; identical across Rust/Py/TS.
    ///
    /// # Errors
    /// Returns [`AppError::Json`] if the envelope cannot be serialized (it never
    /// can in practice — the type holds only JSON-safe fields).
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, AppError> {
        let value = serde_json::to_value(self)?;
        Ok(serde_json::to_vec(&value)?)
    }

    /// The human export form: pretty (2-space) + sorted keys + a trailing newline.
    ///
    /// # Errors
    /// Returns [`AppError::Json`] if the envelope cannot be serialized.
    pub fn to_pretty_json(&self) -> Result<String, AppError> {
        let value = serde_json::to_value(self)?;
        let mut s = serde_json::to_string_pretty(&value)?;
        s.push('\n');
        Ok(s)
    }

    /// Every content-store ref this App references — the transitive content closure
    /// for a portable export, and the seed for the future GC reachability walk.
    ///
    /// Returns 64-char lowercase-hex refs, **sorted and deduplicated** (empties
    /// skipped). Covers the always-travel artifact rail — `references.context`,
    /// `prompts`, `rules`, `memory` (`content_ref`), `skills` (`instructions_ref`),
    /// and `steering_config.context.context_refs`. `include_datasets` gates the
    /// (potentially large) RAG payload refs in `references.datasets[].cas_refs`
    /// (export's `--with-data`); the GC reachability set passes `true`.
    ///
    /// The opaque `blueprint` is intentionally NOT scanned — it carries inline text,
    /// never a content ref (validated by [`AppEnvelope::validate`]). Any future
    /// ref-bearing blueprint field MUST extend both this walk and `validate`.
    #[must_use]
    pub fn content_refs(&self, include_datasets: bool) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        for c in &self.references.context {
            set.insert(c.content_ref.clone());
        }
        for a in self
            .references
            .prompts
            .iter()
            .chain(&self.references.rules)
            .chain(&self.references.memory)
        {
            set.insert(a.content_ref.clone());
        }
        for s in &self.references.skills {
            set.insert(s.instructions_ref.clone());
        }
        for r in &self.steering_config.context.context_refs {
            set.insert(r.clone());
        }
        if include_datasets {
            for d in &self.references.datasets {
                for r in &d.cas_refs {
                    set.insert(r.clone());
                }
            }
        }
        set.into_iter().filter(|r| !r.is_empty()).collect()
    }

    /// The catalog summary derived from this envelope.
    #[must_use]
    pub fn summary(&self) -> AppSummary {
        let step_count = self
            .blueprint
            .as_ref()
            .and_then(|b| b.get("steps"))
            .and_then(Value::as_array)
            .map_or(0, |s| u32::try_from(s.len()).unwrap_or(u32::MAX));
        AppSummary {
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            delivers: self.delivers.clone(),
            tags: self.tags.clone(),
            step_count,
            kind: self.kind(),
            mode: self.mode(),
        }
    }

    /// Validate structure + the security boundary:
    /// - `schema` is [`APP_SCHEMA`] (Functional) or [`EXPERIENCE_SCHEMA`] (Experience);
    /// - a Functional App carries a JSON-object `blueprint` and NO `hosted` config;
    /// - an Experience App carries a `hosted` config, NO `blueprint`, and a non-empty
    ///   `branch_handle` (the project file tree) — so it can never be scheduled (D213);
    /// - every content/instructions/cas ref is 64-char lowercase hex;
    /// - connection descriptors carry NO URL userinfo and credential refs are bare names;
    /// - no floats anywhere (SN-8 — identity bytes are integer-only).
    ///
    /// # Errors
    /// Returns [`AppError::Schema`] on a schema-tag mismatch, or [`AppError::Invalid`]
    /// for a lane/field-shape mismatch (missing or non-object blueprint, missing hosted
    /// config, an Experience App with a blueprint, an empty hosted `branch_handle`), a
    /// malformed ref, URL userinfo in a connection descriptor, a non-bare credential
    /// name, or any float.
    //
    // One flat per-lane then per-rail sequence of independent shape checks. Splitting it
    // would scatter the contract across helpers that are each read once, and the value of
    // this function is that the whole envelope contract is visible in one place.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), AppError> {
        match self.kind() {
            AppKind::Functional => {
                if self.schema != APP_SCHEMA {
                    return Err(AppError::Schema {
                        got: self.schema.clone(),
                        expected: APP_SCHEMA,
                    });
                }
                if !self.blueprint.as_ref().is_some_and(Value::is_object) {
                    return Err(AppError::Invalid("blueprint must be a JSON object".into()));
                }
                if self.hosted.is_some() {
                    return Err(AppError::Invalid(
                        "a functional app must not carry a hosted config".into(),
                    ));
                }
                // Refuse an unknown label rather than let `AppMode::from_label` degrade it.
                // The degrade is the right READ posture (contextual authors markdown only, so
                // it can never write files a reader does not understand), but at AUTHORING a
                // typo silently becoming a text app is a surprise the user cannot see: they
                // asked for a project and got prose, with nothing to point at.
                if !self.mode.is_empty() && !matches!(self.mode.as_str(), "contextual" | "codified")
                {
                    return Err(AppError::Invalid(format!(
                        "unknown app mode {:?} (expected \"contextual\" or \"codified\")",
                        self.mode
                    )));
                }
            }
            AppKind::Experience => {
                if self.hosted.is_none() {
                    return Err(AppError::Invalid(
                        "an experience app must carry a hosted config".into(),
                    ));
                }
                if self.blueprint.is_some() {
                    return Err(AppError::Invalid(
                        "an experience app must not carry a blueprint (it is not schedulable)"
                            .into(),
                    ));
                }
                if self.branch_handle.is_empty() {
                    return Err(AppError::Invalid(
                        "an experience app must carry a branch_handle (the project file tree)"
                            .into(),
                    ));
                }
                // The contextual/codified axis is a property of the SCHEDULED lane. A hosted
                // app is a real project by construction, so the question does not apply — and
                // answering it anyway would give two surfaces disagreeing about what an
                // Experience app is.
                if !self.mode.is_empty() {
                    return Err(AppError::Invalid(
                        "an experience app must not carry a mode (it is a project by \
                         construction, not a contextual/codified choice)"
                            .into(),
                    ));
                }
            }
        }
        // References by-ref/by-name discipline.
        for c in &self.references.context {
            check_ref("context.content_ref", &c.content_ref)?;
        }
        for a in self
            .references
            .prompts
            .iter()
            .chain(&self.references.rules)
            .chain(&self.references.memory)
        {
            check_ref("artifact.content_ref", &a.content_ref)?;
        }
        for s in &self.references.skills {
            check_ref("skill.instructions_ref", &s.instructions_ref)?;
        }
        for d in &self.references.datasets {
            for r in &d.cas_refs {
                check_ref("dataset.cas_ref", r)?;
            }
        }
        for conn in &self.references.connections {
            check_descriptor_no_userinfo(&conn.descriptor)?;
            check_bare_name("credential_ref", &conn.credential_ref)?;
        }
        for a in &self.references.apps {
            check_app_handle(&a.handle)?;
        }
        for r in &self.steering_config.context.context_refs {
            check_ref("steering.context_ref", r)?;
        }
        // Tool rails: every wished/displayed tool id is well-formed and its version is
        // an integer — across `references.tools`, each skill's wish set, and the
        // steering-config grant wish. Left unvalidated, a hand-authored envelope could
        // carry a malformed id or DISPLAY a tool it never legitimately requests (SN-8).
        for t in &self.references.tools {
            check_tool_id("references.tools.tool_id", &t.tool_id)?;
            check_integer("references.tools.tool_version", &t.tool_version)?;
        }
        for s in &self.references.skills {
            for (id, version) in &s.tools {
                check_tool_id("skill.tools", id)?;
                check_integer(&format!("skill.tools[{id}]"), version)?;
            }
        }
        for (id, version) in &self.steering_config.tools.requested_grants {
            check_tool_id("steering.requested_grants", id)?;
            check_integer(&format!("steering.requested_grants[{id}]"), version)?;
        }
        // No floats anywhere (the whole serialized tree, incl. the opaque blueprint).
        let value = serde_json::to_value(self)?;
        reject_floats(&value)?;
        Ok(())
    }
}

/// Re-canonicalize received envelope bytes (the gateway host derives the `app_ref`
/// over this form, so client byte-ordering never affects identity). Validates first.
///
/// # Errors
/// Returns the [`AppError`] from [`AppEnvelope::from_json_slice`] if the bytes are
/// not a valid envelope.
pub fn canonical_json(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    AppEnvelope::from_json_slice(bytes)?.to_canonical_json()
}

/// Extract the catalog summary from received envelope bytes (validates first).
///
/// # Errors
/// Returns the [`AppError`] from [`AppEnvelope::from_json_slice`] if the bytes are
/// not a valid envelope.
pub fn summary_of(bytes: &[u8]) -> Result<AppSummary, AppError> {
    Ok(AppEnvelope::from_json_slice(bytes)?.summary())
}

fn check_ref(field: &str, r: &str) -> Result<(), AppError> {
    if r.len() != 64
        || !r
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(AppError::Invalid(format!(
            "{field} must be 64-char lowercase hex, got {r:?}"
        )));
    }
    Ok(())
}

fn check_bare_name(field: &str, name: &str) -> Result<(), AppError> {
    if name.contains('@') || name.contains(':') || name.contains(char::is_whitespace) {
        return Err(AppError::Invalid(format!(
            "{field} must be a bare credential name (no '@', ':', or whitespace), got {name:?}"
        )));
    }
    Ok(())
}

/// A catalog handle: exactly three non-empty `[a-z0-9._-]` segments, none starting or
/// ending with `.`/`-` — the `AssetPath` shape `SaveApp` already enforces on the wire.
///
/// Re-stated here rather than shared because `kx-app` sits below the gateway and takes no
/// dependency on it; the two must agree, and the same reasoning applies as for
/// [`check_tool_id`]. Checked at authoring so a handle that could NEVER resolve is refused
/// where the author can still fix it, instead of surfacing at run as a missing App.
fn check_app_handle(handle: &str) -> Result<(), AppError> {
    let segments: Vec<&str> = handle.split('/').collect();
    let valid = segments.len() == 3
        && segments.iter().all(|s| {
            let b = s.as_bytes();
            !s.is_empty()
                && s.len() <= 128
                && s.bytes().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
                })
                && !matches!(b[0], b'.' | b'-')
                && !matches!(b[b.len() - 1], b'.' | b'-')
        });
    if !valid {
        return Err(AppError::Invalid(format!(
            "references.apps[].handle must be a 'namespace/collection/name' AssetPath \
             ([a-z0-9._-] segments), got {handle:?}"
        )));
    }
    Ok(())
}

/// A wished/displayed tool id: one or two non-empty `[a-z0-9._-]` segments, e.g.
/// `retrieve` (a bundled capability) or `gmail/search` (a connector tool). Mirrors
/// the same check in `kx-skill`'s manifest so the two rails agree on what a tool id is.
fn check_tool_id(field: &str, id: &str) -> Result<(), AppError> {
    let segments: Vec<&str> = id.split('/').collect();
    let valid = id.len() <= 128
        && (1..=2).contains(&segments.len())
        && segments.iter().all(|s| {
            !s.is_empty()
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
                })
        });
    if !valid {
        return Err(AppError::Invalid(format!(
            "{field} must be a tool id ('name' or 'server/name' of [a-z0-9._-]), got {id:?}"
        )));
    }
    Ok(())
}

/// A tool version must be an integer string (e.g. `"1"`) — never a float or free text.
fn check_integer(field: &str, v: &str) -> Result<(), AppError> {
    if v.parse::<u64>().is_err() {
        return Err(AppError::Invalid(format!(
            "{field} must be an integer string, got {v:?}"
        )));
    }
    Ok(())
}

/// Reject a connection descriptor that smuggles URL userinfo (`user:pw@host`), with
/// or without a `scheme://` prefix. Keying on the presence of `://` missed
/// scheme-less/opaque authorities (`user:pw@host/mcp`), which carry userinfo in the
/// same position — so the authority is ALWAYS derived and scanned for '@'.
fn check_descriptor_no_userinfo(descriptor: &str) -> Result<(), AppError> {
    // Strip an OPTIONAL `scheme://`, falling through to the whole descriptor when it
    // is absent — a scheme-less authority carries userinfo just the same.
    let after_scheme = descriptor
        .split_once("://")
        .map_or(descriptor, |(_, rest)| rest);
    // The authority ends at the first '/', '?', or '#'.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    if authority.contains('@') {
        return Err(AppError::Invalid(format!(
            "connection descriptor must not carry URL userinfo, got {descriptor:?}"
        )));
    }
    Ok(())
}

/// Walk a JSON value and reject any non-integer number (SN-8 — no floats on identity).
fn reject_floats(v: &Value) -> Result<(), AppError> {
    match v {
        Value::Number(n) => {
            if !n.is_i64() && !n.is_u64() {
                return Err(AppError::Invalid(format!("floats are not allowed: {n}")));
            }
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(reject_floats),
        Value::Object(o) => o.values().try_for_each(reject_floats),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn sample_blueprint() -> Value {
        json!({
            "seed": 0,
            "steps": [
                { "kind": "model", "prompt": "Use the echo tool.", "tool_contract": { "mcp-echo/echo": "1" } }
            ]
        })
    }

    #[test]
    fn canonical_json_is_sorted_and_round_trips() {
        let mut env = AppEnvelope::new("echo-app", sample_blueprint());
        env.description = "demo".to_string();
        env.tags = vec!["demo".to_string()];
        let canon = env.to_canonical_json().unwrap();
        // sorted: blueprint before description before name before schema before ...
        let s = String::from_utf8(canon.clone()).unwrap();
        assert!(
            s.starts_with("{\"blueprint\":"),
            "keys must be sorted, got {s}"
        );
        // round-trip: parse → canonical bytes are identical.
        let again = AppEnvelope::from_json_slice(&canon).unwrap();
        assert_eq!(again.to_canonical_json().unwrap(), canon);
        assert_eq!(again, env);
    }

    #[test]
    fn pretty_round_trips_to_same_canonical_bytes() {
        let env = AppEnvelope::new("echo-app", sample_blueprint());
        let pretty = env.to_pretty_json().unwrap();
        assert!(pretty.ends_with("}\n"));
        let from_pretty = AppEnvelope::from_json_slice(pretty.as_bytes()).unwrap();
        assert_eq!(
            from_pretty.to_canonical_json().unwrap(),
            env.to_canonical_json().unwrap()
        );
    }

    #[test]
    fn preserve_order_is_off_pin() {
        // If serde_json's `preserve_order` is ever enabled (transitively), Value maps
        // become insertion-ordered and the canonical contract breaks. Pin sorted order.
        let v: Value = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn empty_fields_are_omitted() {
        let env = AppEnvelope::new("x", json!({"steps": []}));
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert!(
            !s.contains("references"),
            "empty references must be omitted: {s}"
        );
        assert!(
            !s.contains("steering_config"),
            "empty steering must be omitted: {s}"
        );
        assert!(!s.contains("replay"));
        assert!(!s.contains("branch_handle"));
        // required fields always present.
        assert!(s.contains("\"schema\":\"kortecx.app/v1\""));
        assert!(s.contains("\"version\":\"1\""));
    }

    #[test]
    fn reach_default_is_omitted_from_canonical_bytes() {
        // A default (Explicit) reach must not emit a `reach` key — the byte-invariance
        // guard that keeps every pre-reach App's canonical bytes unchanged.
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.steering_config.tools.requested_grants = [("echo".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        assert_eq!(env.steering_config.tools.reach, Reach::Explicit);
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert!(
            s.contains("\"requested_grants\""),
            "the wish is present: {s}"
        );
        assert!(
            !s.contains("\"reach\""),
            "default reach must be omitted: {s}"
        );
    }

    #[test]
    fn reach_inherit_principal_round_trips() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.steering_config.tools.reach = Reach::InheritPrincipal;
        let canon = env.to_canonical_json().unwrap();
        let s = String::from_utf8(canon.clone()).unwrap();
        assert!(
            s.contains("\"reach\":\"inherit_principal\""),
            "inherit_principal serializes snake_case: {s}"
        );
        let again = AppEnvelope::from_json_slice(&canon).unwrap();
        assert_eq!(again.steering_config.tools.reach, Reach::InheritPrincipal);
        assert_eq!(again.to_canonical_json().unwrap(), canon);
    }

    #[test]
    fn lone_inherit_principal_keeps_the_tools_axis_emitted() {
        // `ToolsSteering::is_empty` gates the whole `tools` axis; a reach-only steering
        // must still emit (else the authority intent is silently dropped from the bytes).
        let mut ts = ToolsSteering::default();
        assert!(ts.is_empty());
        ts.reach = Reach::InheritPrincipal;
        assert!(!ts.is_empty(), "a lone InheritPrincipal is not empty");
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.steering_config.tools = ts;
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert!(s.contains("\"steering_config\""), "steering emitted: {s}");
        assert!(
            s.contains("\"tools\":{\"reach\":\"inherit_principal\"}"),
            "tools axis emitted: {s}"
        );
    }

    #[test]
    fn unknown_steering_field_is_ignored_forward_compat() {
        // No `deny_unknown_fields`: an envelope from a NEWER binary carrying an unknown
        // steering key must parse (ignored) and re-canonicalize without it, so old↔new
        // binaries interoperate on the tools axis while the known `reach` survives.
        let bytes = br#"{"blueprint":{"steps":[]},"name":"x","schema":"kortecx.app/v1","steering_config":{"tools":{"future_field":"whatever","reach":"inherit_principal"}},"version":"1"}"#;
        let env = AppEnvelope::from_json_slice(bytes).unwrap();
        assert_eq!(env.steering_config.tools.reach, Reach::InheritPrincipal);
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert!(!s.contains("future_field"), "unknown field dropped: {s}");
        assert!(
            s.contains("\"reach\":\"inherit_principal\""),
            "known field kept: {s}"
        );
    }

    #[test]
    fn validate_rejects_bad_schema() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.schema = "kortecx.app/v2".to_string();
        assert!(matches!(env.validate(), Err(AppError::Schema { .. })));
    }

    #[test]
    fn validate_rejects_short_ref() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.prompts.push(ArtifactRef {
            name: "p".into(),
            content_ref: "abc".into(),
        });
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_url_userinfo() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.connections.push(ConnectionRef {
            descriptor: "https://user:pw@evil.example/mcp".into(),
            credential_ref: String::new(),
        });
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_schemeless_userinfo() {
        // A scheme-less `user:pw@host` descriptor carries userinfo in the same
        // authority position as a `scheme://` URL. Before the fix the guard only
        // fired on `://`, so this smuggled a secret past validate().
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.connections.push(ConnectionRef {
            descriptor: "user:pw@evil.example/mcp".into(),
            credential_ref: String::new(),
        });
        assert!(
            env.validate().is_err(),
            "scheme-less userinfo must be rejected"
        );
    }

    #[test]
    fn validate_accepts_clean_schemeless_descriptor() {
        // A clean scheme-less descriptor (no userinfo) still validates — the widened
        // guard must add no false positive.
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.connections.push(ConnectionRef {
            descriptor: "mcp-echo/echo".into(),
            credential_ref: String::new(),
        });
        env.validate().unwrap();
    }

    #[test]
    fn validate_rejects_malformed_references_tool_id() {
        // `references.tools` was never validated: a three-segment / uppercase id is
        // not a tool id and must be refused.
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.tools.push(ToolRef {
            tool_id: "Bad/ID/x".into(),
            tool_version: "1".into(),
        });
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_integer_tool_version() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.tools.push(ToolRef {
            tool_id: "mcp-echo/echo".into(),
            tool_version: "v1".into(),
        });
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_malformed_skill_tool_id() {
        // A skill's tool wish set was also unvalidated (only its instructions_ref was).
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.skills.push(SkillRef {
            name: "s".into(),
            instructions_ref: hexref(0xaa),
            tools: [("BAD".to_string(), "1".to_string())].into_iter().collect(),
        });
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_malformed_requested_grant_id() {
        // The load-bearing grant wish: a hand-authored envelope could "request" a
        // tool whose id is not even a tool id.
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.steering_config.tools.requested_grants = [("bad id".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_accepts_wellformed_tool_rails() {
        // Well-formed ids/versions across all three tool rails validate cleanly.
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.tools.push(ToolRef {
            tool_id: "mcp-echo/echo".into(),
            tool_version: "1".into(),
        });
        env.references.skills.push(SkillRef {
            name: "s".into(),
            instructions_ref: hexref(0xbb),
            tools: [("retrieve".to_string(), "2".to_string())]
                .into_iter()
                .collect(),
        });
        env.steering_config.tools.requested_grants =
            [("gmail/search".to_string(), "3".to_string())]
                .into_iter()
                .collect();
        env.validate().unwrap();
    }

    #[test]
    fn validate_rejects_floats() {
        let env = AppEnvelope::new("x", json!({"steps": [], "weight": 1.5}));
        assert!(env.validate().is_err());
    }

    #[test]
    fn summary_counts_steps() {
        let env = AppEnvelope::new("x", sample_blueprint());
        assert_eq!(env.summary().step_count, 1);
        assert_eq!(env.summary().kind, AppKind::Functional);
    }

    #[test]
    fn functional_canonical_bytes_are_unchanged_by_the_kind_additions() {
        // THE DIGEST-INVARIANCE GUARD (D213 / `7d22d4bd`). Adding the Experience lane must
        // add ZERO bytes to any existing Functional envelope: no `kind` key (derived from
        // schema, never serialized) and no `hosted` key (None ⇒ omitted). The exact canonical
        // string below is what a minimal Functional envelope produced BEFORE this change — if
        // a stray key ever leaks in, this pins it.
        let env = AppEnvelope::new("x", json!({"steps": []}));
        let canon = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert_eq!(
            canon,
            r#"{"blueprint":{"steps":[]},"name":"x","schema":"kortecx.app/v1","version":"1"}"#,
            "Functional canonical bytes must be byte-identical to the pre-kind form"
        );
        assert!(
            !canon.contains("hosted"),
            "no hosted key on a functional app"
        );
        assert!(
            !canon.contains("\"kind\""),
            "kind is derived, never serialized"
        );
    }

    #[test]
    fn experience_envelope_round_trips_and_omits_blueprint() {
        let env = AppEnvelope::new_experience(
            "landing",
            HostedConfig {
                framework: HostedFramework::ViteReact,
                ..Default::default()
            },
            "app/landing/main",
        );
        env.validate().unwrap();
        assert_eq!(env.kind(), AppKind::Experience);
        assert_eq!(env.summary().kind, AppKind::Experience);
        assert_eq!(env.summary().step_count, 0);
        let canon = env.to_canonical_json().unwrap();
        let s = String::from_utf8(canon.clone()).unwrap();
        assert!(
            !s.contains("blueprint"),
            "experience carries no blueprint: {s}"
        );
        assert!(s.contains("\"schema\":\"kortecx.experience/v1\""), "{s}");
        assert!(s.contains("\"framework\":\"vite_react\""), "{s}");
        // round-trip identical
        let again = AppEnvelope::from_json_slice(&canon).unwrap();
        assert_eq!(again, env);
        assert_eq!(again.to_canonical_json().unwrap(), canon);
    }

    #[test]
    fn validate_rejects_experience_without_hosted() {
        let mut env = AppEnvelope::new_experience("x", HostedConfig::default(), "app/x/main");
        env.hosted = None;
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_experience_with_a_blueprint() {
        // An Experience app must NOT be schedulable — a smuggled blueprint is refused.
        let mut env = AppEnvelope::new_experience("x", HostedConfig::default(), "app/x/main");
        env.blueprint = Some(json!({"steps": []}));
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_experience_without_branch_handle() {
        let mut env = AppEnvelope::new_experience("x", HostedConfig::default(), "app/x/main");
        env.branch_handle = String::new();
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_functional_carrying_a_hosted_config() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.hosted = Some(HostedConfig::default());
        assert!(env.validate().is_err());
    }

    #[test]
    fn validate_rejects_functional_without_blueprint() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.blueprint = None;
        assert!(env.validate().is_err());
    }

    /// A distinct, valid 64-char lowercase-hex ref per seed byte.
    fn hexref(seed: u8) -> String {
        format!("{seed:02x}").repeat(32)
    }

    #[test]
    fn content_refs_walks_every_artifact_field_sorted_and_deduped() {
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.context.push(ContextRef {
            name: "c".into(),
            content_ref: hexref(0x01),
            media_type: String::new(),
        });
        env.references.prompts.push(ArtifactRef {
            name: "p".into(),
            content_ref: hexref(0x02),
        });
        env.references.rules.push(ArtifactRef {
            name: "r".into(),
            content_ref: hexref(0x03),
        });
        env.references.memory.push(ArtifactRef {
            name: "m".into(),
            content_ref: hexref(0x04),
        });
        env.references.skills.push(SkillRef {
            name: "s".into(),
            instructions_ref: hexref(0x05),
            tools: BTreeMap::new(),
        });
        env.steering_config.context.context_refs.push(hexref(0x06));
        // A dataset ref — travels ONLY with include_datasets.
        env.references.datasets.push(DatasetRef {
            dataset_ref: "d".into(),
            cas_refs: vec![hexref(0x07)],
        });
        // A duplicate across two fields MUST collapse to one.
        env.references.prompts.push(ArtifactRef {
            name: "dup".into(),
            content_ref: hexref(0x01),
        });
        // The envelope is still valid (every ref is 64-hex).
        env.validate().unwrap();

        let without = env.content_refs(false);
        assert_eq!(
            without,
            vec![
                hexref(0x01),
                hexref(0x02),
                hexref(0x03),
                hexref(0x04),
                hexref(0x05),
                hexref(0x06),
            ],
            "artifact rail only, sorted + deduped, datasets excluded"
        );

        let with = env.content_refs(true);
        assert!(
            with.contains(&hexref(0x07)),
            "dataset ref travels with --with-data"
        );
        assert_eq!(with.len(), without.len() + 1);
    }

    #[test]
    fn content_refs_skips_empty_refs() {
        // A default (empty content_ref) artifact must not yield a bogus "" ref.
        let mut env = AppEnvelope::new("x", json!({"steps": []}));
        env.references.prompts.push(ArtifactRef::default());
        assert!(env.content_refs(true).is_empty());
    }

    proptest! {
        /// `content_refs(true)` is sorted, deduplicated, and set-equal to the union of
        /// every content ref placed across the rail — over the arbitrary ref space
        /// (SN-4 v2 #5).
        #[test]
        fn content_refs_is_sorted_deduped_and_complete(
            seeds in prop::collection::vec(any::<u8>(), 0..40)
        ) {
            let mut env = AppEnvelope::new("x", json!({"steps": []}));
            for (i, s) in seeds.iter().enumerate() {
                match i % 3 {
                    0 => env.references.prompts.push(ArtifactRef {
                        name: "p".into(),
                        content_ref: hexref(*s),
                    }),
                    1 => env.references.skills.push(SkillRef {
                        name: "s".into(),
                        instructions_ref: hexref(*s),
                        tools: BTreeMap::new(),
                    }),
                    _ => env.steering_config.context.context_refs.push(hexref(*s)),
                }
            }
            let refs = env.content_refs(true);
            let mut sorted = refs.clone();
            sorted.sort();
            prop_assert_eq!(&refs, &sorted);
            let mut deduped = refs.clone();
            deduped.dedup();
            prop_assert_eq!(&refs, &deduped);
            let got: std::collections::BTreeSet<String> = refs.into_iter().collect();
            let expect: std::collections::BTreeSet<String> =
                seeds.iter().map(|s| hexref(*s)).collect();
            prop_assert_eq!(got, expect);
        }
    }

    /// THE LOAD-BEARING PROPERTY of the mode field: adding it must cost an existing App
    /// exactly nothing. `skip_serializing_if` means an unset mode contributes no bytes, so
    /// every envelope authored before it existed canonicalizes identically — and `app_ref` /
    /// `app_digest`, which are hashes of exactly these bytes, cannot move.
    #[test]
    fn an_unset_mode_adds_no_bytes() {
        let env = AppEnvelope::new("x", sample_blueprint());
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        // Match the KEY, not the substring: `"kind":"model"` contains "mode".
        assert!(
            !s.contains("\"mode\":"),
            "an unset mode must be omitted: {s}"
        );
        // And the old bytes still parse, with the field defaulted rather than demanded.
        let old =
            br#"{"blueprint":{"steps":[]},"name":"x","schema":"kortecx.app/v1","version":"1"}"#;
        let parsed = AppEnvelope::from_json_slice(old).unwrap();
        assert_eq!(parsed.mode, "");
        assert_eq!(parsed.mode(), AppMode::Contextual);
        assert_eq!(parsed.to_canonical_json().unwrap(), old.to_vec());
    }

    /// The same load-bearing property for `delivers`: an App authored before the composition
    /// registry existed must canonicalize to the SAME bytes, so its `app_ref` / `app_digest`
    /// — and therefore every stored catalog row and every export that pins one — cannot move.
    #[test]
    fn an_unset_delivers_adds_no_bytes() {
        let env = AppEnvelope::new("x", sample_blueprint());
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert!(
            !s.contains("\"delivers\""),
            "an unset delivers must be omitted: {s}"
        );
        let old =
            br#"{"blueprint":{"steps":[]},"name":"x","schema":"kortecx.app/v1","version":"1"}"#;
        let parsed = AppEnvelope::from_json_slice(old).unwrap();
        assert_eq!(parsed.delivers, "");
        assert_eq!(parsed.summary().delivers, "");
        assert_eq!(parsed.to_canonical_json().unwrap(), old.to_vec());
    }

    /// `delivers` is advisory prose that must survive the round trip intact and reach the
    /// SUMMARY — the summary is what `ListApps` projects, and the composition menu is built
    /// from that projection, so a value that stops at the envelope would leave the menu blank.
    #[test]
    fn delivers_round_trips_onto_the_summary() {
        let mut env = AppEnvelope::new("x", sample_blueprint());
        env.delivers = "a ranked shortlist of candidate suppliers".to_string();
        let canon = env.to_canonical_json().unwrap();
        let again = AppEnvelope::from_json_slice(&canon).unwrap();
        assert_eq!(
            again.summary().delivers,
            "a ranked shortlist of candidate suppliers"
        );
        assert_eq!(again.to_canonical_json().unwrap(), canon);
    }

    #[test]
    fn a_codified_mode_round_trips() {
        let mut env = AppEnvelope::new("x", sample_blueprint());
        env.mode = "codified".to_string();
        let canon = env.to_canonical_json().unwrap();
        let again = AppEnvelope::from_json_slice(&canon).unwrap();
        assert_eq!(again.mode(), AppMode::Codified);
        assert_eq!(again.summary().mode, AppMode::Codified);
        assert_eq!(again.to_canonical_json().unwrap(), canon);
    }

    /// A typo must be an honest authoring error, not a silent downgrade to a text app: the
    /// user asked for a project and would otherwise get prose with nothing to point at.
    #[test]
    fn an_unknown_mode_is_refused_at_validate() {
        let mut env = AppEnvelope::new("x", sample_blueprint());
        env.mode = "codifed".to_string(); // sic
        let err = env.validate().unwrap_err().to_string();
        assert!(err.contains("unknown app mode"), "{err}");
        // ...while the READ path still degrades safely for anything that got past validate.
        assert_eq!(AppMode::from_label("codifed"), AppMode::Contextual);
        assert_eq!(AppMode::from_label(""), AppMode::Contextual);
    }

    /// The mode axis belongs to the scheduled lane. A hosted app is a project by
    /// construction, so carrying the field would give two surfaces different answers about
    /// what an Experience app is.
    #[test]
    fn an_experience_app_must_not_carry_a_mode() {
        let mut env = AppEnvelope::new_experience("x", HostedConfig::default(), "acme/apps/x");
        env.validate().expect("a hosted app with no mode is valid");
        env.mode = "codified".to_string();
        let err = env.validate().unwrap_err().to_string();
        assert!(err.contains("must not carry a mode"), "{err}");
    }

    /// A hosted app DECLARES the apps its served page may call back and run — its
    /// `references.apps` is the page's runnable set, read by the supervisor to scope the
    /// page's token. Once refused (a Functional-only reading of the field); now accepted,
    /// because the field means "the apps this one may invoke" on both lanes.
    #[test]
    fn an_experience_app_may_declare_the_apps_its_page_can_run() {
        let mut env = AppEnvelope::new_experience("x", HostedConfig::default(), "acme/apps/x");
        env.references.apps.push(AppRef {
            handle: "apps/local/greeting".into(),
        });
        env.validate()
            .expect("a hosted app may declare the apps its page can run");
        // And it survives the canonical round trip on the reference rail.
        let again = AppEnvelope::from_json_slice(&env.to_canonical_json().unwrap()).unwrap();
        assert_eq!(again.references.apps.len(), 1);
    }

    #[test]
    fn mode_labels_are_stable() {
        assert_eq!(AppMode::Contextual.as_str(), "contextual");
        assert_eq!(AppMode::Codified.as_str(), "codified");
        assert_eq!(AppMode::default(), AppMode::Contextual);
        for m in [AppMode::Contextual, AppMode::Codified] {
            assert_eq!(AppMode::from_label(m.as_str()), m);
        }
    }
}
