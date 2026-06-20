//! `kx blueprint run --file <dag.json> [--wait] ...` — author a Tier-1 DAG (a
//! vetted palette of PURE / MODEL steps + DATA/CONTROL edges) and run it via the
//! `SubmitWorkflow` path. The server compiles the DAG, derives all identity, and
//! builds every warrant from the party's grants (SN-8) — the client sends only the
//! topology + params. The authored run is then viewable in the console (Runs → the
//! live DAG, Monitoring).
//!
//! `kx blueprint import --file <dag.json>` (Batch B / D161.2) — validate + summarize a
//! portable blueprint JSON WITHOUT contacting a gateway (the symmetric counterpart of
//! `kx chain run --emit-blueprint <file>`): the same `to_request` compile validates the
//! DAG client-side (kinds / edges / tool args / reserved `exec` all fail-closed) and
//! prints the resolved shape; then run it with `blueprint run --file`. A portable DAG
//! JSON is the share/round-trip artifact; cross-party sharing/marketplace = Cloud.
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
//! `kind` ∈ {`pure`, `model`, `tool`} (`exec` is reserved) and is now **OPTIONAL**
//! (Batch A): omit it and the kind is inferred from field presence (`model_id`/`prompt`
//! ⇒ `model`, a `tool_contract` with no model fields ⇒ `tool`, else ⇒ `pure`); an
//! explicit kind must agree with the fields (fail-closed). `edge` ∈ {`data`,
//! `control`}. `context_bundles` (PR-7, optional) attaches named context bundles to
//! the run — the server injects them into every entry Mote at bind (SN-8).
//!
//! A `tool` step (PR-6b-2) fires ONE registered tool: it carries `tool_contract`
//! `{ tool_id: version }` + (optional) `args` (lowered to the canonical `kx.tool.args`
//! blob). A `model` step carrying a non-empty `tool_contract` is a **deterministic-
//! agentic step** (PR-9b / D161.1) — the model runs a bounded reason→tool→observe
//! loop over the granted tool SET, bounded by optional `max_turns` / `max_tool_calls`.
//! In every case the SERVER resolves the tool(s) in its live registry + builds the
//! per-step warrant (the client never supplies a warrant or grants — SN-8).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use kx_proto::proto;
use serde::{Deserialize, Serialize};

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, hex, verbs, wait};

const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The author-side DAG shape parsed from `--file`.
///
/// `pub(crate)` so the string-DSL lowering in [`crate::verbs::chain`] can reuse
/// the one canonical proto assembly ([`to_request`]) instead of re-deriving the
/// `SubmitWorkflowRequest` — a chain only changes *how* the `(steps, edges)` are
/// authored, never how they lower to the wire.
/// `Serialize` (Batch B / D161.2): a parsed/lowered `DagSpec` re-serializes to a
/// portable blueprint JSON (`kx chain run --emit-blueprint`). The `skip_serializing_if`
/// guards keep the artifact clean — each skipped field's `#[serde(default)]` exactly
/// reproduces the omitted value on re-read, so export→import is byte-stable.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct DagSpec {
    #[serde(default)]
    pub(crate) seed: u32,
    pub(crate) steps: Vec<StepSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) edges: Vec<EdgeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_mode: Option<String>,
    /// PR-7: context-bundle handles to attach to the run (chain-level grounding the
    /// SERVER resolves + injects into every entry Mote at bind, SN-8). Verbatim
    /// order; empty ⇒ byte-identical to pre-PR-7.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) context_bundles: Vec<String>,
}

/// PR-6b-2: the single canonical config key a `tool` step's authored args ride
/// under. MUST equal `kx_mote::TOOL_ARGS_KEY` + the Py/TS `TOOL_ARGS_KEY` (pinned
/// identical by the golden corpus). Hardcoded to avoid a `kx-mote` dep on the CLI.
const TOOL_ARGS_KEY: &str = "kx.tool.args";

