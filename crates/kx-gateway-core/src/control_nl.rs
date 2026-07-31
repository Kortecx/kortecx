//! The NL-AUTHORING seam (`ProposeControlAction`) — one sentence in, the exact
//! typed request the runtime WOULD issue out.
//!
//! `ProposeWorkflow` answers "what steps would achieve this goal?". `DeriveApp`
//! answers "what APP is this?". This seam answers the remaining question — "what
//! would you have me REGISTER?" — across every authoring domain: workflows,
//! tools, connectors, secrets, scripts, triggers and policy.
//!
//! # Approval is client-held, and that is the whole shape
//!
//! This seam VALIDATES ONLY. It writes nothing, journals nothing, and registers
//! nothing. What it returns is a [`ControlProposal`] that maps 1:1 onto a real
//! `KxGateway` request message, so approving is FORWARDING THE BYTES THAT WERE
//! DISPLAYED — never re-deriving them from a rendered summary. The mutation a
//! human approves is the ordinary domain RPC, issued by the client.
//!
//! D114's journal-staged approval gate covers in-run tool DISPATCHES and is
//! deliberately NOT extended here. Two approval concepts is exactly the drift
//! this design exists to avoid, and the existing one cannot carry these bytes
//! anyway: an `ApprovalState::Requested` holds a synthesized one-line intent
//! string, not a request body.
//!
//! # Two proposals are REDUCED, and the reduction is the security property
//!
//! [`SecretProposal`] carries no value, and [`ScriptProposal`] carries no `argv`
//! and no `env`. Neither omission is a policy that code must remember to apply —
//! the types cannot express those fields, so the request a client forwards
//! necessarily has them empty. A descriptor-walk test in `kx-proto` holds the
//! same line on the wire.
//!
//! # Admissibility is per SHAPE, two gates, neither duplicated
//!
//! Plan-shaped proposals (workflows) keep `compile_plan`, which already gates
//! both `ProposeWorkflow` and `DeriveApp`. Registration-shaped proposals decode
//! with `deny_unknown_fields`, intersect against the caller's authority, and
//! then face the domain's EXISTING register-time refusal contract — the same
//! `admit_*` functions the real RPC runs. A third definition of "admissible" is
//! how these paths would come to disagree.
//!
//! Like the other model-served seams, the host owns the runtime surface: the
//! concrete impl lives in `kx-gateway` behind `serve-engine`. gateway-core
//! defines only the seam and the proposal shapes. A `None` seam ⇒
//! `ProposeControlAction` returns `unimplemented`.

use crate::policy_admin::PolicyRoleRow;

/// What the author asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControlNlInput {
    /// The author's single natural-language ask. The whole input.
    pub goal: String,
    /// An OPTIONAL domain steer: `workflows` | `tools` | `connectors` | `secrets`
    /// | `scripts` | `triggers` | `policy`. Empty ⇒ the host classifies.
    ///
    /// A steer that names an unknown domain is REFUSED rather than ignored. A
    /// silently-ignored steer proposes into the wrong domain while looking like
    /// it obeyed, which is worse than saying no.
    pub domain: String,
}

/// A proposed tool registration. Mirrors `RegisterToolRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolProposal {
    /// Identity half — the grant-set key.
    pub tool_name: String,
    /// Identity half.
    pub tool_version: String,
    /// Free-form; NEVER parsed for enforcement.
    pub description: String,
    /// `"Token"` | `"Readback"` | `"Staged"` | `"AtLeastOnce"`.
    pub idempotency_class: String,
    /// `host[:port]` the gateway will dial; SSRF-vetted at the real register.
    pub server_host: String,
    /// The tool's name on the remote server; empty ⇒ `tool_name`.
    pub remote_name: String,
}

