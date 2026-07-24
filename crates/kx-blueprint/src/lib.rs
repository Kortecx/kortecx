// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The author-side Tier-1 **blueprint** shape and the ONE canonical lowering to a
//! `proto::SubmitWorkflowRequest`.
//!
//! A blueprint is a vetted palette of PURE / MODEL / TOOL steps + DATA/CONTROL
//! edges (`exec` is reserved). The client sends TOPOLOGY + PARAMS only; the server
//! compiles the DAG, derives all identity, and builds every warrant from the
//! party's grants (SN-8). [`to_request`] is the single canonical assembly every
//! caller funnels through, so a blueprint lowers to a **byte-identical**
//! `SubmitWorkflowRequest` whether it comes from `kx blueprint run --file`, the
//! `kx chain` string DSL, `kx app run`, or the gateway's server-side App-pointer
//! run resolution (G2). Extracted from `kx-cli` (was `verbs::blueprint`) into this
//! FFI-free leaf so the gateway host can lower a stored App's blueprint without a
//! `kx-cli` dependency, guaranteeing that server-authored App runs and
//! client-authored workflows produce identical wire bytes (the digest no-op proof).
//!
//! The `<dag.json>` shape:
//! ```json
//! {
//!   "seed": 7,
//!   "steps": [
//!     { "kind": "pure", "params": { "topic": "hello" } },
//!     { "kind": "pure" }
//!   ],
//!   "edges": [ { "parent": 0, "child": 1, "edge": "data" } ],
//!   "execution_mode": "frozen",
//!   "context_bundles": [ "team/ctx/spec" ]
//! }
//! ```
//! `params` values are UTF-8 strings (their bytes land in the step's config).
//! `kind` ∈ {`pure`, `model`, `tool`} (`exec` is reserved) and is OPTIONAL: omit it
//! and the kind is inferred from field presence (`model_id`/`prompt` ⇒ `model`, a
//! `tool_contract` with no model fields ⇒ `tool`, else ⇒ `pure`); an explicit kind
//! must agree with the fields (fail-closed). `edge` ∈ {`data`, `control`}.
//!
//! ## The App-only per-step fields
//!
//! A step may also carry `skills` / `connections` / `datasets` — NAMES that bind an App
//! envelope's declared capabilities to THAT node, so an App's knowledge and reach are
//! properties of the step that needs them rather than of the whole App:
//! ```json
//! { "prompt": "Collect this week's escalations",
//!   "tool_contract": { "retrieve": "1" },
//!   "skills": ["triage"], "connections": ["kx-connector-gmail"], "datasets": ["support"] }
//! ```
//! They are resolved by `RunApp` against the envelope's `references`, and they are
//! **App-only**: [`to_request`] refuses them, because a `SubmitWorkflowRequest` has no
//! `references` rail to name into. Each is omitted from the emitted JSON when empty, so a
//! blueprint that binds nothing is byte-identical to one written before these existed —
//! which is what keeps every already-authored App's `MoteId`s unchanged.

use std::collections::BTreeMap;

use kx_proto::proto;
use serde::{Deserialize, Serialize};

/// A blueprint lowering / validation failure. Carries a human message; the caller
/// (`kx-cli`) maps it to its `CliError::Usage`, the gateway host to
/// `Status::invalid_argument`. Kept as one actionable variant — every failure here
/// is a client-side authoring error (bad kind / edge / conflicting fields / bad
/// hex), never an internal fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintError(pub String);

impl std::fmt::Display for BlueprintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BlueprintError {}

impl BlueprintError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// PR-6b-2: the single canonical config key a `tool` step's authored args ride
/// under. MUST equal `kx_mote::TOOL_ARGS_KEY` + the Py/TS `TOOL_ARGS_KEY` (pinned
/// identical by the golden corpus). Hardcoded to avoid a `kx-mote` dep on this leaf.
pub const TOOL_ARGS_KEY: &str = "kx.tool.args";

