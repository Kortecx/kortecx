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
    // An engine that supplies no chat template is NOT an engine that cannot answer:
    // `render_chat` is optional by contract and `None` means "format it yourself".
    // Refusing here made the entire authoring surface unreachable on Ollama.
    let rendered =
        crate::model_exec::render_chat_or_chatml(backend, model_id, CONTROL_SYSTEM, &user);
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

    // GATE 2 — authority.
    if let Err(reason) = admit_role_names_only_fireable_tools(&proposal, registered) {
        return rejected(&reason);
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

/// GATE 2 — authority. Naming is never granting: a role may only narrow to tools that are
/// actually fireable on this serve, so a form naming a tool nobody registered is a form that
/// would narrow to nothing without saying so.
///
/// Extracted from `propose_blocking` so it can be asserted WITHOUT a model. Inline, the only
/// thing exercising it was the live `nlauthor` refusal task — and that task expects
/// `terminal: rejected`, which any refusal satisfies. A malformed generation refused by GATE 1
/// scored it green while this gate never ran, so the security-critical gate was the one with
/// no test. A refusal oracle has to assert the REASON, and it can only do that where the
/// reason is reachable.
///
/// Behaviour-preserving: same predicate, same message, same singular/plural.
fn admit_role_names_only_fireable_tools(
    proposal: &ControlProposal,
    registered: &dyn RegisteredToolsView,
) -> Result<(), String> {
    let ControlProposal::PutPolicyRole(role) = proposal else {
        return Ok(());
    };
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
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "this role names {} that no registered tool matches — a role NARROWS to \
         tools that exist, so naming one that does not would silently narrow to \
         nothing: {unknown:?}",
        if unknown.len() == 1 {
            "a tool"
        } else {
            "tools"
        }
    ))
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
    use super::{
        admit_role_names_only_fireable_tools, admit_through_the_real_enforcer, propose_blocking,
        summarize,
    };
    use kx_gateway_core::{
        ControlNlInput, ControlOutcome, ControlProposal, PolicyRoleRow, PolicyRoleToolWire,
        RegisteredToolsView, ScriptProposal, SecretProposal, ToolProposal,
    };
    use kx_inference::{InferenceBackend, InferenceError, InferenceInput, InferenceOutput};
    use kx_mote::{InferenceParams, ModelId};
    use kx_warrant::{ExecutorClass, WarrantSpec};
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::time::Duration;

    /// A serve on which exactly `retrieve@1` is fireable.
    struct OneFireableTool;
    impl RegisteredToolsView for OneFireableTool {
        fn registered_grants(&self) -> BTreeSet<(String, String)> {
            [("retrieve".to_string(), "1".to_string())]
                .into_iter()
                .collect()
        }
    }

    fn model() -> ModelId {
        ModelId("test-model".into())
    }

    /// A control form the REAL decoder accepts and every gate admits on
    /// [`OneFireableTool`] — byte-identical in shape to the frozen golden in
    /// `tests/nlauthor_bench_drive.rs`, which is output the served Gemma actually produced.
    const GOLDEN_POLICY: &str = r#"{"control":{"domain":"policy","name":"reporting-only","fields":{"description":"narrows to the retrieve tool version 1","tools":["retrieve@1"]}}}"#;

    /// A backend implementing ONLY the trait's required methods, so `render_chat` is the
    /// trait DEFAULT. It pins the premise the guard below rests on: an engine may
    /// legitimately not implement the method, and `kx_ollama::OllamaBackend` does not.
    struct DefaultRenderBackend;

    impl InferenceBackend for DefaultRenderBackend {
        fn dispatch(
            &self,
            model_id: &ModelId,
            _input: &InferenceInput,
            _params: &InferenceParams,
            _warrant: &WarrantSpec,
        ) -> Result<InferenceOutput, InferenceError> {
            Ok(InferenceOutput {
                bytes: Vec::new(),
                output_tokens: 0,
                backend_name: "test",
                model_id: model_id.clone(),
                elapsed: Duration::from_millis(1),
            })
        }
        fn supports(&self, _model_id: &ModelId) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "test"
        }
    }

    /// The A/B backend. `renders` is the ONE variable: `true` applies a chat template
    /// (llama.cpp's shape, via `LlamaInferenceBackend::render_chat`), `false` yields the
    /// trait default `None` (Ollama's shape — it never implements the method). Both arms
    /// return the SAME body, so any difference in outcome is attributable to templating
    /// alone. `reached` records whether the model was called at all.
    struct StubBackend {
        renders: bool,
        reached: Mutex<bool>,
    }

    impl StubBackend {
        fn new(renders: bool) -> Self {
            Self {
                renders,
                reached: Mutex::new(false),
            }
        }
        fn reached(&self) -> bool {
            *self.reached.lock().expect("reached flag")
        }
    }

    impl InferenceBackend for StubBackend {
        fn dispatch(
            &self,
            model_id: &ModelId,
            _input: &InferenceInput,
            _params: &InferenceParams,
            _warrant: &WarrantSpec,
        ) -> Result<InferenceOutput, InferenceError> {
            *self.reached.lock().expect("reached flag") = true;
            Ok(InferenceOutput {
                bytes: GOLDEN_POLICY.as_bytes().to_vec(),
                output_tokens: 1,
                backend_name: "test",
                model_id: model_id.clone(),
                elapsed: Duration::from_millis(1),
            })
        }
        fn render_chat(&self, _model_id: &ModelId, system: &str, user: &str) -> Option<String> {
            self.renders.then(|| format!("{system}\n{user}"))
        }
        fn supports(&self, _model_id: &ModelId) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "test"
        }
    }

    /// The premise, asserted rather than assumed: `render_chat` is OPTIONAL, and a backend
    /// that does not implement it returns `None`. If this ever stops being true the guard
    /// below is testing a condition that can no longer arise, and should be re-read.
    #[test]
    fn render_chat_is_optional_and_defaults_to_none() {
        assert!(
            DefaultRenderBackend
                .render_chat(&model(), "system", "user")
                .is_none(),
            "the trait default must be None — the authoring paths' fallback exists for it"
        );
    }

    /// **The incident.** On a serve whose engine does not implement `render_chat`, ALL FIVE
    /// `nlauthor` tasks came back `Rejected` with "the served model could not format the
    /// control prompt" — the model was never called. Measured on `gemma3:12b` via Ollama at
    /// suite digest `69a89582cbdd9854`: 0 of 5 accepting tasks reached the model, while
    /// `echo-roundtrip` scored 1000 in the same run, so the serve and the tool loop were
    /// healthy and only this path refused.
    ///
    /// The trait documents `None` as *"the caller should format the prompt itself (e.g.
    /// hand-rolled ChatML)"*. The ReAct turn does exactly that (`chatml_with`); this path
    /// treated the same `None` as fatal. Same value, opposite treatment.
    ///
    /// Both arms must REACH the model — the only difference between them is where the chat
    /// template comes from, which is presentation, not admissibility. Every other stub in
    /// this workspace implements `render_chat`, which is why no existing test could fail
    /// for this reason.
    #[test]
    fn the_model_is_reached_whether_or_not_the_engine_renders_the_chat_template() {
        for renders in [true, false] {
            let backend = StubBackend::new(renders);
            let outcome = propose_blocking(
                &backend,
                &model(),
                ExecutorClass::Bwrap,
                &OneFireableTool,
                &ControlNlInput {
                    goal: "Create a durable role named reporting-only that narrows tool \
                           authority to just the retrieve tool at version 1."
                        .into(),
                    domain: String::new(),
                },
            );
            assert!(
                backend.reached(),
                "renders={renders}: the model was never dispatched — the path refused before \
                 generation, so no grammar, prompt or contract change could ever affect it"
            );
            match outcome {
                ControlOutcome::Proposed { .. } => {}
                ControlOutcome::Rejected { reason } => {
                    panic!("renders={renders}: expected a proposal, got Rejected: {reason}")
                }
            }
        }
    }

    /// Build a role narrowing to exactly one `(id, version)` pair.
    fn role_naming(tool_id: &str, tool_version: &str) -> ControlProposal {
        ControlProposal::PutPolicyRole(PolicyRoleRow {
            name: "reporting-only".into(),
            description: String::new(),
            tools: vec![PolicyRoleToolWire {
                tool_id: tool_id.into(),
                tool_version: tool_version.into(),
            }],
            created_unix_ms: 0,
            updated_unix_ms: 0,
        })
    }

    /// The authority A/B: two roles identical in EVERY field but the tool id.
    ///
    /// The bench family's refusal task cannot make this claim. It expects
    /// `terminal: rejected`, which a malformed generation refused by GATE 1 also satisfies —
    /// so it can go green with this gate never running. Varying exactly one variable is what
    /// makes the refusal attributable, and the accepting arm is what stops a
    /// refuse-everything implementation from scoring perfectly.
    #[test]
    fn a_role_naming_an_unregistered_tool_is_refused_and_one_naming_a_registered_tool_is_not() {
        let ok =
            admit_role_names_only_fireable_tools(&role_naming("retrieve", "1"), &OneFireableTool);
        assert!(
            ok.is_ok(),
            "a role narrowing to a REGISTERED tool must pass the authority gate: {ok:?}"
        );

        let err = admit_role_names_only_fireable_tools(
            &role_naming("definitely-not-registered", "1"),
            &OneFireableTool,
        )
        .expect_err("a role naming a tool nobody registered must be refused");

        // Assert the REASON, not merely that it refused: a refusal for any other cause
        // means this boundary was never exercised.
        assert!(
            err.contains("definitely-not-registered@1"),
            "the refusal must name the offending pair, got: {err}"
        );
        assert!(
            err.contains("NARROWS"),
            "the refusal must give the narrowing rationale, got: {err}"
        );
    }

    /// The grant-set key is the PAIR. A role naming a registered id at an unregistered
    /// version is refused — otherwise a role could reach a version nobody registered by
    /// borrowing a known id.
    #[test]
    fn the_version_half_of_the_pair_is_load_bearing() {
        let err =
            admit_role_names_only_fireable_tools(&role_naming("retrieve", "9"), &OneFireableTool)
                .expect_err("retrieve@9 is not registered even though retrieve@1 is");
        assert!(err.contains("retrieve@9"), "got: {err}");
    }

    /// The gate is SCOPED to policy roles. Every other proposal shape passes it untouched
    /// and meets its own enforcement later; widening it here would be a second, divergent
    /// definition of admissibility.
    #[test]
    fn a_non_policy_proposal_passes_the_authority_gate_untouched() {
        let tool = ControlProposal::RegisterTool(ToolProposal {
            tool_name: "retrieve".into(),
            tool_version: "1".into(),
            idempotency_class: "Token".into(),
            server_host: "api.example.com".into(),
            ..ToolProposal::default()
        });
        assert!(admit_role_names_only_fireable_tools(&tool, &OneFireableTool).is_ok());
    }

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