/// A proposed MCP connector registration. Mirrors `RegisterMcpServerRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectorProposal {
    /// Unique handle; namespaces discovered tool ids.
    pub server_name: String,
    /// `"stdio"` | `"http"`.
    pub transport: String,
    /// stdio: program path. http: URL.
    pub endpoint: String,
    /// stdio command-line args (ignored for http).
    pub args: Vec<String>,
    /// http: refuse plaintext when true.
    pub tls_required: bool,
    /// An OPTIONAL secret-less ref NAME. Never the secret — the field names a
    /// credential, it does not carry one, and that is true on the wire too.
    pub credential_ref: String,
    /// `"stateful"` | `"stateless"`; empty ⇒ stateless.
    pub session_mode: String,
}

/// A proposed trigger registration. Mirrors `RegisterTriggerRequest`.
///
/// The kind-aware refusals that already exist at registration (an unwired
/// workflow seam, a workflow the caller does not own, a DRAFT workflow) must
/// surface here as an NL REFUSAL. A proposal that would dead-letter at fire time
/// is not a proposal, it is a trap.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TriggerProposal {
    /// Unique operator handle.
    pub name: String,
    /// `"webhook"` | `"cron"` | `"grpc"`.
    pub kind: String,
    /// `"none"` | `"hmac_sha256"` | `"bearer"`.
    pub auth: String,
    /// The SecretRef NAME of the HMAC/bearer secret. Never the value.
    pub auth_secret_ref: String,
    /// cron: interval seconds or a 5-field crontab expression.
    pub schedule_spec: String,
    /// IANA zone for a 5-field cron expression; empty ⇒ UTC.
    pub timezone: String,
    /// Whether the trigger starts enabled.
    pub enabled: bool,
    /// Per-trigger HITL (D114).
    pub require_approval: bool,
    /// EXACTLY ONE of these three is set; the host refuses otherwise.
    pub recipe_handle: String,
    /// See `recipe_handle`.
    pub app_handle: String,
    /// See `recipe_handle`.
    pub workflow_handle: String,
}

/// A proposed secret. **NAMES AND SCOPES ONLY — there is no value field, and
/// there is deliberately no way to add one here.**
///
/// D81 makes a secret write loopback-only and write-only; this type makes the
/// PROPOSAL side structurally incapable of carrying a credential, so a proposal
/// can be logged, rendered, replayed and stored without ever holding one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SecretProposal {
    /// The SecretRef NAME the caller would set.
    pub name: String,
    /// The scope the secret would be readable under (display).
    pub secret_scope: String,
    /// The egress scope it would authorize (display).
    pub net_scope: String,
}

/// A proposed script registration.
///
/// **`argv` and `env` are absent BY CONSTRUCTION.** They are fixed at
/// registration by an operator and are never model-controlled, so a proposal
/// type that cannot express them is a proposal that cannot smuggle them. See
/// [`ScriptProposal::into_registration`], which is total and can only ever
/// produce empty `argv` / `env`.
///
/// The interpreter is still validated against the host's CLOSED allowlist, and
/// still PROBED inside the sandbox, at the real `RegisterScript`. Proposing one
/// grants nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptProposal {
    /// Identity half.
    pub script_name: String,
    /// Identity half.
    pub script_version: String,
    /// Free-form; NEVER parsed for enforcement.
    pub description: String,
    /// The interpreter token.
    pub interpreter: String,
    /// The script's source bytes.
    pub source: Vec<u8>,
    /// The filesystem it DECLARES it needs.
    pub fs_mounts: Vec<crate::script_admin::ScriptMountWire>,
    /// The hosts it DECLARES it needs; empty ⇒ no egress.
    pub net_hosts: Vec<String>,
    /// Wall-clock budget in ms (0 ⇒ host default).
    pub wall_clock_ms: u64,
    /// Memory ceiling in bytes (0 ⇒ unset).
    pub mem_bytes: u64,
    /// Output ceiling in bytes (0 ⇒ host default).
    pub max_output_bytes: u64,
}

