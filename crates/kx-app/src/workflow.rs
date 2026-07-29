//! The [`WorkflowEnvelope`] type — the durable, standalone Workflow entity
//! (`kortecx.workflow/v1`).
//!
//! A workflow is the SAME portable blueprint an App carries, wrapped with the
//! SAME by-reference rail and steering config — but it is a first-class entity
//! of its own: never hosted, never moded, runnable and schedulable as itself
//! rather than only "Saved as an App". The schema tag is the honest
//! discriminator (the D213 posture): `AppEnvelope` readers fail closed on a
//! workflow tag and vice versa, so the two catalogs can never swallow each
//! other's bytes.
//!
//! **Authority:** none, identically to an App envelope. The envelope carries
//! wishes; the server re-resolves every warrant from the caller's OWN grants
//! at run time.
//!
//! **Canonical bytes:** the same contract as [`AppEnvelope`] — keys sorted,
//! compact, integers only, every optional field `skip_serializing_if`-guarded
//! so an empty value adds ZERO bytes (the digest-invariance discipline).
//!
//! **Validation:** the schema/blueprint shape is checked here; the whole
//! reference-rail + float discipline is delegated to the converted
//! [`AppEnvelope`]'s `validate` so the contract lives in exactly one place and
//! the two rails can never drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppEnvelope, AppError, References, Replay, SteeringConfig, APP_SCHEMA};

/// The envelope schema/version tag for a durable Workflow. Readers fail closed
/// on a mismatch (an App reader refuses these bytes and vice versa).
pub const WORKFLOW_SCHEMA: &str = "kortecx.workflow/v1";

fn default_version() -> String {
    "1".to_string()
}

/// A `kortecx.workflow/v1` envelope: a portable blueprint wrapped with
/// references, a steering config, and replay intent. Carries NO authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEnvelope {
    /// The schema/version tag — always [`WORKFLOW_SCHEMA`].
    pub schema: String,
    /// The workflow name (the human handle within the catalog).
    pub name: String,
    /// The workflow version (default `"1"`).
    #[serde(default = "default_version")]
    pub version: String,
    /// Free-form description.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What one run of this workflow PRODUCES, in one line (the
    /// `AppEnvelope::delivers` posture: advisory prose, never enforcement).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delivers: String,
    /// Catalog tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional input schema (opaque JSON) for `run` args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// The portable blueprint (a `DagSpec`) carried VERBATIM as opaque JSON.
    /// REQUIRED — a workflow without a blueprint is not a workflow (there is no
    /// hosted lane here), so the field is not optional and adds no
    /// present/absent ambiguity to the canonical bytes.
    pub blueprint: Value,
    /// The by-reference rail (shared with the App envelope).
    #[serde(default, skip_serializing_if = "References::is_empty")]
    pub references: References,
    /// The four-axis steering config (shared with the App envelope).
    #[serde(default, skip_serializing_if = "SteeringConfig::is_empty")]
    pub steering_config: SteeringConfig,
    /// Per-step replay intent (shared with the App envelope).
    #[serde(default, skip_serializing_if = "Replay::is_empty")]
    pub replay: Replay,
    /// Optional definition-history branch handle. The host records every saved
    /// definition as a branch version at the WORKFLOW handle, so this stays
    /// empty in practice; it exists so an exported envelope can carry the
    /// association explicitly without a second schema rev.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch_handle: String,
}

/// The envelope-derived catalog summary (the host adds handle + `workflow_ref`
/// + lifecycle, which are catalog state rather than envelope content).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowSummary {
    /// Workflow name.
    pub name: String,
    /// Workflow version.
    pub version: String,
    /// Description.
    pub description: String,
    /// What one run produces (the `delivers` line).
    pub delivers: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Number of blueprint steps (display only).
    pub step_count: u32,
}

