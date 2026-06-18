//! `kx blueprint run --file <dag.json> [--wait] ...` — author a Tier-1 DAG (a
//! vetted palette of PURE / MODEL steps + DATA/CONTROL edges) and run it via the
//! `SubmitWorkflow` path. The server compiles the DAG, derives all identity, and
//! builds every warrant from the party's grants (SN-8) — the client sends only the
//! topology + params. The authored run is then viewable in the console (Runs → the
//! live DAG, Monitoring).
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
//! `kind` ∈ {`pure`, `model`} (`exec` is reserved); `edge` ∈ {`data`, `control`}.
//! `context_bundles` (PR-7, optional) attaches named context bundles to the run —
//! the server injects them into every entry Mote at bind (SN-8).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use kx_proto::proto;
use serde::Deserialize;

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
#[derive(Debug, Deserialize)]
pub(crate) struct DagSpec {
    #[serde(default)]
    pub(crate) seed: u32,
    pub(crate) steps: Vec<StepSpec>,
    #[serde(default)]
    pub(crate) edges: Vec<EdgeSpec>,
    #[serde(default)]
    pub(crate) execution_mode: Option<String>,
    /// PR-7: context-bundle handles to attach to the run (chain-level grounding the
    /// SERVER resolves + injects into every entry Mote at bind, SN-8). Verbatim
    /// order; empty ⇒ byte-identical to pre-PR-7.
    #[serde(default)]
    pub(crate) context_bundles: Vec<String>,
}

/// PR-6b-2: the single canonical config key a `tool` step's authored args ride
/// under. MUST equal `kx_mote::TOOL_ARGS_KEY` + the Py/TS `TOOL_ARGS_KEY` (pinned
/// identical by the golden corpus). Hardcoded to avoid a `kx-mote` dep on the CLI.
const TOOL_ARGS_KEY: &str = "kx.tool.args";

#[derive(Debug, Deserialize)]
pub(crate) struct StepSpec {
    /// `pure` | `model` | `exec` (reserved) | `tool` (PR-6b-2).
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) model_id: String,
    #[serde(default)]
    pub(crate) prompt: String,
    /// EXEC only: the registered body's content/signature id as 64-char hex.
    #[serde(default)]
    pub(crate) body_signature_id: Option<String>,
    #[serde(default)]
    pub(crate) tool_contract: BTreeMap<String, String>,
    /// Free config entries; values are UTF-8 strings.
    #[serde(default)]
    pub(crate) params: BTreeMap<String, String>,
    /// TOOL only (PR-6b-2): the tool-call arguments, serialized at lowering to ONE
    /// canonical-JSON object under [`TOOL_ARGS_KEY`] (sorted keys, compact) —
    /// byte-identical to the Py/TS `tool()` factories. No floats (SN-8).
    #[serde(default)]
    pub(crate) args: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdgeSpec {
    pub(crate) parent: u32,
    pub(crate) child: u32,
    /// `data` (default) | `control`.
    #[serde(default = "default_edge")]
    pub(crate) edge: String,
    #[serde(default)]
    pub(crate) non_cascade: bool,
}

fn default_edge() -> String {
    "data".to_string()
}

/// Parsed `blueprint` arguments.
#[derive(Debug)]
pub struct BlueprintArgs {
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

/// Parse `blueprint run --file <p> [--wait] ...` (the verb already consumed `run`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<BlueprintArgs, CliError> {
    let sub = args
        .next()
        .ok_or_else(|| CliError::Usage("blueprint expects a subcommand (run)".into()))?;
    if sub != "run" {
        return Err(CliError::Usage(format!(
            "unknown blueprint subcommand {sub:?} (only `run`)"
        )));
    }
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

    let file =
        file.ok_or_else(|| CliError::Usage("blueprint run requires --file <dag.json>".into()))?;
    Ok(BlueprintArgs {
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
        let kind = match s.kind.as_str() {
            "pure" => proto::WorkflowStepKind::Pure,
            "model" => proto::WorkflowStepKind::Model,
            "exec" => proto::WorkflowStepKind::Exec,
            "tool" => proto::WorkflowStepKind::Tool,
            other => {
                return Err(CliError::Usage(format!(
                    "step kind must be pure|model|exec|tool, got {other:?}"
                )));
            }
        };
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

/// Execute `blueprint run`.
pub async fn execute(args: BlueprintArgs) -> Result<(), CliError> {
    let raw = std::fs::read(&args.file)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", args.file.display())))?;
    let spec: DagSpec = serde_json::from_slice(&raw)
        .map_err(|e| CliError::Usage(format!("invalid blueprint JSON: {e}")))?;
    let req = to_request(spec)?;

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
}