impl ScriptProposal {
    /// Lower a proposal into the real [`crate::script_admin::ScriptRegistration`].
    ///
    /// Total, and deliberately so: there is no argument by which `argv` or `env`
    /// could become non-empty, because the input type has no such field. This is
    /// the function that makes "script argv/env are proposed EMPTY" a property of
    /// the types rather than a rule someone has to keep applying.
    #[must_use]
    pub fn into_registration(self) -> crate::script_admin::ScriptRegistration {
        crate::script_admin::ScriptRegistration {
            script_name: self.script_name,
            script_version: self.script_version,
            description: self.description,
            interpreter: self.interpreter,
            source: self.source,
            // Not a default that could be overridden later — the only value
            // reachable from a proposal.
            argv: Vec::new(),
            env: Vec::new(),
            fs_mounts: self.fs_mounts,
            net_hosts: self.net_hosts,
            wall_clock_ms: self.wall_clock_ms,
            mem_bytes: self.mem_bytes,
            max_output_bytes: self.max_output_bytes,
        }
    }
}

/// A proposed workflow save. Mirrors `SaveWorkflowRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowSaveProposal {
    /// Catalog key within the caller's scope.
    pub handle: String,
    /// The `kortecx.workflow/v1` JSON, already through `compile_plan`.
    pub envelope_json: Vec<u8>,
    /// `""` (active) or `"draft"`.
    pub lifecycle: String,
}

/// The exact typed request the runtime WOULD issue, one variant per authoring
/// domain. Each maps 1:1 onto a real `KxGateway` request message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlProposal {
    /// → `SaveWorkflow`.
    SaveWorkflow(WorkflowSaveProposal),
    /// → `RegisterTool`.
    RegisterTool(ToolProposal),
    /// → `RegisterMcpServer`.
    RegisterMcpServer(ConnectorProposal),
    /// → `RegisterTrigger`.
    RegisterTrigger(TriggerProposal),
    /// → `PutPolicyRole`.
    PutPolicyRole(PolicyRoleRow),
    /// → `AssignPolicyRole`. `role` empty ⇒ unassign.
    AssignPolicyRole {
        /// The PartyId the role would apply to.
        party: String,
        /// The role name; empty ⇒ unassign.
        role: String,
    },
    /// → `PutSecret`, with the value supplied by the human out of band.
    PutSecret(SecretProposal),
    /// → `RegisterScript`, via [`ScriptProposal::into_registration`].
    RegisterScript(ScriptProposal),
}

impl ControlProposal {
    /// The `GatewayRpc` wire name a client should call to enact this proposal.
    ///
    /// Returned on the wire so a client never has to map variant → RPC itself.
    /// Two clients deriving that mapping independently is two chances to send an
    /// approved body to the wrong verb.
    #[must_use]
    pub const fn rpc_name(&self) -> &'static str {
        match self {
            Self::SaveWorkflow(_) => "SaveWorkflow",
            Self::RegisterTool(_) => "RegisterTool",
            Self::RegisterMcpServer(_) => "RegisterMcpServer",
            Self::RegisterTrigger(_) => "RegisterTrigger",
            Self::PutPolicyRole(_) => "PutPolicyRole",
            Self::AssignPolicyRole { .. } => "AssignPolicyRole",
            Self::PutSecret(_) => "PutSecret",
            Self::RegisterScript(_) => "RegisterScript",
        }
    }

    /// The authoring domain token this proposal belongs to.
    #[must_use]
    pub const fn domain(&self) -> &'static str {
        match self {
            Self::SaveWorkflow(_) => "workflows",
            Self::RegisterTool(_) => "tools",
            Self::RegisterMcpServer(_) => "connectors",
            Self::RegisterTrigger(_) => "triggers",
            Self::PutPolicyRole(_) | Self::AssignPolicyRole { .. } => "policy",
            Self::PutSecret(_) => "secrets",
            Self::RegisterScript(_) => "scripts",
        }
    }
}

