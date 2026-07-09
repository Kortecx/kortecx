//! The [`AppEnvelope`] type + its canonical (de)serialization and validation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The envelope schema/version tag. Readers fail closed on a mismatch.
pub const APP_SCHEMA: &str = "kortecx.app/v1";

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

/// A `kortecx.app/v1` envelope: a portable blueprint wrapped with references, a
/// steering config, replay intent, and an optional project branch handle. Carries
/// NO authority. See the crate docs for the full contract.
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
    /// Catalog tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional input schema (opaque JSON), e.g. a JSON-schema for `run` args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// The portable blueprint (a `DagSpec`) carried VERBATIM as opaque JSON.
    pub blueprint: Value,
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
    /// Tags.
    pub tags: Vec<String>,
    /// Number of blueprint steps (display only).
    pub step_count: u32,
}

impl AppEnvelope {
    /// A minimal envelope wrapping `blueprint` under `name`, schema + version preset.
    #[must_use]
    pub fn new(name: impl Into<String>, blueprint: Value) -> Self {
        Self {
            schema: APP_SCHEMA.to_string(),
            name: name.into(),
            version: default_version(),
            description: String::new(),
            tags: Vec::new(),
            input_schema: None,
            blueprint,
            references: References::default(),
            steering_config: SteeringConfig::default(),
            replay: Replay::default(),
            branch_handle: String::new(),
        }
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
            .get("steps")
            .and_then(Value::as_array)
            .map_or(0, |s| u32::try_from(s.len()).unwrap_or(u32::MAX));
        AppSummary {
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
            step_count,
        }
    }

    /// Validate structure + the security boundary:
    /// - `schema` is [`APP_SCHEMA`];
    /// - `blueprint` is a JSON object;
    /// - every content/instructions/cas ref is 64-char lowercase hex;
    /// - connection descriptors carry NO URL userinfo and credential refs are bare names;
    /// - no floats anywhere (SN-8 — identity bytes are integer-only).
    ///
    /// # Errors
    /// Returns [`AppError::Schema`] on a schema-tag mismatch, or [`AppError::Invalid`]
    /// for a non-object blueprint, a malformed ref, URL userinfo in a connection
    /// descriptor, a non-bare credential name, or any float.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema != APP_SCHEMA {
            return Err(AppError::Schema {
                got: self.schema.clone(),
                expected: APP_SCHEMA,
            });
        }
        if !self.blueprint.is_object() {
            return Err(AppError::Invalid("blueprint must be a JSON object".into()));
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
        for r in &self.steering_config.context.context_refs {
            check_ref("steering.context_ref", r)?;
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

/// Reject a connection descriptor that smuggles URL userinfo (`scheme://user:pw@host`).
fn check_descriptor_no_userinfo(descriptor: &str) -> Result<(), AppError> {
    if let Some(after_scheme) = descriptor.split_once("://").map(|(_, rest)| rest) {
        // authority ends at the first '/', '?', or '#'.
        let authority = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(after_scheme);
        if authority.contains('@') {
            return Err(AppError::Invalid(format!(
                "connection descriptor must not carry URL userinfo, got {descriptor:?}"
            )));
        }
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
    fn validate_rejects_floats() {
        let env = AppEnvelope::new("x", json!({"steps": [], "weight": 1.5}));
        assert!(env.validate().is_err());
    }

    #[test]
    fn summary_counts_steps() {
        let env = AppEnvelope::new("x", sample_blueprint());
        assert_eq!(env.summary().step_count, 1);
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
}
