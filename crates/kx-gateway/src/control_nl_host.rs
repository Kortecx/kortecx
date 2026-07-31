//! `HostControlProposer` — the served-model side of the `ProposeControlAction` seam.
//!
//! Runs the served model ONCE with the [`crate::prompt_library::CONTROL_SYSTEM`] contract,
//! decodes the reply through [`crate::control_decode::decode_control`], and then puts the
//! result through the domain's REAL admissibility gate before returning it. It registers
//! nothing and writes no journal, so it is digest-invariant.
//!
//! ## The three gates a proposal passes, in order
//!
//! 1. **Decode** — `deny_unknown_fields`, with the credential/argv keys refused by name.
//! 2. **Authority** — the proposal is intersected against what the CALLER could do anyway.
//!    A proposal is not a grant, so a form naming a tool the caller cannot reach is refused
//!    here rather than previewed and refused later.
//! 3. **The domain's own enforcer** — `admit_script` / `admit_registration`, the SAME
//!    functions `RegisterScript` and `RegisterTool` run. Not a restatement of them.
//!
//! A failure at any gate is an honest [`ControlOutcome::Rejected`] with the reason, never a
//! transport error and never a panic. The whole point of a preview is that a refusal is an
//! ANSWER; a caller who asked for something inadmissible should learn that before a human is
//! asked to approve it.
//!
//! ## What it deliberately does not check
//!
//! The host half of admission — an interpreter probed inside the sandbox, an SSRF-vetted
//! egress host — is NOT run here. Those need a live sandbox and operator configuration, and
//! a preview that claimed them would be promising something only the real registration can
//! deliver. A proposal that passes here can still be refused at registration, and that is
//! correct.

use std::sync::Arc;

use kx_gateway_core::{
    ControlNlInput, ControlOutcome, ControlProposal, ControlProposer, RegisteredToolsView,
};
use kx_inference::{InferenceBackend, InferenceInput};
use kx_mote::{InferenceParams, ModelId};
use kx_warrant::ExecutorClass;

use crate::control_decode::decode_control;
use crate::model_exec::shaper_warrant;
use crate::prompt_library::{control_user_message, CONTROL_SYSTEM};
use crate::routing_backend::RoutingBackend;

/// The host proposer: the served-model backend plus the live registry view a proposal is
/// intersected against.
pub(crate) struct HostControlProposer {
    backend: Arc<RoutingBackend>,
    model_id: ModelId,
    exec_class: ExecutorClass,
    /// The broker-fireable view — the SAME truth `RunApp` intersects against at fire. A
    /// proposal naming a tool that is not fireable is refused rather than previewed.
    registered: Arc<dyn RegisteredToolsView>,
}

impl HostControlProposer {
    /// Wire the proposer for a served model.
    pub(crate) fn new(
        backend: Arc<RoutingBackend>,
        model_id: ModelId,
        exec_class: ExecutorClass,
        registered: Arc<dyn RegisteredToolsView>,
    ) -> Self {
        Self {
            backend,
            model_id,
            exec_class,
            registered,
        }
    }
}