/// The outcome of an NL control proposal.
///
/// NEVER a transport error: no served model, an unknown domain steer, an
/// authority the caller does not hold, or a registration the domain's own
/// enforcer refuses all come back as [`ControlOutcome::Rejected`] with a reason.
/// A refusal is an answer, and the same posture `ProposeWorkflow` and
/// `DeriveApp` already take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The proposal, plus a one-line DISPLAY rendering. The summary is never
    /// parsed — it exists so a terminal can show something honest without
    /// re-implementing a renderer per domain.
    Proposed {
        /// The typed request that would be issued.
        proposal: Box<ControlProposal>,
        /// Display only.
        summary: String,
    },
    /// Why nothing is being proposed.
    Rejected {
        /// Operator-facing reason.
        reason: String,
    },
}

/// The NL-authoring seam. A `None` seam ⇒ `ProposeControlAction` returns
/// `unimplemented`.
#[tonic::async_trait]
pub trait ControlProposer: Send + Sync {
    /// Turn one natural-language goal into a typed, admissible proposal for
    /// `principal`. Writes nothing.
    async fn propose(&self, principal: &str, input: ControlNlInput) -> ControlOutcome;
}

#[cfg(test)]
mod tests {
    use super::{ControlProposal, ScriptProposal, SecretProposal};

    /// The reduction is structural, so the lowering cannot produce argv/env.
    ///
    /// This is not a test that the code remembered to clear two fields — it is a
    /// test that there was nothing to clear. If `ScriptProposal` ever grows an
    /// `argv` or `env` field, this stops compiling before it stops passing.
    #[test]
    fn a_proposed_script_lowers_with_empty_argv_and_env() {
        let reg = ScriptProposal {
            script_name: "tidy".into(),
            script_version: "1".into(),
            interpreter: "sh".into(),
            source: b"echo hi".to_vec(),
            net_hosts: vec!["example.com".into()],
            ..ScriptProposal::default()
        }
        .into_registration();

        assert!(reg.argv.is_empty(), "a proposal cannot express argv");
        assert!(reg.env.is_empty(), "a proposal cannot express env");
        // The fields it CAN express survive — otherwise the emptiness above
        // would be satisfied by a lowering that dropped everything.
        assert_eq!(reg.script_name, "tidy");
        assert_eq!(reg.source, b"echo hi");
        assert_eq!(reg.net_hosts, vec!["example.com".to_string()]);
    }

    /// A secret proposal has no value to leak, by type.
    #[test]
    fn a_secret_proposal_carries_only_a_name_and_scopes() {
        let p = SecretProposal {
            name: "OPENAI_API_KEY".into(),
            secret_scope: "app:reports".into(),
            net_scope: "egress:api.example.com:443".into(),
        };
        let rendered = format!("{p:?}");
        assert!(rendered.contains("OPENAI_API_KEY"));
        // The Debug rendering is the whole type. If a value field is ever added,
        // this stops being a meaningful assertion — which is why the descriptor
        // walk in kx-proto is the load-bearing half, and this is the local one.
        assert_eq!(
            std::mem::size_of::<SecretProposal>(),
            std::mem::size_of::<String>() * 3,
            "SecretProposal is exactly three strings — no fourth field slipped in"
        );
    }

    /// Every variant names an RPC and a domain, and the two agree.
    #[test]
    fn every_proposal_names_its_rpc_and_domain() {
        let cases = [
            (
                ControlProposal::RegisterScript(ScriptProposal::default()),
                "RegisterScript",
                "scripts",
            ),
            (
                ControlProposal::PutSecret(SecretProposal::default()),
                "PutSecret",
                "secrets",
            ),
            (
                ControlProposal::AssignPolicyRole {
                    party: "p".into(),
                    role: "r".into(),
                },
                "AssignPolicyRole",
                "policy",
            ),
        ];
        for (proposal, rpc, domain) in cases {
            assert_eq!(proposal.rpc_name(), rpc);
            assert_eq!(proposal.domain(), domain);
        }
    }
}
