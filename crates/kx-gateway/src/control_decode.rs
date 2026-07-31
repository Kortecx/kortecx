//! The NL-CONTROL decoder: model bytes → a typed [`ControlProposal`].
//!
//! The `derive_plan.rs` discipline verbatim: `deny_unknown_fields` everywhere, a byte cap
//! before parsing, and a REFUSAL rather than a trim when the model invents an axis.
//!
//! ## Why an invented key is refused rather than dropped
//!
//! Silently ignoring an unknown field means the human reviews a form that does not say what
//! the model meant. If the model wrote `"value": "hunter2"` into a secret proposal, dropping
//! it would render a clean preview of a request that was never what the model produced — and
//! the reviewer would approve the clean one having never seen the other. A refusal is the
//! only outcome where what is displayed is what was proposed.
//!
//! ## The forbidden keys are refused TWICE, deliberately
//!
//! `value` / `password` / `token` / `argv` / `env` are not merely absent from the field
//! structs — `deny_unknown_fields` would already reject them with a generic "unknown field"
//! message. They are ALSO named explicitly, so the refusal says WHY, and so a reader of this
//! file can see the list without reconstructing it from the absence of struct members.
//! Defence in depth costs one match arm here; the alternative is a security property nobody
//! can find.

use kx_gateway_core::{
    ConnectorProposal, ControlProposal, PolicyRoleRow, PolicyRoleToolWire, ScriptProposal,
    SecretProposal, ToolProposal, TriggerProposal,
};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The hard byte cap on a control form. One registration is small; anything larger is a
/// runaway generation, and parsing it would only turn a budget failure into a parse failure.
pub(crate) const MAX_CONTROL_BYTES: usize = 16 * 1024;

/// Field keys a proposal may never carry, whatever the domain.
///
/// Two families. `value` / `secret` / `password` / `token` are CREDENTIAL-shaped: a proposal
/// is displayed, logged and forwarded, so one carrying a credential discloses it by design.
/// `argv` / `env` are EXECUTION-shaped: both are fixed at registration by an operator and are
/// documented "NEVER model-controlled", so a model that could set them would hold the one
/// axis it must not.
const FORBIDDEN_KEYS: &[&str] = &["value", "secret", "password", "token", "argv", "env"];

/// Why a control form was not accepted.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlDecodeError {
    /// Over [`MAX_CONTROL_BYTES`].
    #[error("the control form is {got} bytes, over the {MAX_CONTROL_BYTES} byte cap")]
    Oversize {
        /// The size that was refused.
        got: usize,
    },
    /// Not UTF-8.
    #[error("the control form is not valid UTF-8")]
    NotUtf8,
    /// Malformed, unknown key, or unknown domain.
    #[error("{0}")]
    Malformed(String),
    /// A key that must never appear in a proposal.
    #[error(
        "the form carries `{0}`, which a proposal may never contain — credentials and script \
         argv/env are set by an operator, never proposed"
    )]
    Forbidden(String),
}

/// The strict outer envelope.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlEnvelope {
    control: ControlForm,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlForm {
    domain: String,
    name: String,
    #[serde(default)]
    fields: BTreeMap<String, serde_json::Value>,
}

/// Decode model bytes into a typed proposal.
///
/// Strips a leading code fence the way the sibling decoders do — a fenced block is a
/// formatting habit, not a different intent, and refusing it would be pedantry that costs a
/// real proposal.
#[allow(clippy::too_many_lines)] // a flat per-domain dispatcher; splitting it would
                                 // scatter the field vocabulary the contract teaches
