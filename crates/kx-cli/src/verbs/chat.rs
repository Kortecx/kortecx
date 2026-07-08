//! `kx chat --message <text> [--dataset <name>] [--k <n>] ...` — POC-1 CHAT-RAG.
//!
//! An ergonomic wrapper over Invoke of `kx/recipes/chat` (plain) or
//! `kx/recipes/chat-rag` (AUTO-RAG grounding over a dataset). When `--dataset` names
//! an existing, non-empty dataset the server embeds the message, retrieves the
//! dataset's top-`k` documents, and folds the EXACT refs into the prompt (edge-free,
//! replayable, SN-8). The verb prints an HONEST grounding indicator: grounding is
//! reported ONLY when the dataset is present + non-empty — otherwise it answers
//! plainly (grounding is never faked). Always Invoke (server-warranted, SN-8); never
//! SubmitRun.

use std::path::PathBuf;
use std::time::Duration;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::verbs::blueprint::{to_request, DagSpec, StepSpec};
use crate::{verbs, wait};

/// The vision recipe (PR-B2) — `--image` binds it (image→text / prompted OCR on a
/// vision-capable model on either engine).
const VISION_CHAT_HANDLE: &str = "kx/recipes/vision";
/// RC4b VISION-RAG: `--image` + `--dataset` binds this (the served VLM answers about the
/// image WHILE grounded on the dataset's top-k retrieved text passages — one generation).
const VISION_RAG_CHAT_HANDLE: &str = "kx/recipes/vision-rag";

/// Default `--wait` timeout (matches `invoke`/`agent`).
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Default + max number of grounding documents (mirrors the server-side bound).
const DEFAULT_K: u32 = 4;
/// See [`DEFAULT_K`].
const MAX_K: u32 = 16;
/// (`--tools`): the bounded ReAct-turn budget defaults (mirror `kx agent run`).
const DEFAULT_MAX_TURNS: u32 = 8;
/// See [`DEFAULT_MAX_TURNS`].
const DEFAULT_MAX_TOOL_CALLS: u32 = 20;
/// The canonical plain-chat recipe (the served model, no grounding).
const PLAIN_CHAT_HANDLE: &str = "kx/recipes/chat";
/// The AUTO-RAG chat recipe (POC-1; grounds the turn over a dataset).
const RAG_CHAT_HANDLE: &str = "kx/recipes/chat-rag";

/// Parsed `chat` arguments.
#[derive(Debug)]
pub struct ChatArgs {
    /// The user message (the prompt + the retrieval query text when grounded).
    pub message: String,
    /// Ground the turn over this dataset (`--dataset`); `None` ⇒ a plain chat.
    pub dataset: Option<String>,
    /// PR-B2: attach an image (`--image <path>`) ⇒ bind `kx/recipes/vision`
    /// (image→text / OCR). Mutually exclusive with `--dataset`.
    pub image: Option<PathBuf>,
    /// Top-`k` grounding documents (`--k`, clamped to the server-side max).
    pub k: u32,
    /// Attach tools (`--tools <id@ver,…>`) ⇒ the turn is a BOUNDED ReAct turn with
    /// EXPLICIT per-turn grants (only these tools). Empty ⇒ a plain chat. Parsed as
    /// `(tool_id, tool_version)` pairs (order-preserving); the server builds the scoped
    /// warrant from the contract (SN-8), never the `KX_SERVE_AUTOGRANT` blanket.
    pub tools: Vec<(String, String)>,
    /// The ReAct loop turn budget (`--max-turns`, default 8).
    pub max_turns: u32,
    /// The ReAct loop tool-call budget (`--max-tool-calls`, default 20).
    pub max_tool_calls: u32,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `chat` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ChatArgs, CliError> {
    let mut message: Option<String> = None;
    let mut dataset: Option<String> = None;
    let mut image: Option<PathBuf> = None;
    let mut k = DEFAULT_K;
    let mut tools: Vec<(String, String)> = Vec::new();
    let mut max_turns = DEFAULT_MAX_TURNS;
    let mut max_tool_calls = DEFAULT_MAX_TOOL_CALLS;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--message" => message = Some(next_value(&mut args, "--message")?),
            "--dataset" => dataset = Some(next_value(&mut args, "--dataset")?),
            "--image" => image = Some(PathBuf::from(next_value(&mut args, "--image")?)),
            "--tools" => tools = parse_tools(&next_value(&mut args, "--tools")?)?,
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
            "--k" => {
                let v = next_value(&mut args, "--k")?;
                k = v
                    .parse()
                    .map_err(|_| CliError::Usage(format!("--k expects an integer, got {v:?}")))?;
            }
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            other => {
                if other.starts_with("--") {
                    return Err(CliError::Usage(format!("unknown flag {other:?}")));
                }
                // A bare positional is the message (ergonomic: `kx chat "hello"`).
                if message.is_some() {
                    return Err(CliError::Usage(format!("unexpected argument {other:?}")));
                }
                message = Some(other.to_string());
            }
        }
    }

    let message = message.ok_or_else(|| {
        CliError::Usage("chat requires a message (--message <text> or a bare positional)".into())
    })?;
    // `--tools` routes the turn through the agentic (SubmitWorkflow) path, which does
    // not compose with the vision / RAG recipe binders yet — reject the combination
    // honestly rather than silently dropping one (tool-grounded RAG chat is a follow-up).
    if !tools.is_empty() && (image.is_some() || dataset.is_some()) {
        return Err(CliError::Usage(
            "--tools cannot be combined with --image or --dataset yet (a tool-attached chat \
             turn runs the agentic path); use them separately"
                .into(),
        ));
    }
    Ok(ChatArgs {
        message,
        dataset,
        image,
        k: k.clamp(1, MAX_K),
        tools,
        max_turns,
        max_tool_calls,
        timeout_secs,
        common,
    })
}