/// PR-9b (D161.1): the canonical config keys a deterministic-agentic MODEL step's
/// bounded-loop budget rides under (decimal-string bytes ⇒ canonical-JSON `u32`,
/// the form the coordinator's `react_seed_params` reads). MUST equal
/// `kx_mote::REACT_MAX_TURNS_KEY` / `REACT_MAX_TOOL_CALLS_KEY` (pinned by the
/// golden corpus). Hardcoded to avoid a `kx-mote` dep on this leaf.
pub const REACT_MAX_TURNS_KEY: &str = "max_turns";
pub const REACT_MAX_TOOL_CALLS_KEY: &str = "max_tool_calls";

/// The author-side DAG shape parsed from a blueprint JSON.
///
/// `Serialize` (Batch B / D161.2): a parsed/lowered `DagSpec` re-serializes to a
/// portable blueprint JSON (`kx chain run --emit-blueprint`). The `skip_serializing_if`
/// guards keep the artifact clean — each skipped field's `#[serde(default)]` exactly
/// reproduces the omitted value on re-read, so export→import is byte-stable. All
/// fields are `pub` so the `kx-cli` `chain` DSL and the gateway host can construct /
/// mutate the spec across the crate boundary.
#[derive(Debug, Deserialize, Serialize)]
pub struct DagSpec {
    #[serde(default)]
    pub seed: u32,
    pub steps: Vec<StepSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    /// PR-7: context-bundle handles to attach to the run (chain-level grounding the
    /// SERVER resolves + injects into every entry Mote at bind, SN-8). Verbatim
    /// order; empty ⇒ byte-identical to pre-PR-7.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_bundles: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StepSpec {
    /// `pure` | `model` | `tool` (PR-6b-2); `exec` is reserved (rejected client-side).
    /// OPTIONAL (Batch A authoring veneer): when omitted the kind is INFERRED from
    /// field presence — a non-empty `model_id`/`prompt` ⇒ `model` (a `model` step that
    /// also carries a `tool_contract` is the deterministic-agentic step — still
    /// `model`), a non-empty `tool_contract` with no model fields ⇒ `tool`, else ⇒
    /// `pure`. An explicit kind is an override that MUST agree with the present fields
    /// (fail-closed on conflict, e.g. `kind:"pure"` + a `model_id`). The SDK factories
    /// (`pure()`/`model()`/`tool()`) always set it; this only eases the JSON surface.
    /// Export (Batch B) sets it EXPLICITLY (the self-describing portable form); omitted
    /// ⇒ inferred on re-read, so the round-trip is stable either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    /// EXEC only: the registered body's content/signature id as 64-char hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_signature_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_contract: BTreeMap<String, String>,
    /// Free config entries; values are UTF-8 strings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// TOOL only (PR-6b-2): the tool-call arguments, serialized at lowering to ONE
    /// canonical-JSON object under [`TOOL_ARGS_KEY`] (sorted keys, compact) —
    /// byte-identical to the Py/TS `tool()` factories. No floats (SN-8).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, serde_json::Value>,
    /// Agentic MODEL step only (PR-9b, D161.1): the bounded reason→tool→observe
    /// loop budget. Lowered to canonical-JSON `u32` bytes under
    /// [`REACT_MAX_TURNS_KEY`] / [`REACT_MAX_TOOL_CALLS_KEY`] in `params` when the
    /// step is a MODEL step with a non-empty `tool_contract`; ignored otherwise.
    /// Absent ⇒ the coordinator default (8 turns / 6 tool calls).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    /// APP ONLY: the catalog SKILL names bound to THIS step, naming entries in the App
    /// envelope's `references.skills[].name`.
    ///
    /// A name, not a descriptor: `references.*` stays the DECLARATION (the CAS
    /// `instructions_ref`, the tool wish set — and what `GetAppManifest` reports) and the
    /// step carries only the BINDING, so two steps sharing a skill duplicate nothing and a
    /// reorder cannot misbind. `RunApp` resolves it; a name no step mentions binds to the
    /// entry agentic step, which is the pre-existing App-wide behaviour and is why an
    /// envelope authored before per-step binding lowers byte-identically.
    ///
    /// **Not a workflow concept.** `SubmitWorkflow` has no `references` to name into, so
    /// [`to_request`] REFUSES a non-empty list rather than dropping it silently.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// APP ONLY: the connection DESCRIPTORS bound to this step, naming entries in
    /// `references.connections[].descriptor`. Same posture as [`Self::skills`]; the run's
    /// per-step secret scope is derived from what these connections provide, bounded by the
    /// App-level `guards.secret_scope`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<String>,
    /// APP ONLY: the DATASET names this step grounds over, naming entries in
    /// `references.datasets[].dataset_ref` (or `steering_config.context.dataset_refs`).
    /// Same posture as [`Self::skills`]: the bound step gets `retrieve@1` + the grounding
    /// steer, instead of the entry step getting them on the whole App's behalf.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datasets: Vec<String>,
    /// APP ONLY: the other APPS this step calls, naming entries in
    /// `references.apps[].handle`. Same declaration/binding split as [`Self::skills`].
    ///
    /// **This one changes the graph.** The other three axes give a step more to work with;
    /// this one lowers the named App's OWN blueprint into the run and makes its terminal a
    /// parent of this step — so the step reads that App's output exactly as it reads any
    /// other parent's. That is what "an App is a capability, not just a job" means here.
    ///
    /// Unlike the other three there is NO legacy fallback site: a declared App that no step
    /// names is simply not called. There is nothing to be backward-compatible with (no App
    /// composed before this existed), and an App-wide default would silently run someone's
    /// workflow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<String>,
}