pub(crate) fn decode_control(bytes: &[u8]) -> Result<ControlProposal, ControlDecodeError> {
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(ControlDecodeError::Oversize { got: bytes.len() });
    }
    let raw = std::str::from_utf8(bytes).map_err(|_| ControlDecodeError::NotUtf8)?;
    let raw = strip_fence(raw);

    let env: ControlEnvelope = serde_json::from_str(raw)
        .map_err(|e| ControlDecodeError::Malformed(format!("not a valid control form: {e}")))?;
    let form = env.control;

    if form.name.trim().is_empty() {
        return Err(ControlDecodeError::Malformed(
            "the form has no `name` — every registration needs an operator handle".into(),
        ));
    }
    // Check the forbidden keys BEFORE per-domain decoding, so the reason is the real one
    // rather than whatever the domain's own `deny_unknown_fields` happens to say first.
    for key in form.fields.keys() {
        if FORBIDDEN_KEYS.contains(&key.as_str()) {
            return Err(ControlDecodeError::Forbidden(key.clone()));
        }
    }

    let fields = serde_json::Value::Object(form.fields.into_iter().collect());
    let name = form.name.trim().to_string();

    let malformed = |e: serde_json::Error| {
        ControlDecodeError::Malformed(format!(
            "the {} form is not admissible: {e}",
            "requested domain"
        ))
    };

    match form.domain.trim() {
        "workflows" => {
            let f: WorkflowFields = serde_json::from_value(fields).map_err(malformed)?;
            Ok(ControlProposal::SaveWorkflow(
                kx_gateway_core::WorkflowSaveProposal {
                    handle: name,
                    envelope_json: f.envelope_json.unwrap_or_default().into_bytes(),
                    lifecycle: f.lifecycle.unwrap_or_default(),
                },
            ))
        }
        "tools" => {
            let f: ToolFields = serde_json::from_value(fields).map_err(malformed)?;
            Ok(ControlProposal::RegisterTool(ToolProposal {
                tool_name: name,
                tool_version: f.tool_version.unwrap_or_else(|| "1".into()),
                description: f.description.unwrap_or_default(),
                idempotency_class: f.idempotency_class.unwrap_or_else(|| "Token".into()),
                server_host: f.server_host.unwrap_or_default(),
                remote_name: f.remote_name.unwrap_or_default(),
            }))
        }
        "connectors" => {
            let f: ConnectorFields = serde_json::from_value(fields).map_err(malformed)?;
            Ok(ControlProposal::RegisterMcpServer(ConnectorProposal {
                server_name: name,
                transport: f.transport.unwrap_or_else(|| "http".into()),
                endpoint: f.endpoint.unwrap_or_default(),
                args: f.args.unwrap_or_default(),
                tls_required: f.tls_required.unwrap_or(true),
                credential_ref: f.credential_ref.unwrap_or_default(),
                session_mode: f.session_mode.unwrap_or_default(),
            }))
        }
        "secrets" => {
            let f: SecretFields = serde_json::from_value(fields).map_err(malformed)?;
            Ok(ControlProposal::PutSecret(SecretProposal {
                name,
                secret_scope: f.secret_scope.unwrap_or_default(),
                net_scope: f.net_scope.unwrap_or_default(),
            }))
        }
        "scripts" => {
            let f: ScriptFields = serde_json::from_value(fields).map_err(malformed)?;
            Ok(ControlProposal::RegisterScript(ScriptProposal {
                script_name: name,
                script_version: f.script_version.unwrap_or_else(|| "1".into()),
                description: f.description.unwrap_or_default(),
                interpreter: f.interpreter.unwrap_or_default(),
                source: f.source.unwrap_or_default().into_bytes(),
                fs_mounts: Vec::new(),
                net_hosts: f.net_hosts.unwrap_or_default(),
                wall_clock_ms: f.wall_clock_ms.unwrap_or(0),
                mem_bytes: 0,
                max_output_bytes: 0,
            }))
        }
        "triggers" => {
            let f: TriggerFields = serde_json::from_value(fields).map_err(malformed)?;
            Ok(ControlProposal::RegisterTrigger(TriggerProposal {
                name,
                kind: f.kind.unwrap_or_else(|| "cron".into()),
                auth: f.auth.unwrap_or_else(|| "none".into()),
                auth_secret_ref: f.auth_secret_ref.unwrap_or_default(),
                schedule_spec: f.schedule_spec.unwrap_or_default(),
                timezone: f.timezone.unwrap_or_default(),
                enabled: f.enabled.unwrap_or(true),
                require_approval: f.require_approval.unwrap_or(false),
                recipe_handle: f.recipe_handle.unwrap_or_default(),
                app_handle: f.app_handle.unwrap_or_default(),
                workflow_handle: f.workflow_handle.unwrap_or_default(),
            }))
        }
        "policy" => {
            let f: PolicyFields = serde_json::from_value(fields).map_err(malformed)?;
            // Assigning and defining are different acts. A form naming a party is an
            // ASSIGNMENT; one naming tools defines the role. Guessing between them would
            // silently narrow the wrong thing.
            if let Some(party) = f.party.filter(|p| !p.trim().is_empty()) {
                return Ok(ControlProposal::AssignPolicyRole { party, role: name });
            }
            Ok(ControlProposal::PutPolicyRole(PolicyRoleRow {
                name,
                description: f.description.unwrap_or_default(),
                tools: f
                    .tools
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|t| {
                        t.split_once('@').map(|(id, ver)| PolicyRoleToolWire {
                            tool_id: id.to_string(),
                            tool_version: ver.to_string(),
                        })
                    })
                    .collect(),
                created_unix_ms: 0,
                updated_unix_ms: 0,
            }))
        }
        other => Err(ControlDecodeError::Malformed(format!(
            "unknown domain {other:?}: expected workflows | tools | connectors | secrets | \
             scripts | triggers | policy"
        ))),
    }
}