/// PR-9b (D161.1): the canonical config keys a deterministic-agentic MODEL step's
/// bounded-loop budget rides under (decimal-string bytes ⇒ canonical-JSON `u32`,
/// the form the coordinator's `react_seed_params` reads). MUST equal
/// `kx_mote::REACT_MAX_TURNS_KEY` / `REACT_MAX_TOOL_CALLS_KEY` (pinned by the
/// golden corpus). Hardcoded to avoid a `kx-mote` dep on the CLI.
const REACT_MAX_TURNS_KEY: &str = "max_turns";
const REACT_MAX_TOOL_CALLS_KEY: &str = "max_tool_calls";

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct StepSpec {
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
    pub(crate) kind: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) model_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) prompt: String,
    /// EXEC only: the registered body's content/signature id as 64-char hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) body_signature_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) tool_contract: BTreeMap<String, String>,
    /// Free config entries; values are UTF-8 strings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) params: BTreeMap<String, String>,
    /// TOOL only (PR-6b-2): the tool-call arguments, serialized at lowering to ONE
    /// canonical-JSON object under [`TOOL_ARGS_KEY`] (sorted keys, compact) —
    /// byte-identical to the Py/TS `tool()` factories. No floats (SN-8).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) args: BTreeMap<String, serde_json::Value>,
    /// Agentic MODEL step only (PR-9b, D161.1): the bounded reason→tool→observe
    /// loop budget. Lowered to canonical-JSON `u32` bytes under
    /// [`REACT_MAX_TURNS_KEY`] / [`REACT_MAX_TOOL_CALLS_KEY`] in `params` when the
    /// step is a MODEL step with a non-empty `tool_contract`; ignored otherwise.
    /// Absent ⇒ the coordinator default (8 turns / 6 tool calls).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_tool_calls: Option<u32>,
}