/// Refuse a step carrying an App-envelope capability binding on the WORKFLOW lowering path.
///
/// A `SubmitWorkflowRequest` has no `references` rail, so there is nothing for a
/// `skills`/`connections`/`datasets`/`apps` NAME to resolve against — the runtime could only
/// drop it. Dropping it would hand the author a workflow that silently lacks the knowledge and
/// reach they wrote down, so this fails at authoring with a message that says where the
/// field IS honoured. Same posture as the reserved `exec` kind in
/// [`StepSpec::resolve_kind`]: fail with a clear message rather than a server round-trip.
///
/// `apps` is the one whose silent drop would be worst: the workflow would lose an entire
/// SUB-GRAPH rather than some context, and it would still run — just without the work.
fn refuse_app_only_bindings(index: usize, s: &StepSpec) -> Result<(), BlueprintError> {
    if !s.has_app_bindings() {
        return Ok(());
    }
    let named: Vec<&str> = [
        (!s.skills.is_empty()).then_some("skills"),
        (!s.connections.is_empty()).then_some("connections"),
        (!s.datasets.is_empty()).then_some("datasets"),
        (!s.apps.is_empty()).then_some("apps"),
    ]
    .into_iter()
    .flatten()
    .collect();
    Err(BlueprintError::new(format!(
        "step {index} declares {} — a per-step capability list is an App-envelope BINDING \
         that names an entry in the App's `references`, and `RunApp` is what resolves it. A \
         workflow has no references to name into: author this as an App (kx app new / the SDK \
         `app(...)`), or grant the step a tool directly via `tool_contract`",
        named.join(" + "),
    )))
}

impl StepSpec {
    /// True when this step carries an App-envelope capability BINDING
    /// ([`Self::skills`] / [`Self::connections`] / [`Self::datasets`] / [`Self::apps`]).
    ///
    /// `RunApp` takes these off the spec before lowering, so by the time a blueprint
    /// reaches [`to_request`] on the App path they are always empty — which is exactly what
    /// keeps the lowering (and therefore every `MoteId`) byte-identical to the pre-binding
    /// form. Reaching `to_request` with one still set means the blueprint came from a
    /// workflow path that has no App to resolve it against.
    #[must_use]
    pub fn has_app_bindings(&self) -> bool {
        !self.skills.is_empty()
            || !self.connections.is_empty()
            || !self.datasets.is_empty()
            || !self.apps.is_empty()
    }