macro_rules! fields {
    ($name:ident { $( $f:ident : $t:ty ),* $(,)? }) => {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $name { $( #[serde(default)] $f: Option<$t> ),* }
    };
}

fields!(WorkflowFields {
    envelope_json: String,
    lifecycle: String
});
fields!(ToolFields {
    tool_version: String,
    description: String,
    idempotency_class: String,
    server_host: String,
    remote_name: String,
});
fields!(ConnectorFields {
    transport: String,
    endpoint: String,
    args: Vec<String>,
    tls_required: bool,
    credential_ref: String,
    session_mode: String,
});
fields!(SecretFields {
    secret_scope: String,
    net_scope: String
});
fields!(ScriptFields {
    script_version: String,
    description: String,
    interpreter: String,
    source: String,
    net_hosts: Vec<String>,
    wall_clock_ms: u64,
});
fields!(TriggerFields {
    kind: String,
    auth: String,
    auth_secret_ref: String,
    schedule_spec: String,
    timezone: String,
    enabled: bool,
    require_approval: bool,
    recipe_handle: String,
    app_handle: String,
    workflow_handle: String,
});
fields!(PolicyFields { description: String, tools: Vec<String>, party: String });

/// Strip a ```json fence, if present.
fn strip_fence(raw: &str) -> &str {
    let t = raw.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.trim_start_matches('\n')
        .rsplit_once("```")
        .map_or(rest, |(body, _)| body)
        .trim()
}

#[cfg(test)]
mod tests {
    use super::{decode_control, ControlDecodeError, MAX_CONTROL_BYTES};
    use kx_gateway_core::ControlProposal;

    #[test]
    fn the_taught_example_decodes() {
        // Byte-identical to the example inside CONTROL_SYSTEM. A contract whose example
        // does not decode teaches a shape nothing accepts.
        let raw = r#"{"control":{"domain":"triggers","name":"nightly-report","fields":{"kind":"cron","schedule_spec":"0 9 * * *","app_handle":"ops/reports/daily"}}}"#;
        match decode_control(raw.as_bytes()).unwrap() {
            ControlProposal::RegisterTrigger(t) => {
                assert_eq!(t.name, "nightly-report");
                assert_eq!(t.kind, "cron");
                assert_eq!(t.schedule_spec, "0 9 * * *");
                assert_eq!(t.app_handle, "ops/reports/daily");
            }
            other => panic!("expected a trigger, got {other:?}"),
        }
    }

    /// Every forbidden key is refused, and the refusal NAMES it.
    #[test]
    fn a_credential_or_argv_key_is_refused_by_name() {
        for key in ["value", "secret", "password", "token", "argv", "env"] {
            let raw = format!(
                r#"{{"control":{{"domain":"secrets","name":"API_KEY","fields":{{"{key}":"x"}}}}}}"#
            );
            match decode_control(raw.as_bytes()) {
                Err(ControlDecodeError::Forbidden(k)) => assert_eq!(k, key),
                other => panic!("{key} must be Forbidden, got {other:?}"),
            }
        }
    }

    /// An invented axis is REFUSED, never trimmed. See the module doc.
    #[test]
    fn an_unknown_field_is_refused_rather_than_dropped() {
        let raw =
            r#"{"control":{"domain":"secrets","name":"API_KEY","fields":{"expires_at":"soon"}}}"#;
        assert!(matches!(
            decode_control(raw.as_bytes()),
            Err(ControlDecodeError::Malformed(_))
        ));
    }

    #[test]
    fn an_unknown_domain_names_the_alternatives() {
        let raw = r#"{"control":{"domain":"telepathy","name":"x","fields":{}}}"#;
        let Err(ControlDecodeError::Malformed(m)) = decode_control(raw.as_bytes()) else {
            panic!("expected Malformed");
        };
        assert!(m.contains("telepathy"));
        assert!(m.contains("workflows | tools | connectors"));
    }

    /// A policy form naming a party is an ASSIGNMENT; one naming tools DEFINES the role.
    #[test]
    fn policy_distinguishes_defining_from_assigning() {
        let define =
            r#"{"control":{"domain":"policy","name":"ops","fields":{"tools":["fs.read@1"]}}}"#;
        match decode_control(define.as_bytes()).unwrap() {
            ControlProposal::PutPolicyRole(r) => {
                assert_eq!(r.name, "ops");
                assert_eq!(r.tools.len(), 1);
                assert_eq!(r.tools[0].tool_id, "fs.read");
                assert_eq!(r.tools[0].tool_version, "1");
            }
            other => panic!("expected PutPolicyRole, got {other:?}"),
        }
        let assign =
            r#"{"control":{"domain":"policy","name":"ops","fields":{"party":"alice@acme"}}}"#;
        match decode_control(assign.as_bytes()).unwrap() {
            ControlProposal::AssignPolicyRole { party, role } => {
                assert_eq!(party, "alice@acme");
                assert_eq!(role, "ops");
            }
            other => panic!("expected AssignPolicyRole, got {other:?}"),
        }
    }

    /// A script form cannot express argv/env even by accident — the field struct has no
    /// such member, so the forbidden check and the decoder agree.
    #[test]
    fn a_script_form_lowers_with_empty_argv_and_env() {
        let raw = r#"{"control":{"domain":"scripts","name":"tidy","fields":{"interpreter":"sh","source":"echo hi"}}}"#;
        match decode_control(raw.as_bytes()).unwrap() {
            ControlProposal::RegisterScript(s) => {
                let reg = s.into_registration();
                assert!(reg.argv.is_empty());
                assert!(reg.env.is_empty());
                assert_eq!(reg.interpreter, "sh");
            }
            other => panic!("expected a script, got {other:?}"),
        }
    }

    #[test]
    fn a_fenced_reply_still_decodes() {
        let raw = "```json\n{\"control\":{\"domain\":\"secrets\",\"name\":\"API_KEY\",\"fields\":{}}}\n```";
        assert!(decode_control(raw.as_bytes()).is_ok());
    }

    #[test]
    fn an_oversize_form_is_refused_before_parsing() {
        let raw = vec![b'{'; MAX_CONTROL_BYTES + 1];
        assert!(matches!(
            decode_control(&raw),
            Err(ControlDecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn a_nameless_form_is_refused() {
        let raw = r#"{"control":{"domain":"secrets","name":"  ","fields":{}}}"#;
        assert!(matches!(
            decode_control(raw.as_bytes()),
            Err(ControlDecodeError::Malformed(_))
        ));
    }
}