impl WorkflowEnvelope {
    /// A minimal envelope wrapping `blueprint` under `name`, schema + version preset.
    #[must_use]
    pub fn new(name: impl Into<String>, blueprint: Value) -> Self {
        Self {
            schema: WORKFLOW_SCHEMA.to_string(),
            name: name.into(),
            version: default_version(),
            description: String::new(),
            delivers: String::new(),
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
    /// Returns [`AppError::Json`] if the bytes are not valid envelope JSON, or
    /// the [`AppError`] from [`WorkflowEnvelope::validate`] if the parsed
    /// envelope is invalid — including [`AppError::Schema`] for App-tagged
    /// bytes (the mutual-refusal contract).
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, AppError> {
        let env: Self = serde_json::from_slice(bytes)?;
        env.validate()?;
        Ok(env)
    }

    /// Canonical bytes: keys sorted (via [`serde_json::Value`]), compact, no
    /// floats — the hashable + on-the-wire form (the [`AppEnvelope`] contract).
    ///
    /// # Errors
    /// Returns [`AppError::Json`] if the envelope cannot be serialized (it
    /// never can in practice — the type holds only JSON-safe fields).
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

    /// Every content-store ref this workflow references (sorted, deduplicated;
    /// the [`AppEnvelope::content_refs`] walk, via the lossless conversion).
    #[must_use]
    pub fn content_refs(&self, include_datasets: bool) -> Vec<String> {
        self.to_app_envelope().content_refs(include_datasets)
    }

    /// The catalog summary derived from this envelope.
    #[must_use]
    pub fn summary(&self) -> WorkflowSummary {
        let step_count = self
            .blueprint
            .get("steps")
            .and_then(Value::as_array)
            .map_or(0, |s| u32::try_from(s.len()).unwrap_or(u32::MAX));
        WorkflowSummary {
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            delivers: self.delivers.clone(),
            tags: self.tags.clone(),
            step_count,
        }
    }

    /// The lossless Functional-App view of this workflow, used SERVER-SIDE at
    /// author time so `RunWorkflow` rides the exact `RunApp` preparation
    /// pipeline (lowering, reference resolution, warrant intersection).
    ///
    /// Identity NEVER derives from this form — `workflow_ref` and
    /// `workflow_digest` are computed over the WORKFLOW canonical bytes — so
    /// the conversion can stay a plain field copy without any byte-stability
    /// obligation of its own.
    #[must_use]
    pub fn to_app_envelope(&self) -> AppEnvelope {
        self.clone().into_app_envelope()
    }

    /// Owned variant of [`WorkflowEnvelope::to_app_envelope`] (no clone).
    #[must_use]
    pub fn into_app_envelope(self) -> AppEnvelope {
        AppEnvelope {
            schema: APP_SCHEMA.to_string(),
            name: self.name,
            version: self.version,
            description: self.description,
            delivers: self.delivers,
            tags: self.tags,
            input_schema: self.input_schema,
            blueprint: Some(self.blueprint),
            hosted: None,
            references: self.references,
            steering_config: self.steering_config,
            replay: self.replay,
            branch_handle: self.branch_handle,
            mode: String::new(),
        }
    }

    /// Validate structure + the security boundary:
    /// - `schema` is [`WORKFLOW_SCHEMA`] (App-tagged bytes are refused — the
    ///   mutual-exclusion contract with [`AppEnvelope`]);
    /// - `blueprint` is a JSON object;
    /// - everything else — the by-ref/by-name rail discipline, connection
    ///   userinfo, tool-id shapes, the no-floats rule — is the App envelope's
    ///   contract, checked via the lossless conversion so the two rails share
    ///   one validator and can never drift.
    ///
    /// # Errors
    /// Returns [`AppError::Schema`] on a schema-tag mismatch, or the
    /// [`AppError::Invalid`] the shared rail validation produces.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema != WORKFLOW_SCHEMA {
            return Err(AppError::Schema {
                got: self.schema.clone(),
                expected: WORKFLOW_SCHEMA,
            });
        }
        if !self.blueprint.is_object() {
            return Err(AppError::Invalid("blueprint must be a JSON object".into()));
        }
        self.to_app_envelope().validate()
    }
}

/// Re-canonicalize received workflow-envelope bytes (the gateway host derives
/// `workflow_ref` over this form, so client byte-ordering never affects
/// identity). Validates first.
///
/// # Errors
/// Returns the [`AppError`] from [`WorkflowEnvelope::from_json_slice`] if the
/// bytes are not a valid workflow envelope.
pub fn workflow_canonical_json(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    WorkflowEnvelope::from_json_slice(bytes)?.to_canonical_json()
}