    /// Resolve the step's wire kind (Batch A authoring veneer). When `kind` is omitted
    /// it is INFERRED from field presence; when present it is an override that must
    /// AGREE with the fields (fail-closed). `exec` is rejected client-side (the binder
    /// reserves it — fail at authoring with a clear message rather than a server
    /// round-trip). Pure derivation of `&self` — `to_request` and the chain `@`-grant
    /// check both call it, and it is idempotent under grant injection (model fields are
    /// checked before `tool_contract`, so injecting tags never re-classifies a step).
    ///
    /// # Errors
    /// [`BlueprintError`] on a reserved/unknown kind or a kind that conflicts with the
    /// present fields.
    pub fn resolve_kind(&self) -> Result<proto::WorkflowStepKind, BlueprintError> {
        let has_model = !self.model_id.is_empty() || !self.prompt.is_empty();
        let has_tool = !self.tool_contract.is_empty();
        let has_args = !self.args.is_empty();
        // Inference (kind omitted): model fields win (an agentic model step carries a
        // tool_contract too — it is STILL a model step), then a tool contract, else pure.
        let inferred = if has_model {
            proto::WorkflowStepKind::Model
        } else if has_tool {
            proto::WorkflowStepKind::Tool
        } else {
            proto::WorkflowStepKind::Pure
        };
        let Some(explicit) = self.kind.as_deref() else {
            return Ok(inferred);
        };
        let kind = match explicit {
            "pure" => proto::WorkflowStepKind::Pure,
            "model" => proto::WorkflowStepKind::Model,
            "tool" => proto::WorkflowStepKind::Tool,
            "exec" => {
                return Err(BlueprintError::new(
                    "step kind `exec` is reserved (a registered body is not yet runnable); \
                     use pure|model|tool",
                ));
            }
            other => {
                return Err(BlueprintError::new(format!(
                    "step kind must be pure|model|tool, got {other:?}"
                )));
            }
        };
        // Agreement: an explicit kind must not CONTRADICT the present fields (so a typo
        // like `kind:"pure"` next to a `model_id` fails loudly instead of silently
        // dropping the model identity).
        let conflict = match kind {
            proto::WorkflowStepKind::Pure if has_model || has_tool || has_args => Some(
                "a `pure` step carries only params (no model_id / prompt / tool_contract / args)",
            ),
            proto::WorkflowStepKind::Model if has_args => Some(
                "`args` are tool-only; a `model` step uses `prompt` + an optional `tool_contract`",
            ),
            proto::WorkflowStepKind::Tool if has_model => {
                Some("a `tool` step has no model_id / prompt; name the tool in `tool_contract`")
            }
            _ => None,
        };
        if let Some(why) = conflict {
            return Err(BlueprintError::new(format!(
                "step kind {explicit:?} conflicts with its fields ({why})"
            )));
        }
        Ok(kind)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EdgeSpec {
    pub parent: u32,
    pub child: u32,
    /// `data` (default) | `control`. Omitted on export when it is the `data` default.
    #[serde(default = "default_edge", skip_serializing_if = "is_default_edge")]
    pub edge: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub non_cascade: bool,
}

fn default_edge() -> String {
    "data".to_string()
}

/// Export guard: a `data` edge is the default, so omit it from the emitted JSON
/// (re-read restores it via [`default_edge`]) — keeps exported blueprints clean.
fn is_default_edge(edge: &str) -> bool {
    edge == "data"
}

/// Export guard for a plain `bool` default (serde's `skip_serializing_if` needs a
/// `fn(&T) -> bool`, so the `&bool` is required by the trait, not a choice).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Decode a lowercase-or-uppercase 32-byte hex string (an EXEC body signature id).
/// A hand-rolled, dependency-free codec (mirrors the `kx-cli` `hex::decode_fixed`
/// behaviour): fail-closed on odd length, a non-hex digit, or the wrong byte count.
fn decode_hex_32(s: &str) -> Result<[u8; 32], BlueprintError> {
    if !s.len().is_multiple_of(2) {
        return Err(BlueprintError::new(
            "body_signature_id: hex string has an odd number of digits",
        ));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    <[u8; 32]>::try_from(out.as_slice()).map_err(|_| {
        BlueprintError::new(format!(
            "body_signature_id: expected 32 bytes (64 hex chars), got {}",
            out.len()
        ))
    })
}

fn hex_val(c: u8) -> Result<u8, BlueprintError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(BlueprintError::new(format!(
            "body_signature_id: not a hex digit: {:?}",
            c as char
        ))),
    }
}

/// Build the `SubmitWorkflowRequest` from a parsed [`DagSpec`]. The single canonical
/// assembly the `kx blueprint`/`chain`/`app` verbs AND the gateway host all funnel
/// through — a blueprint lowers to byte-identical wire bytes regardless of caller.
///
/// # Errors
/// [`BlueprintError`] on a bad step kind, a bad edge kind, a reserved `exec`, or a
/// malformed `body_signature_id`.
pub fn to_request(spec: DagSpec) -> Result<proto::SubmitWorkflowRequest, BlueprintError> {
    let mut steps = Vec::with_capacity(spec.steps.len());
    for (i, s) in spec.steps.into_iter().enumerate() {
        // A per-step skills/connections/datasets list is an APP binding with no workflow
        // meaning — refuse it here rather than let it vanish into a lowering that cannot
        // carry it (`RunApp` takes them off the spec before calling this, so the App path
        // never trips it).
        refuse_app_only_bindings(i, &s)?;
        // Batch A: the kind is resolved (inferred when omitted, validated when explicit;
        // `exec` reserved) — see [`StepSpec::resolve_kind`].
        let kind = s.resolve_kind()?;
        let body_signature_id = match s.body_signature_id {
            Some(h) => decode_hex_32(&h)?.to_vec(),
            None => Vec::new(),
        };
        let mut params: BTreeMap<String, Vec<u8>> = s
            .params
            .into_iter()
            .map(|(k, v)| (k, v.into_bytes()))
            .collect();
        // PR-6b-2: a TOOL step lowers its authored args to the canonical-JSON blob
        // under TOOL_ARGS_KEY (`s.args` is a BTreeMap ⇒ sorted keys; serde_json ⇒
        // compact) — byte-identical to the Py/TS factories + the coordinator's
        // `is_authored_tool` discriminant.
        if kind == proto::WorkflowStepKind::Tool {
            let blob = serde_json::to_string(&s.args)
                .map_err(|e| BlueprintError::new(format!("tool args: {e}")))?;
            params.insert(TOOL_ARGS_KEY.to_string(), blob.into_bytes());
        }
        // PR-9b (D161.1): an agentic MODEL step (MODEL + a non-empty tool_contract)
        // lowers its bounded-loop budget to canonical-JSON `u32` bytes under the
        // react budget keys (the form `react_seed_params` reads). Absent ⇒ the
        // coordinator default. The decimal string of a `u32` IS canonical JSON.
        if kind == proto::WorkflowStepKind::Model && !s.tool_contract.is_empty() {
            if let Some(n) = s.max_turns {
                params.insert(REACT_MAX_TURNS_KEY.to_string(), n.to_string().into_bytes());
            }
            if let Some(n) = s.max_tool_calls {
                params.insert(
                    REACT_MAX_TOOL_CALLS_KEY.to_string(),
                    n.to_string().into_bytes(),
                );
            }
        }
        steps.push(proto::WorkflowStep {
            kind: kind as i32,
            model_id: s.model_id,
            prompt: s.prompt,
            body_signature_id,
            tool_contract: s.tool_contract.into_iter().collect(),
            params: params.into_iter().collect(),
        });
    }
    let mut edges = Vec::with_capacity(spec.edges.len());
    for e in spec.edges {
        let edge_kind = match e.edge.as_str() {
            "data" => proto::EdgeKind::Data,
            "control" => proto::EdgeKind::Control,
            other => {
                return Err(BlueprintError::new(format!(
                    "edge must be data|control, got {other:?}"
                )));
            }
        };
        edges.push(proto::WorkflowEdge {
            parent: e.parent,
            child: e.child,
            edge_kind: edge_kind as i32,
            non_cascade: e.non_cascade,
        });
    }
    let execution_mode = match spec.execution_mode.as_deref() {
        Some("dynamic") => proto::WorkflowExecutionMode::Dynamic,
        _ => proto::WorkflowExecutionMode::Frozen,
    };
    Ok(proto::SubmitWorkflowRequest {
        seed: spec.seed,
        steps,
        edges,
        execution_mode: execution_mode as i32,
        context_bundles: spec.context_bundles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_two_step_data_dag_to_the_request() {
        let spec: DagSpec = serde_json::from_str(
            r#"{ "seed": 7,
                 "steps": [ {"kind":"pure","params":{"topic":"hi"}}, {"kind":"pure"} ],
                 "edges": [ {"parent":0,"child":1,"edge":"data"} ] }"#,
        )
        .unwrap();
        let req = to_request(spec).unwrap();
        assert_eq!(req.seed, 7);
        assert_eq!(req.steps.len(), 2);
        assert_eq!(req.steps[0].kind, proto::WorkflowStepKind::Pure as i32);
        assert_eq!(req.steps[0].params.get("topic").unwrap(), b"hi");
        assert_eq!(req.edges.len(), 1);
        assert_eq!(req.edges[0].edge_kind, proto::EdgeKind::Data as i32);
        assert_eq!(
            req.execution_mode,
            proto::WorkflowExecutionMode::Frozen as i32
        );
    }

    #[test]
    fn rejects_a_bad_kind() {
        let spec: DagSpec =
            serde_json::from_str(r#"{ "steps": [ {"kind":"frobnicate"} ] }"#).unwrap();
        assert!(to_request(spec).is_err());
    }

    /// ★ THE DIGEST-INVARIANCE PROPERTY, at this layer. The four App-binding fields are
    /// omitted from the emitted JSON when empty and are absent from the lowering, so a
    /// blueprint written before they existed and the same blueprint parsed by this build
    /// compile to the IDENTICAL request. Every already-authored App's `MoteId`s depend on
    /// this holding.
    #[test]
    fn app_bindings_absent_lower_identically_to_a_pre_binding_blueprint() {
        let json = r#"{ "seed": 3,
             "steps": [ {"kind":"model","prompt":"go"}, {"kind":"pure"} ],
             "edges": [ {"parent":0,"child":1} ] }"#;
        let before: DagSpec = serde_json::from_str(json).unwrap();
        let emitted = serde_json::to_string(&before).unwrap();
        assert!(
            !emitted.contains("skills")
                && !emitted.contains("connections")
                && !emitted.contains("datasets")
                && !emitted.contains("apps"),
            "an unbound blueprint must not grow keys: {emitted}"
        );
        let reparsed: DagSpec = serde_json::from_str(&emitted).unwrap();
        assert_eq!(
            to_request(serde_json::from_str::<DagSpec>(json).unwrap()).unwrap(),
            to_request(reparsed).unwrap()
        );
    }

    /// A per-step capability list is an APP binding. Lowering it as a WORKFLOW has nowhere
    /// to resolve the name, so it must refuse — dropping it would hand the author a
    /// workflow silently missing the reach they wrote down.
    #[test]
    fn refuses_app_only_bindings_on_the_workflow_lowering_path() {
        for field in ["skills", "connections", "datasets", "apps"] {
            let spec: DagSpec = serde_json::from_str(&format!(
                r#"{{ "steps": [ {{"kind":"pure"}}, {{"kind":"model","prompt":"go","{field}":["x"]}} ] }}"#
            ))
            .unwrap();
            let err = to_request(spec).expect_err("an App binding must not lower as a workflow");
            assert!(err.0.contains("step 1"), "names the step: {err}");
            assert!(err.0.contains(field), "names the field: {err}");
            assert!(err.0.contains("App"), "says where it IS honoured: {err}");
        }
    }

    /// The bindings survive an export→import round trip verbatim — the App path reads them
    /// off the parsed spec, so a lossy round trip would silently unbind a live App.
    #[test]
    fn app_bindings_round_trip_through_serialize() {
        let json = r#"{ "seed": 1, "steps": [ {"kind":"model","prompt":"go",
             "skills":["triage"],"connections":["kx-connector-gmail"],"datasets":["support"],
             "apps":["apps/local/research"]} ] }"#;
        let spec: DagSpec = serde_json::from_str(json).unwrap();
        assert!(spec.steps[0].has_app_bindings());
        let reparsed: DagSpec =
            serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(reparsed.steps[0].skills, vec!["triage".to_string()]);
        assert_eq!(
            reparsed.steps[0].connections,
            vec!["kx-connector-gmail".to_string()]
        );
        assert_eq!(reparsed.steps[0].datasets, vec!["support".to_string()]);
        assert_eq!(
            reparsed.steps[0].apps,
            vec!["apps/local/research".to_string()]
        );
    }

    /// Batch B: a `DagSpec` survives Serialize → Deserialize and re-compiles to the
    /// IDENTICAL proto — the export→import byte-stability invariant (covering the
    /// `skip_serializing_if` guards: the tool `args` + the agentic budget round-trip).
    #[test]
    fn dagspec_serialize_round_trip_compiles_identically() {
        let json = r#"{
            "seed": 5,
            "steps": [
                {"kind":"pure","params":{"topic":"hi"}},
                {"kind":"tool","tool_contract":{"echo":"1"},"args":{"n":3,"msg":"x"}},
                {"kind":"model","prompt":"go","tool_contract":{"web-search":"1"},"max_turns":4,"max_tool_calls":3}
            ],
            "edges": [ {"parent":0,"child":1}, {"parent":1,"child":2,"non_cascade":true} ],
            "context_bundles": ["team/ctx/spec"]
        }"#;
        let spec: DagSpec = serde_json::from_str(json).unwrap();
        let req_direct = to_request(spec).unwrap();

        let spec2: DagSpec = serde_json::from_str(json).unwrap();
        let emitted = serde_json::to_string_pretty(&spec2).unwrap();
        let reparsed: DagSpec = serde_json::from_str(&emitted).unwrap();
        let req_round_trip = to_request(reparsed).unwrap();

        assert_eq!(
            req_direct, req_round_trip,
            "export→import must re-compile to a byte-identical SubmitWorkflowRequest"
        );
    }

    fn step(json: serde_json::Value) -> StepSpec {
        serde_json::from_value(json).expect("a StepSpec")
    }

    #[test]
    fn omitted_kind_is_inferred_from_field_presence() {
        use proto::WorkflowStepKind::{Model, Pure, Tool};
        assert_eq!(step(serde_json::json!({})).resolve_kind().unwrap(), Pure);
        assert_eq!(
            step(serde_json::json!({ "params": { "topic": "hi" } }))
                .resolve_kind()
                .unwrap(),
            Pure
        );
        assert_eq!(
            step(serde_json::json!({ "model_id": "m" }))
                .resolve_kind()
                .unwrap(),
            Model
        );
        assert_eq!(
            step(serde_json::json!({ "prompt": "go" }))
                .resolve_kind()
                .unwrap(),
            Model
        );
        assert_eq!(
            step(serde_json::json!({ "tool_contract": { "echo": "1" } }))
                .resolve_kind()
                .unwrap(),
            Tool
        );
        assert_eq!(
            step(serde_json::json!({ "prompt": "go", "tool_contract": { "echo": "1" } }))
                .resolve_kind()
                .unwrap(),
            Model
        );
    }

    #[test]
    fn omitted_kind_lowers_byte_identically_to_the_explicit_form() {
        for (omitted, explicit) in [
            (
                serde_json::json!({ "params": { "topic": "hi" } }),
                serde_json::json!({ "kind": "pure", "params": { "topic": "hi" } }),
            ),
            (
                serde_json::json!({ "model_id": "m", "prompt": "go" }),
                serde_json::json!({ "kind": "model", "model_id": "m", "prompt": "go" }),
            ),
            (
                serde_json::json!({ "tool_contract": { "echo": "1" }, "args": { "n": 3 } }),
                serde_json::json!({ "kind": "tool", "tool_contract": { "echo": "1" }, "args": { "n": 3 } }),
            ),
        ] {
            let lower = |s: serde_json::Value| {
                to_request(DagSpec {
                    seed: 0,
                    steps: vec![step(s)],
                    edges: vec![],
                    execution_mode: None,
                    context_bundles: vec![],
                })
                .unwrap()
                .steps
                .remove(0)
            };
            assert_eq!(lower(omitted.clone()), lower(explicit.clone()), "{omitted}");
        }
    }

    #[test]
    fn explicit_kind_conflicting_with_fields_is_rejected() {
        assert!(step(serde_json::json!({ "kind": "pure", "model_id": "m" }))
            .resolve_kind()
            .is_err());
        assert!(
            step(serde_json::json!({ "kind": "pure", "tool_contract": { "echo": "1" } }))
                .resolve_kind()
                .is_err()
        );
        assert!(
            step(serde_json::json!({ "kind": "model", "args": { "n": 3 } }))
                .resolve_kind()
                .is_err()
        );
        assert!(step(
            serde_json::json!({ "kind": "tool", "tool_contract": { "e": "1" }, "model_id": "m" })
        )
        .resolve_kind()
        .is_err());
    }

    #[test]
    fn exec_kind_is_reserved_and_rejected_client_side() {
        let err = step(serde_json::json!({ "kind": "exec", "body_signature_id": null }))
            .resolve_kind()
            .expect_err("exec is reserved")
            .to_string()
            .to_lowercase();
        assert!(err.contains("reserved"), "got: {err}");
    }

    #[test]
    fn omitted_kind_round_trips_through_to_request() {
        let spec: DagSpec = serde_json::from_str(
            r#"{ "steps": [ {"params":{"topic":"hi"}}, {"model_id":"m","prompt":"go"} ],
                 "edges": [ {"parent":0,"child":1} ] }"#,
        )
        .unwrap();
        let req = to_request(spec).unwrap();
        assert_eq!(req.steps[0].kind, proto::WorkflowStepKind::Pure as i32);
        assert_eq!(req.steps[1].kind, proto::WorkflowStepKind::Model as i32);
    }

    #[test]
    fn decode_hex_32_round_trips_and_rejects_bad_input() {
        let hex = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let bytes = decode_hex_32(hex).unwrap();
        assert_eq!(bytes[0], 0x00);
        assert_eq!(bytes[1], 0x11);
        assert_eq!(bytes[31], 0xff);
        assert!(decode_hex_32("abc").is_err(), "odd length");
        assert!(decode_hex_32("zz").is_err(), "bad digit");
        assert!(decode_hex_32("00").is_err(), "wrong length");
    }
}