/// Parse a `--tools <id@ver,id@ver,…>` list into `(tool_id, tool_version)` pairs. The
/// `@` separator + version-pin is the convention used everywhere (`mcp-echo@1`); a
/// missing `@`, empty part, or empty list is a usage error (a tool is always pinned).
fn parse_tools(raw: &str) -> Result<Vec<(String, String)>, CliError> {
    let tools: Vec<(String, String)> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|tok| match tok.rsplit_once('@') {
            Some((id, ver)) if !id.is_empty() && !ver.is_empty() => {
                Ok((id.to_string(), ver.to_string()))
            }
            _ => Err(CliError::Usage(format!(
                "--tools expects a comma-separated <tool_id>@<tool_version> list \
                 (e.g. mcp-echo@1,calc@1), got {tok:?}"
            ))),
        })
        .collect::<Result<_, _>>()?;
    if tools.is_empty() {
        return Err(CliError::Usage(
            "--tools requires at least one <tool_id>@<tool_version>".into(),
        ));
    }
    Ok(tools)
}

/// Execute `chat`.
pub async fn execute(args: ChatArgs) -> Result<(), CliError> {
    // `--tools` makes the turn a BOUNDED ReAct turn (the agentic SubmitWorkflow path
    // with EXPLICIT per-turn grants), not a plain-chat recipe Invoke.
    if !args.tools.is_empty() {
        return execute_agentic(args).await;
    }
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    // PR-B2 vision: `--image` binds `kx/recipes/vision` (image→text / OCR). Mutually
    // exclusive with `--dataset` (vision-RAG is a follow-up). HONEST-degrade: if no
    // image-capable model is served (the vision recipe form is absent / lacks the
    // image_ref slot), print a notice and answer the message plainly — never silently
    // drop the image, never fake an answer about it.
    let (handle, args_json): (String, serde_json::Value) = if let Some(path) = args.image.clone() {
        // RC4b: `--image` + `--dataset` binds `kx/recipes/vision-rag` (the VLM answers about
        // the image WHILE grounded on the dataset's retrieved text). HONEST-degrade ladder:
        // vision-rag → plain vision (image only) → plain chat — never silently drop the image
        // or fake grounding (GR15).
        let rag = args.dataset.as_deref().map(|d| (d, args.k));
        if let Some(plan) = plan_vision(&mut client, &resolved, &path, &args.message, rag).await? {
            plan
        } else if rag.is_some() {
            // vision-rag not provisioned — try plain vision (image only), then plain chat.
            if let Some(plan) =
                plan_vision(&mut client, &resolved, &path, &args.message, None).await?
            {
                eprintln!(
                    "· vision-RAG not available — answering about the image without dataset grounding"
                );
                plan
            } else {
                eprintln!(
                    "· image + dataset attached but no image-capable model is served — answering plainly"
                );
                (
                    PLAIN_CHAT_HANDLE.to_string(),
                    serde_json::json!({ "prompt": args.message }),
                )
            }
        } else {
            eprintln!(
                "· image attached but no image-capable model is served — answering without it"
            );
            (
                PLAIN_CHAT_HANDLE.to_string(),
                serde_json::json!({ "prompt": args.message }),
            )
        }
    } else {
        plan_text(&mut client, &resolved, &args).await?
    };

    let args_bytes =
        serde_json::to_vec(&args_json).map_err(|e| CliError::Io(format!("chat args: {e}")))?;

    let resp = client
        .invoke(resolved.request(proto::InvokeRequest {
            handle: handle.clone(),
            args: args_bytes,
            context_bundles: Vec::new(),
            context_refs: Vec::new(),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    let outcome = wait::await_result(
        &mut client,
        &resolved,
        resp.instance_id,
        resp.terminal_mote_id,
        Duration::from_secs(args.timeout_secs),
    )
    .await?;
    verbs::finish_wait(&outcome, args.common.json, None)
}

/// Build the `SubmitWorkflowRequest` for a `--tools` chat turn: ONE MODEL step whose
/// `tool_contract` names ONLY the attached tools, plus the bounded ReAct budget. The
/// server builds the scoped warrant FROM the contract and re-verifies each grant against
/// its live registry (SN-8) — never a client warrant, never the autogrant blanket. Split
/// out so the lowering (the tool_contract + budget reaching the wire) is unit-testable
/// without a gateway.
fn build_agentic_request(args: &ChatArgs) -> Result<proto::SubmitWorkflowRequest, CliError> {
    let tool_contract: serde_json::Map<String, serde_json::Value> = args
        .tools
        .iter()
        .map(|(id, ver)| (id.clone(), serde_json::Value::String(ver.clone())))
        .collect();
    let step: StepSpec = serde_json::from_value(serde_json::json!({
        "kind": "model",
        "prompt": args.message,
        "tool_contract": tool_contract,
        "max_turns": args.max_turns,
        "max_tool_calls": args.max_tool_calls,
    }))
    .map_err(|e| CliError::Io(format!("chat --tools step: {e}")))?;
    let spec = DagSpec {
        seed: 0,
        steps: vec![step],
        edges: Vec::new(),
        execution_mode: Some("frozen".to_string()),
        context_bundles: Vec::new(),
    };
    Ok(to_request(spec)?)
}

/// Execute a `--tools` chat turn: submit the single-step agentic workflow, then wait for
/// the ReAct chain to settle — scoped by the server-returned `react_chain_salt` so a
/// shared-journal serve surfaces THIS turn's answer, not a stale/foreign committed Mote.
async fn execute_agentic(args: ChatArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let req = build_agentic_request(&args)?;
    let handle = client
        .submit_workflow(resolved.request(req)?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    let outcome = wait::await_react_result(
        &mut client,
        &resolved,
        handle.instance_id,
        handle.react_chain_salt,
        Duration::from_secs(args.timeout_secs),
    )
    .await?;
    verbs::finish_wait(&outcome, args.common.json, None)
}

/// Plan the plain / AUTO-RAG chat (the pre-PR-B2 path), with the honest grounding
/// indicator.
async fn plan_text(
    client: &mut kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
    args: &ChatArgs,
) -> Result<(String, serde_json::Value), CliError> {
    // HONEST grounding indicator: report grounding ONLY when the named dataset
    // exists + is non-empty (best-effort — a datasets-less serve or an RPC error
    // simply degrades to plain chat). The server independently degrades too, so the
    // indicator and the actual recipe always agree.
    let grounded = if let Some(name) = args.dataset.as_deref() {
        let ready = client
            .list_datasets(resolved.request(proto::ListDatasetsRequest {})?)
            .await
            .ok()
            .is_some_and(|resp| {
                resp.into_inner()
                    .datasets
                    .iter()
                    .any(|d| (d.name == name || d.dataset_id == name) && d.doc_count > 0)
            });
        if ready {
            eprintln!("· grounding on dataset '{name}' (top-{})", args.k);
        } else {
            eprintln!("· dataset '{name}' not found or empty — answering without grounding");
        }
        ready
    } else {
        false
    };

    if grounded {
        let name = args.dataset.as_deref().unwrap_or_default().to_string();
        Ok((
            RAG_CHAT_HANDLE.to_string(),
            serde_json::json!({ "prompt": args.message, "dataset": name, "k": args.k }),
        ))
    } else {
        Ok((
            PLAIN_CHAT_HANDLE.to_string(),
            serde_json::json!({ "prompt": args.message }),
        ))
    }
}

/// Plan an image-bearing chat: upload the image, resolve the recipe form, and assemble
/// `{ prompt, image_ref, model }` (PR-B2). When `rag = Some((dataset, k))` it targets
/// `kx/recipes/vision-rag` and ALSO passes `{ dataset, k }` (stripped + folded server-side
/// into the retrieved-text context) — the VLM answers about the image grounded on the
/// dataset (RC4b). Returns `None` when the targeted recipe is not provisioned (the form is
/// absent / lacks the `image_ref` slot), so the caller can honest-degrade.
async fn plan_vision(
    client: &mut kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
    path: &std::path::Path,
    message: &str,
    rag: Option<(&str, u32)>,
) -> Result<Option<(String, serde_json::Value)>, CliError> {
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
    let image_ref = crate::hex::encode(&put.content_ref);

    // Resolve the targeted recipe form (vision-rag when grounding, else vision) — the SAME
    // form-gate the SDK/console use. An absent form / RPC error ⇒ honest-degrade (`None`).
    let handle = if rag.is_some() {
        VISION_RAG_CHAT_HANDLE
    } else {
        VISION_CHAT_HANDLE
    };
    let Ok(form_resp) = client
        .get_recipe_form(resolved.request(proto::GetRecipeFormRequest {
            handle: handle.to_string(),
        })?)
        .await
    else {
        return Ok(None);
    };
    let form = form_resp.into_inner();
    let has = |n: &str| form.fields.iter().find(|f| f.name == n);
    if has("image_ref").is_none() {
        return Ok(None);
    }
    let mut obj = serde_json::Map::new();
    obj.insert("image_ref".to_string(), serde_json::json!(image_ref));
    if has("prompt").is_some() {
        obj.insert("prompt".to_string(), serde_json::json!(message));
    }
    if let Some(model) = has("model") {
        // The server validates ENUM membership; pre-pick the first legal value (the
        // CLI has no default-model preference here — the server route is what binds).
        let chosen = model.allowed.first().cloned().unwrap_or_default();
        obj.insert("model".to_string(), serde_json::json!(chosen));
    }
    if let Some((dataset, k)) = rag {
        // dataset/k are NOT declared slots — the server strips them and folds the
        // retrieved text into the prompt (the chat-rag grounding path; SN-8 exact refs).
        obj.insert("dataset".to_string(), serde_json::json!(dataset));
        obj.insert("k".to_string(), serde_json::json!(k));
        eprintln!("· image + dataset '{dataset}' — grounding the vision answer on retrieved text");
    } else {
        eprintln!("· image attached — binding the vision model (image→text / OCR)");
    }
    Ok(Some((handle.to_string(), serde_json::Value::Object(obj))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_v(parts: &[&str]) -> Result<ChatArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_message_dataset_and_k() {
        let a = parse_v(&["--message", "hi there", "--dataset", "docs", "--k", "6"]).unwrap();
        assert_eq!(a.message, "hi there");
        assert_eq!(a.dataset.as_deref(), Some("docs"));
        assert_eq!(a.k, 6);
    }

    #[test]
    fn a_bare_positional_is_the_message() {
        let a = parse_v(&["what is kortecx?"]).unwrap();
        assert_eq!(a.message, "what is kortecx?");
        assert!(a.dataset.is_none(), "plain chat without a dataset");
    }

    #[test]
    fn k_is_clamped_to_the_server_bound() {
        assert_eq!(parse_v(&["hi", "--k", "999"]).unwrap().k, MAX_K);
        assert_eq!(parse_v(&["hi", "--k", "0"]).unwrap().k, 1);
    }

    #[test]
    fn message_is_required() {
        assert!(parse_v(&["--dataset", "docs"]).is_err(), "no message");
    }

    #[test]
    fn unknown_flag_and_second_positional_are_usage() {
        assert!(parse_v(&["hi", "--nope"]).is_err());
        assert!(parse_v(&["hi", "again"]).is_err(), "two messages");
    }

    #[test]
    fn bad_k_is_usage() {
        assert!(parse_v(&["hi", "--k", "lots"]).is_err());
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = parse_v(&["hi", "--endpoint", "http://h:1", "--json"]).unwrap();
        assert_eq!(a.common.endpoint, "http://h:1");
        assert!(a.common.json);
    }

    #[test]
    fn parses_image_path() {
        let a = parse_v(&["what is in this?", "--image", "/tmp/cat.png"]).unwrap();
        assert_eq!(a.message, "what is in this?");
        assert_eq!(
            a.image.as_deref(),
            Some(std::path::Path::new("/tmp/cat.png"))
        );
        assert!(a.dataset.is_none());
    }

    #[test]
    fn image_and_dataset_both_parse_and_route_to_vision_rag() {
        // RC4b: `--image` + `--dataset` is now SUPPORTED — it parses and (in `execute`,
        // covered by the live e2e) binds `kx/recipes/vision-rag` with an honest-degrade
        // ladder (vision-rag → plain vision → plain chat). This pins that parse accepts
        // the combination (no pre-rejection).
        let a = parse_v(&["hi", "--image", "/tmp/x.png", "--dataset", "docs"]).unwrap();
        assert!(a.image.is_some() && a.dataset.is_some());
    }

    // ---- `--tools` (a bounded ReAct chat turn) ----

    #[test]
    fn parses_tools_and_react_budget() {
        let a = parse_v(&[
            "hi",
            "--tools",
            "mcp-echo@1, calc@2",
            "--max-turns",
            "5",
            "--max-tool-calls",
            "3",
        ])
        .unwrap();
        assert_eq!(
            a.tools,
            vec![
                ("mcp-echo".to_string(), "1".to_string()),
                ("calc".to_string(), "2".to_string()),
            ]
        );
        assert_eq!(a.max_turns, 5);
        assert_eq!(a.max_tool_calls, 3);
    }

    #[test]
    fn tools_default_react_budget_and_plain_chat_has_none() {
        let a = parse_v(&["hi", "--tools", "mcp-echo@1"]).unwrap();
        assert_eq!(a.max_turns, DEFAULT_MAX_TURNS);
        assert_eq!(a.max_tool_calls, DEFAULT_MAX_TOOL_CALLS);
        assert!(
            parse_v(&["hi"]).unwrap().tools.is_empty(),
            "plain chat has no tools"
        );
    }

    #[test]
    fn bad_tool_ref_is_usage() {
        assert!(
            parse_v(&["hi", "--tools", "echo"]).is_err(),
            "missing @version"
        );
        assert!(parse_v(&["hi", "--tools", "@1"]).is_err(), "empty id");
        assert!(
            parse_v(&["hi", "--tools", "echo@"]).is_err(),
            "empty version"
        );
        assert!(parse_v(&["hi", "--tools", ""]).is_err(), "empty list");
        assert!(parse_v(&["hi", "--max-turns", "lots"]).is_err(), "bad int");
    }

    #[test]
    fn tools_rejects_image_and_dataset_combos() {
        assert!(parse_v(&["hi", "--tools", "echo@1", "--image", "/tmp/x.png"]).is_err());
        assert!(parse_v(&["hi", "--tools", "echo@1", "--dataset", "docs"]).is_err());
    }

    #[test]
    fn tools_lower_to_a_single_agentic_model_step() {
        // SN-4 integration plumbing: the attached tools + budget reach the WorkflowStep the
        // gateway compiles — a MODEL step with EXACTLY the named tool_contract + the react
        // max_turns/max_tool_calls params. The server builds the scoped warrant from this
        // contract (SN-8); the CLI never sends a warrant.
        let a = parse_v(&[
            "Use echo to say hi",
            "--tools",
            "mcp-echo@1",
            "--max-turns",
            "4",
            "--max-tool-calls",
            "2",
        ])
        .unwrap();
        let req = build_agentic_request(&a).unwrap();
        assert_eq!(req.steps.len(), 1);
        assert!(req.edges.is_empty(), "a single-step turn has no edges");
        let step = &req.steps[0];
        assert_eq!(step.kind, proto::WorkflowStepKind::Model as i32);
        assert_eq!(step.prompt, "Use echo to say hi");
        assert_eq!(
            step.tool_contract.get("mcp-echo").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            step.tool_contract.len(),
            1,
            "ONLY the named tool is granted"
        );
        assert_eq!(
            step.params.get("max_turns").map(Vec::as_slice),
            Some(b"4".as_slice())
        );
        assert_eq!(
            step.params.get("max_tool_calls").map(Vec::as_slice),
            Some(b"2".as_slice())
        );
    }
}