/// Extract the catalog summary from received workflow-envelope bytes
/// (validates first).
///
/// # Errors
/// Returns the [`AppError`] from [`WorkflowEnvelope::from_json_slice`] if the
/// bytes are not a valid workflow envelope.
pub fn workflow_summary_of(bytes: &[u8]) -> Result<WorkflowSummary, AppError> {
    Ok(WorkflowEnvelope::from_json_slice(bytes)?.summary())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_blueprint() -> Value {
        json!({
            "seed": 0,
            "steps": [
                { "kind": "pure", "prompt": "" },
                { "kind": "model", "prompt": "Summarize.", "tool_contract": {} }
            ],
            "edges": [ { "parent": 0, "child": 1, "data": true } ]
        })
    }

    #[test]
    fn canonical_json_is_sorted_and_round_trips() {
        let mut env = WorkflowEnvelope::new("triage", sample_blueprint());
        env.description = "demo".to_string();
        env.tags = vec!["demo".to_string()];
        let canon = env.to_canonical_json().unwrap();
        let s = String::from_utf8(canon.clone()).unwrap();
        assert!(
            s.starts_with("{\"blueprint\":"),
            "keys must be sorted, got {s}"
        );
        let again = WorkflowEnvelope::from_json_slice(&canon).unwrap();
        assert_eq!(again.to_canonical_json().unwrap(), canon);
        assert_eq!(again, env);
    }

    #[test]
    fn empty_fields_are_omitted_so_identity_is_stable() {
        // The digest-invariance discipline: an all-default envelope emits ONLY
        // schema/name/version/blueprint. A new optional field that serialized
        // its default would move every stored workflow_ref — this pin is what
        // catches that at review time.
        let env = WorkflowEnvelope::new("x", json!({"steps": []}));
        let s = String::from_utf8(env.to_canonical_json().unwrap()).unwrap();
        assert_eq!(
            s,
            "{\"blueprint\":{\"steps\":[]},\"name\":\"x\",\"schema\":\"kortecx.workflow/v1\",\"version\":\"1\"}"
        );
    }

    #[test]
    fn pretty_round_trips_to_same_canonical_bytes() {
        let env = WorkflowEnvelope::new("triage", sample_blueprint());
        let pretty = env.to_pretty_json().unwrap();
        assert!(pretty.ends_with("}\n"));
        let from_pretty = WorkflowEnvelope::from_json_slice(pretty.as_bytes()).unwrap();
        assert_eq!(
            from_pretty.to_canonical_json().unwrap(),
            env.to_canonical_json().unwrap()
        );
    }

    #[test]
    fn schema_mutual_exclusion_both_directions() {
        // A workflow reader refuses App-tagged bytes …
        let app = AppEnvelope::new("a", sample_blueprint());
        let app_bytes = app.to_canonical_json().unwrap();
        assert!(matches!(
            WorkflowEnvelope::from_json_slice(&app_bytes),
            Err(AppError::Schema { .. })
        ));
        // … and an App reader refuses workflow-tagged bytes, so SaveApp and
        // SaveWorkflow can never silently swallow each other's envelopes.
        let wf = WorkflowEnvelope::new("w", sample_blueprint());
        let wf_bytes = wf.to_canonical_json().unwrap();
        assert!(matches!(
            AppEnvelope::from_json_slice(&wf_bytes),
            Err(AppError::Schema { .. })
        ));
    }

    #[test]
    fn blueprint_must_be_an_object() {
        let mut env = WorkflowEnvelope::new("w", sample_blueprint());
        env.blueprint = json!("not an object");
        assert!(matches!(env.validate(), Err(AppError::Invalid(_))));
    }

    #[test]
    fn rail_validation_is_delegated_not_duplicated() {
        // A malformed content ref is refused via the SHARED App-rail validator.
        let mut env = WorkflowEnvelope::new("w", sample_blueprint());
        env.references.context.push(crate::ContextRef {
            name: "notes".into(),
            content_ref: "not-hex".into(),
            media_type: String::new(),
        });
        assert!(matches!(env.validate(), Err(AppError::Invalid(_))));
        // And so is a float anywhere in the tree (identity bytes are integer-only).
        let mut env = WorkflowEnvelope::new("w", json!({"steps": [], "budget": 1.5}));
        assert!(matches!(env.validate(), Err(AppError::Invalid(_))));
        let _ = &mut env;
    }

    #[test]
    fn into_app_envelope_is_lossless_and_valid() {
        let mut env = WorkflowEnvelope::new("triage", sample_blueprint());
        env.description = "d".into();
        env.delivers = "a dispatch order".into();
        env.tags = vec!["ops".into()];
        env.steering_config.model.model_route = "gemma".into();
        env.steering_config.guards.max_turns = Some(4);
        let app = env.to_app_envelope();
        app.validate().expect("converted form validates");
        assert_eq!(app.schema, APP_SCHEMA);
        assert_eq!(app.blueprint.as_ref(), Some(&env.blueprint));
        assert_eq!(app.hosted, None);
        assert_eq!(app.mode, "");
        assert_eq!(app.name, env.name);
        assert_eq!(app.delivers, env.delivers);
        assert_eq!(
            app.steering_config.model.model_route,
            env.steering_config.model.model_route
        );
        // The summaries agree on the shared fields.
        let ws = env.summary();
        let as_ = app.summary();
        assert_eq!(ws.step_count, as_.step_count);
        assert_eq!(ws.name, as_.name);
        assert_eq!(ws.delivers, as_.delivers);
    }

    #[test]
    fn summary_counts_steps() {
        let env = WorkflowEnvelope::new("w", sample_blueprint());
        assert_eq!(env.summary().step_count, 2);
        assert_eq!(env.summary().name, "w");
    }

    #[test]
    fn free_functions_validate_first() {
        assert!(workflow_canonical_json(b"not json").is_err());
        assert!(workflow_summary_of(b"{}").is_err());
        let env = WorkflowEnvelope::new("w", sample_blueprint());
        let bytes = env.to_pretty_json().unwrap();
        // Pretty bytes re-canonicalize to the canonical form (client byte-order
        // never affects identity — the SaveApp posture).
        assert_eq!(
            workflow_canonical_json(bytes.as_bytes()).unwrap(),
            env.to_canonical_json().unwrap()
        );
        assert_eq!(workflow_summary_of(bytes.as_bytes()).unwrap().step_count, 2);
    }
}
