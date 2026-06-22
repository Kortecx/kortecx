//! `kx agent run --goal <text> ...` — the embeddable agent-runner (PR-9c-1).
//!
//! A thin wrapper over Invoke of `kx/recipes/react`: the runtime completes the
//! goal AGENTICALLY (reason → permission-gated tool calls → answer) and the verb
//! prints the answer plus the AUDITED action set (the chain's settled `tool`
//! turns). NEVER SubmitRun (BLOCKER #5); the warrant is always server-derived
//! (SN-8). `--input k=v` folds into the goal prompt — the react contract has no
//! structured input slot yet (instruction / max_turns / max_tool_calls only).

use std::time::Duration;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;
use crate::wait::{self, WaitState};

/// Default `--timeout-secs` (matches `invoke`).
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// The steered ReAct recipe the runner invokes (Invoke-only, server-warranted).
const REACT_RECIPE_HANDLE: &str = "kx/recipes/react";
/// The recipe's anchored bounded-loop budget (mirrors the SDK + the UI's planReactArgs).
const DEFAULT_MAX_TURNS: u32 = 8;
const DEFAULT_MAX_TOOL_CALLS: u32 = 6;

/// Parsed `agent run` arguments.
#[derive(Debug)]
pub struct AgentArgs {
    /// What to accomplish — becomes the react recipe's instruction.
    pub goal: String,
    /// Published context-bundle handles to attach (`--context`, repeatable).
    pub context_bundles: Vec<String>,
    /// Raw 64-hex content-store refs to attach as context (`--context-ref`, repeatable).
    pub context_refs: Vec<String>,
    /// Structured inputs (`--input k=v`, repeatable) folded into the goal prompt.
    pub inputs: Vec<(String, String)>,
    /// Settle timeout in seconds.
    pub timeout_secs: u64,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `agent` args (the verb already consumed). The first token selects the
/// subcommand (only `run` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<AgentArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("agent requires a subcommand: run".into()))?;
    if kw != "run" {
        return Err(CliError::Usage(format!(
            "unknown agent subcommand {kw:?} (expected: run)"
        )));
    }
    let mut goal: Option<String> = None;
    let mut context_bundles = Vec::new();
    let mut context_refs = Vec::new();
    let mut inputs: Vec<(String, String)> = Vec::new();
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--goal" => goal = Some(next_value(&mut args, "--goal")?),
            "--context" => context_bundles.push(next_value(&mut args, "--context")?),
            "--context-ref" => context_refs.push(next_value(&mut args, "--context-ref")?),
            "--input" => {
                let kv = next_value(&mut args, "--input")?;
                let (k, v) = kv
                    .split_once('=')
                    .ok_or_else(|| CliError::Usage(format!("--input expects k=v, got {kv:?}")))?;
                inputs.push((k.to_string(), v.to_string()));
            }
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    let goal = goal.ok_or_else(|| CliError::Usage("agent run requires --goal <text>".into()))?;
    Ok(AgentArgs {
        goal,
        context_bundles,
        context_refs,
        inputs,
        timeout_secs,
        common,
    })
}

/// Fold `--input k=v` pairs into the goal prompt (no structured recipe slot yet).
fn fold_inputs(goal: &str, inputs: &[(String, String)]) -> String {
    if inputs.is_empty() {
        return goal.to_string();
    }
    let lines: Vec<String> = inputs.iter().map(|(k, v)| format!("- {k}: {v}")).collect();
    format!("{goal}\n\nInputs:\n{}", lines.join("\n"))
}

/// Execute `agent run`.
pub async fn execute(args: AgentArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let instruction = fold_inputs(&args.goal, &args.inputs);
    let req_args = serde_json::json!({
        "instruction": instruction,
        "max_turns": DEFAULT_MAX_TURNS,
        "max_tool_calls": DEFAULT_MAX_TOOL_CALLS,
    })
    .to_string()
    .into_bytes();

    let resp = client
        .invoke(resolved.request(proto::InvokeRequest {
            handle: REACT_RECIPE_HANDLE.to_string(),
            args: req_args,
            context_bundles: args.context_bundles,
            context_refs: args.context_refs,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    let outcome = wait::await_react_result(
        &mut client,
        &resolved,
        resp.instance_id.clone(),
        Duration::from_secs(args.timeout_secs),
    )
    .await?;

    // The AUDITED action set = the chain's settled `tool` turns, in turn order.
    let turns = client
        .list_react_turns(resolved.request(proto::ListReactTurnsRequest {
            limit: None,
            instance_id: Some(resp.instance_id.clone()),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner()
        .turns;
    let mut actions: Vec<(String, String, u32)> = turns
        .iter()
        .filter(|t| t.branch == "tool")
        .map(|t| (t.tool_id.clone(), t.tool_version.clone(), t.turn))
        .collect();
    actions.sort_by_key(|(_, _, turn)| *turn);

    println!(
        "{}",
        format::render_agent_result(&outcome, &actions, args.common.json)
    );

    // The exit-code contract mirrors `finish_wait`: committed → success,
    // failed → exit 1, timed out → exit 3 (resumable).
    match outcome.state {
        WaitState::Committed => Ok(()),
        WaitState::Failed => Err(CliError::Failed),
        WaitState::Running => Err(CliError::WaitTimeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<AgentArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn run_parses_goal_context_inputs_and_json() {
        let a = p(&[
            "run",
            "--goal",
            "echo pong",
            "--context",
            "team/ctx/spec",
            "--input",
            "url=http://x",
            "--input",
            "lang=en",
            "--json",
        ])
        .unwrap();
        assert_eq!(a.goal, "echo pong");
        assert_eq!(a.context_bundles, vec!["team/ctx/spec".to_string()]);
        assert_eq!(
            a.inputs,
            vec![
                ("url".to_string(), "http://x".to_string()),
                ("lang".to_string(), "en".to_string()),
            ]
        );
        assert!(a.common.json);
        assert_eq!(a.timeout_secs, DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn fold_inputs_appends_pairs_else_noop() {
        assert_eq!(fold_inputs("g", &[]), "g");
        let folded = fold_inputs("g", &[("k".into(), "v".into())]);
        assert!(folded.starts_with("g\n\nInputs:\n"));
        assert!(folded.contains("- k: v"));
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["list"]).is_err(), "unknown subcommand");
        assert!(p(&["run"]).is_err(), "missing --goal");
        assert!(p(&["run", "--goal"]).is_err(), "missing --goal value");
        assert!(p(&["run", "--goal", "g", "--input", "novalue"]).is_err());
        assert!(p(&["run", "--goal", "g", "--timeout-secs", "soon"]).is_err());
        assert!(p(&["run", "--goal", "g", "--bogus"]).is_err());
    }
}
