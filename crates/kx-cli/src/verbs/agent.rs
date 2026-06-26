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
/// AGENTIC-VISION: the image-grounded ReAct recipe — bound (form-gated) when `--image`
/// is supplied so the served VLM reasons over the attached image on every turn.
const REACT_VISION_RECIPE_HANDLE: &str = "kx/recipes/react-vision";
/// The recipe's anchored bounded-loop budget (mirrors the SDK + the UI's planReactArgs).
const DEFAULT_MAX_TURNS: u32 = 8;
// T-MULTI-ELEMENT-TOOLCALLS: the default tool-call cap rose 6 → 20 (decoupled from
// max_turns — a turn can now fire N tools). Overridable per run via `--max-tool-calls`.
const DEFAULT_MAX_TOOL_CALLS: u32 = 20;

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
    /// Max model turns (`--max-turns`; default 8, ceiling 8).
    pub max_turns: u32,
    /// Max total tool calls (`--max-tool-calls`; default 20, ceiling 20). A turn may
    /// fire N tools at once (T-MULTI-ELEMENT-TOOLCALLS), so this is independent of turns.
    pub max_tool_calls: u32,
    /// AGENTIC-VISION: an image to ground the agentic run (`--image <path>`). When set,
    /// the run binds `kx/recipes/react-vision` (form-gated) so the served VLM reasons over
    /// the image on EVERY turn; fail-closed (a usage error) when no vision model is served.
    pub image: Option<String>,
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
    let mut max_turns = DEFAULT_MAX_TURNS;
    let mut max_tool_calls = DEFAULT_MAX_TOOL_CALLS;
    let mut image: Option<String> = None;
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
            "--max-turns" => {
                let v = next_value(&mut args, "--max-turns")?;
                max_turns = v.parse().map_err(|_| {
                    CliError::Usage(format!("--max-turns expects an integer, got {v:?}"))
                })?;
            }
            "--max-tool-calls" => {
                let v = next_value(&mut args, "--max-tool-calls")?;
                max_tool_calls = v.parse().map_err(|_| {
                    CliError::Usage(format!("--max-tool-calls expects an integer, got {v:?}"))
                })?;
            }
            "--image" => image = Some(next_value(&mut args, "--image")?),
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
        max_turns,
        max_tool_calls,
        image,
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

/// AGENTIC-VISION: upload an image file to the content store, returning its 64-hex ref
/// (the SAME `PutContent` path `kx chat --image` uses — one upload mechanism, no drift).
async fn upload_image(
    client: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
    path: &std::path::Path,
) -> Result<String, CliError> {
    let payload =
        std::fs::read(path).map_err(|e| CliError::Io(format!("read {}: {e}", path.display())))?;
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let put = client
        .put_content(resolved.request(proto::PutContentRequest {
            payload,
            media_type: String::new(),
            filename,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    Ok(crate::hex::encode(&put.content_ref))
}

/// Execute `agent run`.
pub async fn execute(args: AgentArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let instruction = fold_inputs(&args.goal, &args.inputs);
    let mut obj = serde_json::Map::new();
    obj.insert("instruction".to_string(), serde_json::json!(instruction));
    obj.insert("max_turns".to_string(), serde_json::json!(args.max_turns));
    obj.insert(
        "max_tool_calls".to_string(),
        serde_json::json!(args.max_tool_calls),
    );

    // AGENTIC-VISION: `--image` binds the image-grounded ReAct recipe (form-gated) so the
    // served VLM reasons over the attached image on EVERY turn of the chain. Fail-closed
    // (a usage error) when no vision model is served — never silently run text-only and
    // drop the image (that would be a lie; GR15).
    let handle = if let Some(path) = &args.image {
        let image_ref = upload_image(&mut client, &resolved, std::path::Path::new(path)).await?;
        let form = client
            .get_recipe_form(resolved.request(proto::GetRecipeFormRequest {
                handle: REACT_VISION_RECIPE_HANDLE.to_string(),
            })?)
            .await
            .ok()
            .map(tonic::Response::into_inner);
        let has_image_slot = form
            .as_ref()
            .is_some_and(|f| f.fields.iter().any(|x| x.name == "image_ref"));
        if !has_image_slot {
            return Err(CliError::Usage(
                "no vision model is served — `kx agent run --image` needs an image-capable \
                 model (set KX_SERVE_MMPROJ_GGUF for llama.cpp, or serve a vision model via Ollama)"
                    .into(),
            ));
        }
        obj.insert("image_ref".to_string(), serde_json::json!(image_ref));
        eprintln!(
            "· image attached — binding the vision agent (reasons over the image every turn)"
        );
        REACT_VISION_RECIPE_HANDLE
    } else {
        REACT_RECIPE_HANDLE
    };
    let req_args = serde_json::Value::Object(obj).to_string().into_bytes();

    let resp = client
        .invoke(resolved.request(proto::InvokeRequest {
            handle: handle.to_string(),
            args: req_args,
            context_bundles: args.context_bundles,
            context_refs: args.context_refs,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    // PR-R1: serve shares ONE journal/instance_id across every Invoke, so scope the
    // settle poll + the action fetch to THIS invocation's chain via the per-invocation
    // `react_chain_salt` (32B; EMPTY ⇒ fall back to instance_id-only scoping).
    let chain_salt = resp.react_chain_salt.clone();
    let step_salt = (!chain_salt.is_empty()).then(|| chain_salt.clone());
    let outcome = wait::await_react_result(
        &mut client,
        &resolved,
        resp.instance_id.clone(),
        chain_salt,
        Duration::from_secs(args.timeout_secs),
    )
    .await?;

    // The AUDITED action set = the chain's settled `tool` turns, in turn order.
    let turns = client
        .list_react_turns(resolved.request(proto::ListReactTurnsRequest {
            limit: None,
            instance_id: Some(resp.instance_id.clone()),
            step_salt,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner()
        .turns;
    // T-MULTI-ELEMENT-TOOLCALLS: a multi-call turn fans into N "tool" rows (one per
    // call_index), so a single turn can contribute several actions — list them ALL,
    // ordered by (turn, call_index) so a parallel-tool turn reads N.0, N.1, ….
    let mut actions: Vec<(String, String, u32, u32)> = turns
        .iter()
        .filter(|t| t.branch == "tool")
        .map(|t| {
            (
                t.tool_id.clone(),
                t.tool_version.clone(),
                t.turn,
                t.call_index,
            )
        })
        .collect();
    actions.sort_by_key(|(_, _, turn, call_index)| (*turn, *call_index));

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
