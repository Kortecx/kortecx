//! `kx blueprint run --file <dag.json> [--wait] ...` — author a Tier-1 DAG (a
//! vetted palette of PURE / MODEL / TOOL steps + DATA/CONTROL edges) and run it via
//! the `SubmitWorkflow` path. The server compiles the DAG, derives all identity, and
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
//! The author-side DAG shape ([`DagSpec`]/[`StepSpec`]/[`EdgeSpec`]) and the ONE
//! canonical lowering to a `SubmitWorkflowRequest` ([`to_request`]) live in the
//! FFI-free `kx-blueprint` leaf (re-exported below) so the gateway host lowers a
//! stored App's blueprint through the IDENTICAL path (G2). This module keeps only the
//! CLI verb: arg parsing, `run`/`import` execution, and the import summary.

use std::path::PathBuf;
use std::time::Duration;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, verbs, wait};

/// The canonical blueprint shape + lowering, extracted to the FFI-free `kx-blueprint`
/// leaf. Re-exported so `crate::verbs::blueprint::{DagSpec, StepSpec, EdgeSpec,
/// to_request}` keeps resolving for the `chain` string-DSL + the `app` run path (both
/// funnel through the SAME `to_request`, byte-identical to the server-side App run).
pub(crate) use kx_blueprint::{to_request, DagSpec, EdgeSpec, StepSpec};

const DEFAULT_TIMEOUT_SECS: u64 = 120;

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

    /// Sanity: the re-exported lowering still compiles a minimal DAG (the exhaustive
    /// lowering/round-trip coverage lives in `kx-blueprint`'s own test module).
    #[test]
    fn reexported_to_request_lowers_a_minimal_dag() {
        let spec: DagSpec =
            serde_json::from_str(r#"{ "seed": 1, "steps": [ {"kind":"pure"} ] }"#).unwrap();
        let req = to_request(spec).unwrap();
        assert_eq!(req.seed, 1);
        assert_eq!(req.steps.len(), 1);
    }
}