#[tonic::async_trait]
impl ControlProposer for HostControlProposer {
    async fn propose(&self, _principal: &str, input: ControlNlInput) -> ControlOutcome {
        let backend = self.backend.clone();
        let model_id = self.model_id.clone();
        let exec_class = self.exec_class;
        let registered = self.registered.clone();
        // Model inference is BLOCKING — run the whole render→decode→admit off the async
        // worker, exactly as the planner and derive seams do.
        match tokio::task::spawn_blocking(move || {
            propose_blocking(
                backend.as_ref(),
                &model_id,
                exec_class,
                registered.as_ref(),
                &input,
            )
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => rejected(&format!("the control-proposal task failed: {e}")),
        }
    }
}

/// The synchronous render→decode→admit core (generic over the backend so a stub can drive
/// it). Validate-only; never mutates state.
fn propose_blocking<B: InferenceBackend>(
    backend: &B,
    model_id: &ModelId,
    exec_class: ExecutorClass,
    registered: &dyn RegisteredToolsView,
    input: &ControlNlInput,
) -> ControlOutcome {
    // A steer naming an unknown domain is refused rather than ignored: a silently-dropped
    // steer proposes into a different domain while looking like it obeyed.
    const DOMAINS: &[&str] = &[
        "workflows",
        "tools",
        "connectors",
        "secrets",
        "scripts",
        "triggers",
        "policy",
    ];
    let steer = input.domain.trim();
    if !steer.is_empty() && !DOMAINS.contains(&steer) {
        return rejected(&format!(
            "unknown domain {steer:?}: expected one of {}",
            DOMAINS.join(" | ")
        ));
    }

    let user = control_user_message(&input.goal, steer);
    let parent = shaper_warrant(model_id, exec_class);
    let Some(rendered) = backend.render_chat(model_id, CONTROL_SYSTEM, &user) else {
        return rejected(
            "the served model could not format the control prompt (start `kx serve` with an \
             inference or serve-engine build and a resolved model)",
        );
    };
    let params = InferenceParams {
        max_output_tokens: crate::env_caps::planner_max_output_tokens()
            .min(parent.model_route.max_output_tokens),
        ..InferenceParams::default()
    };
    let raw = match backend.dispatch(model_id, &InferenceInput::text(rendered), &params, &parent) {
        Ok(out) => String::from_utf8_lossy(&out.bytes).into_owned(),
        Err(e) => return rejected(&format!("the model could not produce a control form: {e}")),
    };

    // GATE 1 — decode, fail-closed.
    let proposal = match decode_control(raw.as_bytes()) {
        Ok(p) => p,
        Err(e) => return rejected(&e.to_string()),
    };

    // GATE 2 — authority. Naming is never granting: a role may only narrow to tools that
    // are actually fireable on this serve, so a form naming a tool nobody registered is a
    // form that would narrow to nothing without saying so.
    if let ControlProposal::PutPolicyRole(role) = &proposal {
        let fireable = registered.registered_grants();
        let unknown: Vec<String> = role
            .tools
            .iter()
            .filter(|t| {
                !fireable
                    .iter()
                    .any(|(id, ver)| *id == t.tool_id && *ver == t.tool_version)
            })
            .map(|t| format!("{}@{}", t.tool_id, t.tool_version))
            .collect();
        if !unknown.is_empty() {
            return rejected(&format!(
                "this role names {} that no registered tool matches — a role NARROWS to \
                 tools that exist, so naming one that does not would silently narrow to \
                 nothing: {unknown:?}",
                if unknown.len() == 1 {
                    "a tool"
                } else {
                    "tools"
                }
            ));
        }
    }

    // GATE 3 — the domain's OWN register-time enforcer. The same functions the real RPCs
    // run, not a restatement of their rules.
    if let Err(reason) = admit_through_the_real_enforcer(&proposal) {
        return rejected(&reason);
    }

    let summary = summarize(&proposal);
    ControlOutcome::Proposed {
        proposal: Box::new(proposal),
        summary,
    }
}

/// Run the proposal through the domain's real admission function.
///
/// Only the two domains whose enforcers were EXTRACTED are gated here. The others reach
/// their enforcement at the real RPC, and pretending otherwise — by restating their rules —
/// is the drift the extraction exists to prevent. When another prologue is extracted, it
/// joins this match; until then a proposal in that domain is previewed on decode + authority
/// alone, which is what the caller is told.
fn admit_through_the_real_enforcer(proposal: &ControlProposal) -> Result<(), String> {
    match proposal {
        ControlProposal::RegisterScript(s) => {
            let reg = s.clone().into_registration();
            let req = kx_proto::proto::RegisterScriptRequest {
                script_name: reg.script_name,
                script_version: reg.script_version,
                description: reg.description,
                interpreter: reg.interpreter,
                source: reg.source,
                argv: reg.argv,
                env: Vec::new(),
                fs_mounts: Vec::new(),
                net_hosts: reg.net_hosts,
                wall_clock_ms: reg.wall_clock_ms,
                mem_bytes: reg.mem_bytes,
                max_output_bytes: reg.max_output_bytes,
            };
            kx_gateway_core::admit_script_for_test(&req).map_err(|s| s.message().to_string())
        }
        ControlProposal::RegisterTool(t) => {
            let req = kx_proto::proto::RegisterToolRequest {
                tool_name: t.tool_name.clone(),
                tool_version: t.tool_version.clone(),
                description: t.description.clone(),
                idempotency_class: t.idempotency_class.clone(),
                input_schema: None,
                server_host: t.server_host.clone(),
                remote_name: t.remote_name.clone(),
            };
            kx_gateway_core::admit_registration_for_test(&req).map_err(|s| s.message().to_string())
        }
        _ => Ok(()),
    }
}

/// A one-line DISPLAY rendering. Never parsed — it exists so a terminal can show something
/// honest without re-implementing a renderer per domain.
fn summarize(p: &ControlProposal) -> String {
    match p {
        ControlProposal::SaveWorkflow(w) => format!("save workflow {}", w.handle),
        ControlProposal::RegisterTool(t) => {
            format!(
                "register tool {}@{} via {}",
                t.tool_name, t.tool_version, t.server_host
            )
        }
        ControlProposal::RegisterMcpServer(c) => {
            format!("register connector {} ({})", c.server_name, c.transport)
        }
        ControlProposal::RegisterTrigger(t) => {
            let target = [&t.recipe_handle, &t.app_handle, &t.workflow_handle]
                .iter()
                .find(|h| !h.is_empty())
                .map_or("(no target)", |h| h.as_str());
            format!("register {} trigger {} -> {target}", t.kind, t.name)
        }
        ControlProposal::PutPolicyRole(r) => format!(
            "define role {} narrowing to {} tool(s)",
            r.name,
            r.tools.len()
        ),
        ControlProposal::AssignPolicyRole { party, role } => {
            format!("assign role {role} to {party}")
        }
        // The NAME only. A summary that echoed a value would defeat the whole reduction.
        ControlProposal::PutSecret(s) => format!("declare secret NAME {}", s.name),
        ControlProposal::RegisterScript(s) => format!(
            "register script {}@{} ({})",
            s.script_name, s.script_version, s.interpreter
        ),
    }
}

fn rejected(reason: &str) -> ControlOutcome {
    ControlOutcome::Rejected {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{admit_through_the_real_enforcer, summarize};
    use kx_gateway_core::{ControlProposal, ScriptProposal, SecretProposal, ToolProposal};

    /// A summary must never echo anything a proposal was reduced to avoid carrying.
    #[test]
    fn a_secret_summary_names_only_the_name() {
        let s = summarize(&ControlProposal::PutSecret(SecretProposal {
            name: "OPENAI_API_KEY".into(),
            secret_scope: "app:reports".into(),
            net_scope: "egress:api.example.com:443".into(),
        }));
        assert!(s.contains("OPENAI_API_KEY"));
        assert!(s.contains("NAME"));
    }

    /// The proposal is judged by the SAME function the real RPC runs.
    #[test]
    fn an_inadmissible_script_is_refused_by_the_real_enforcer() {
        // Empty source is `admit_script`'s rule, not a rule restated here.
        let bad = ControlProposal::RegisterScript(ScriptProposal {
            script_name: "tidy".into(),
            script_version: "1".into(),
            interpreter: "sh".into(),
            source: Vec::new(),
            ..ScriptProposal::default()
        });
        let err = admit_through_the_real_enforcer(&bad).unwrap_err();
        assert!(err.contains("source"), "got {err:?}");

        // And a well-formed one passes, so the refusal above is not everything failing.
        let ok = ControlProposal::RegisterScript(ScriptProposal {
            script_name: "tidy".into(),
            script_version: "1".into(),
            interpreter: "sh".into(),
            source: b"echo hi".to_vec(),
            ..ScriptProposal::default()
        });
        assert!(admit_through_the_real_enforcer(&ok).is_ok());
    }

    #[test]
    fn an_inadmissible_tool_is_refused_by_the_real_enforcer() {
        let bad = ControlProposal::RegisterTool(ToolProposal {
            tool_name: "lookup".into(),
            tool_version: "1".into(),
            server_host: String::new(),
            ..ToolProposal::default()
        });
        let err = admit_through_the_real_enforcer(&bad).unwrap_err();
        assert!(err.contains("server_host"), "got {err:?}");
    }

    /// A domain whose enforcer has NOT been extracted is not silently "admitted" by a
    /// restatement — it simply is not gated here, and the module doc says so.
    #[test]
    fn an_ungated_domain_passes_this_stage_without_a_restated_rule() {
        let p = ControlProposal::AssignPolicyRole {
            party: "alice@acme".into(),
            role: "ops".into(),
        };
        assert!(admit_through_the_real_enforcer(&p).is_ok());
    }
}