impl StepSpec {
    /// Resolve the step's wire kind (Batch A authoring veneer). When `kind` is omitted
    /// it is INFERRED from field presence; when present it is an override that must
    /// AGREE with the fields (fail-closed). `exec` is rejected client-side (the binder
    /// reserves it — fail at authoring with a clear message rather than a server
    /// round-trip). Pure derivation of `&self` — `to_request` and the chain `@`-grant
    /// check both call it, and it is idempotent under grant injection (model fields are
    /// checked before `tool_contract`, so injecting tags never re-classifies a step).
    pub(crate) fn resolve_kind(&self) -> Result<proto::WorkflowStepKind, CliError> {
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
                return Err(CliError::Usage(
                    "step kind `exec` is reserved (a registered body is not yet runnable); \
                     use pure|model|tool"
                        .into(),
                ));
            }
            other => {
                return Err(CliError::Usage(format!(
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
            return Err(CliError::Usage(format!(
                "step kind {explicit:?} conflicts with its fields ({why})"
            )));
        }
        Ok(kind)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct EdgeSpec {
    pub(crate) parent: u32,
    pub(crate) child: u32,
    /// `data` (default) | `control`. Omitted on export when it is the `data` default.
    #[serde(default = "default_edge", skip_serializing_if = "is_default_edge")]
    pub(crate) edge: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) non_cascade: bool,
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

/// `blueprint run` (submit + optionally wait) vs `blueprint import` (validate +
/// summarize a portable blueprint JSON WITHOUT contacting a gateway — the symmetric
/// counterpart of `kx chain run --emit-blueprint`, Batch B / D161.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlueprintMode {
    /// `blueprint run` — compile + submit (and optionally `--wait`) via the gateway.
    Run,
    /// `blueprint import` — compile + validate + summarize a portable blueprint JSON
    /// offline (no gateway); the counterpart of `chain run --emit-blueprint`.
    Import,
}

/// Parsed `blueprint` arguments.
#[derive(Debug)]
pub struct BlueprintArgs {
    /// `run` (submit) or `import` (validate + summarize, no gateway).
    pub mode: BlueprintMode,
    /// The author-side DAG JSON file to compile + run.
    pub file: PathBuf,
    /// Run to completion and print the committed result (`--wait`).
    pub wait: bool,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Write the committed result bytes to this file instead of inlining them.
    pub out: Option<PathBuf>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `blueprint run|import --file <p> [--wait] ...` (the verb already consumed the
/// leading `blueprint`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<BlueprintArgs, CliError> {
    let sub = args
        .next()
        .ok_or_else(|| CliError::Usage("blueprint expects a subcommand (run|import)".into()))?;
    let mode = match sub.as_str() {
        "run" => BlueprintMode::Run,
        "import" => BlueprintMode::Import,
        other => {
            return Err(CliError::Usage(format!(
                "unknown blueprint subcommand {other:?} (run|import)"
            )));
        }
    };
    let mut file: Option<PathBuf> = None;
    let mut wait = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut out: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--file" => file = Some(PathBuf::from(next_value(&mut args, "--file")?)),
            "--wait" => wait = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let file = file
        .ok_or_else(|| CliError::Usage("blueprint run|import requires --file <dag.json>".into()))?;
    Ok(BlueprintArgs {
        mode,
        file,
        wait,
        timeout_secs,
        out,
        common,
    })
}

/// Build the `SubmitWorkflowRequest` from a parsed `DagSpec`. `pub(crate)` so the
/// string-DSL lowering ([`crate::verbs::chain`]) feeds the SAME canonical assembly.
pub(crate) fn to_request(spec: DagSpec) -> Result<proto::SubmitWorkflowRequest, CliError> {
    let mut steps = Vec::with_capacity(spec.steps.len());
    for s in spec.steps {
        // Batch A: the kind is resolved (inferred when omitted, validated when explicit;
        // `exec` reserved) — see [`StepSpec::resolve_kind`].
        let kind = s.resolve_kind()?;
        let body_signature_id = match s.body_signature_id {
            Some(h) => hex::decode_fixed::<32>(&h)
                .map_err(|e| CliError::Usage(format!("body_signature_id: {e}")))?
                .to_vec(),
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
                .map_err(|e| CliError::Usage(format!("tool args: {e}")))?;
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
                return Err(CliError::Usage(format!(
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

/// Execute `blueprint run` / `blueprint import`.
pub async fn execute(args: BlueprintArgs) -> Result<(), CliError> {
    let raw = std::fs::read(&args.file)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", args.file.display())))?;
    let spec: DagSpec = serde_json::from_slice(&raw)
        .map_err(|e| CliError::Usage(format!("invalid blueprint JSON: {e}")))?;
    // `to_request` is the canonical compile + client-side validation (kinds / edges /
    // tool args / reserved `exec` all fail-closed here, BEFORE any gateway contact).
    let req = to_request(spec)?;

    // `import` = validate + summarize the portable blueprint WITHOUT a gateway (the
    // counterpart of `chain run --emit-blueprint`): the compile above already validated
    // it; print the resolved DAG shape and stop. No submit, no connection.
    if args.mode == BlueprintMode::Import {
        print_import_summary(&req, &args.file, args.common.json);
        return Ok(());
    }

    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let handle = client
        .submit_workflow(resolved.request(req)?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if args.wait {
        let outcome = wait::await_any_result(
            &mut client,
            &resolved,
            handle.instance_id,
            Duration::from_secs(args.timeout_secs),
        )
        .await?;
        verbs::finish_wait(&outcome, args.common.json, args.out.as_deref())
    } else {
        println!("{}", format::render_submit(&handle, args.common.json));
        Ok(())
    }
}

/// Map a proto `WorkflowStepKind` back to its DSL string (display helper).
fn step_kind_name(k: i32) -> &'static str {
    match proto::WorkflowStepKind::try_from(k) {
        Ok(proto::WorkflowStepKind::Pure) => "pure",
        Ok(proto::WorkflowStepKind::Model) => "model",
        Ok(proto::WorkflowStepKind::Tool) => "tool",
        Ok(proto::WorkflowStepKind::Exec) => "exec",
        _ => "unspecified",
    }
}

/// Print a human / JSON summary of an imported blueprint (`blueprint import`) — the
/// resolved DAG shape, display-only, no run. The `to_request` compile already
/// validated it (kinds / edges / args / reserved `exec`), so reaching here means valid.
fn print_import_summary(req: &proto::SubmitWorkflowRequest, file: &std::path::Path, json: bool) {
    let mode = if req.execution_mode == proto::WorkflowExecutionMode::Dynamic as i32 {
        "dynamic"
    } else {
        "frozen"
    };
    let sorted_tools = |s: &proto::WorkflowStep| {
        let mut t: Vec<String> = s.tool_contract.keys().cloned().collect();
        t.sort();
        t
    };
    if json {
        let steps: Vec<serde_json::Value> = req
            .steps
            .iter()
            .map(|s| {
                serde_json::json!({
                    "kind": step_kind_name(s.kind),
                    "model_id": s.model_id,
                    "tools": sorted_tools(s),
                })
            })
            .collect();
        let out = serde_json::json!({
            "file": file.display().to_string(),
            "valid": true,
            "seed": req.seed,
            "execution_mode": mode,
            "steps": steps,
            "edges": req.edges.len(),
            "context_bundles": req.context_bundles,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        println!("blueprint {} is valid", file.display());
        println!(
            "  seed={}  mode={}  steps={}  edges={}  context_bundles={}",
            req.seed,
            mode,
            req.steps.len(),
            req.edges.len(),
            req.context_bundles.len()
        );
        for (i, s) in req.steps.iter().enumerate() {
            let tools = sorted_tools(s);
            let model = if s.model_id.is_empty() {
                "<served>".to_string()
            } else {
                s.model_id.clone()
            };
            let detail = match step_kind_name(s.kind) {
                "model" if !tools.is_empty() => format!("model {model} @{tools:?}"),
                "model" => format!("model {model}"),
                "tool" => format!("tool {tools:?}"),
                k => k.to_string(),
            };
            println!("  [{i}] {detail}");
        }
        println!("run it with: kx blueprint run --file {}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<BlueprintArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn requires_run_subcommand_and_file() {
        assert!(p(&[]).is_err(), "no subcommand is a usage error");
        assert!(p(&["nope"]).is_err(), "unknown subcommand is a usage error");
        assert!(p(&["run"]).is_err(), "run without --file is a usage error");
    }

    #[test]
    fn parses_run_with_flags() {
        let a = p(&[
            "run",
            "--file",
            "dag.json",
            "--wait",
            "--json",
            "--timeout-secs",
            "30",
        ])
        .unwrap();
        assert!(a.wait && a.common.json);
        assert_eq!(a.timeout_secs, 30);
        assert_eq!(a.file, PathBuf::from("dag.json"));
    }

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

    #[test]
    fn parses_import_subcommand() {
        let a = p(&["import", "--file", "bp.json"]).unwrap();
        assert_eq!(a.mode, BlueprintMode::Import);
        assert_eq!(a.file, PathBuf::from("bp.json"));
        assert_eq!(
            p(&["run", "--file", "bp.json"]).unwrap().mode,
            BlueprintMode::Run
        );
        assert!(
            p(&["import"]).is_err(),
            "import without --file is a usage error"
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

        // Re-parse the SAME source, serialize it (the export path), re-read, re-compile.
        let spec2: DagSpec = serde_json::from_str(json).unwrap();
        let emitted = serde_json::to_string_pretty(&spec2).unwrap();
        let reparsed: DagSpec = serde_json::from_str(&emitted).unwrap();
        let req_round_trip = to_request(reparsed).unwrap();

        assert_eq!(
            req_direct, req_round_trip,
            "export→import must re-compile to a byte-identical SubmitWorkflowRequest"
        );
    }

    // ---- Batch A: kind inference + agreement (the JSON authoring veneer) ----

    fn step(json: serde_json::Value) -> StepSpec {
        serde_json::from_value(json).expect("a StepSpec")
    }

    #[test]
    fn omitted_kind_is_inferred_from_field_presence() {
        use proto::WorkflowStepKind::{Model, Pure, Tool};
        // no fields ⇒ pure
        assert_eq!(step(serde_json::json!({})).resolve_kind().unwrap(), Pure);
        assert_eq!(
            step(serde_json::json!({ "params": { "topic": "hi" } }))
                .resolve_kind()
                .unwrap(),
            Pure
        );
        // model_id OR prompt ⇒ model
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
        // tool_contract with no model fields ⇒ tool
        assert_eq!(
            step(serde_json::json!({ "tool_contract": { "echo": "1" } }))
                .resolve_kind()
                .unwrap(),
            Tool
        );
        // an agentic model step (model fields + tool_contract) is STILL model, not tool
        assert_eq!(
            step(serde_json::json!({ "prompt": "go", "tool_contract": { "echo": "1" } }))
                .resolve_kind()
                .unwrap(),
            Model
        );
    }

    #[test]
    fn omitted_kind_lowers_byte_identically_to_the_explicit_form() {
        // The whole point of the veneer: an omitted kind must produce the SAME wire
        // step as the explicit kind. Compare the lowered WorkflowStep for both.
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
        // pure + a model field / tool_contract / args
        assert!(step(serde_json::json!({ "kind": "pure", "model_id": "m" }))
            .resolve_kind()
            .is_err());
        assert!(
            step(serde_json::json!({ "kind": "pure", "tool_contract": { "echo": "1" } }))
                .resolve_kind()
                .is_err()
        );
        // model + tool-only args
        assert!(
            step(serde_json::json!({ "kind": "model", "args": { "n": 3 } }))
                .resolve_kind()
                .is_err()
        );
        // tool + a model field
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
}
