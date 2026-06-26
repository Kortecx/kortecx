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
use crate::{verbs, wait};

/// The vision recipe (PR-B2) — `--image` binds it (image→text / prompted OCR on a
/// vision-capable model on either engine).
const VISION_CHAT_HANDLE: &str = "kx/recipes/vision";

/// Default `--wait` timeout (matches `invoke`/`agent`).
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Default + max number of grounding documents (mirrors the server-side bound).
const DEFAULT_K: u32 = 4;
/// See [`DEFAULT_K`].
const MAX_K: u32 = 16;
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
    Ok(ChatArgs {
        message,
        dataset,
        image,
        k: k.clamp(1, MAX_K),
        timeout_secs,
        common,
    })
}

/// Execute `chat`.
pub async fn execute(args: ChatArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    // PR-B2 vision: `--image` binds `kx/recipes/vision` (image→text / OCR). Mutually
    // exclusive with `--dataset` (vision-RAG is a follow-up). HONEST-degrade: if no
    // image-capable model is served (the vision recipe form is absent / lacks the
    // image_ref slot), print a notice and answer the message plainly — never silently
    // drop the image, never fake an answer about it.
    let (handle, args_json): (String, serde_json::Value) = if let Some(path) = args.image.clone() {
        if args.dataset.is_some() {
            return Err(CliError::Usage(
                "--image and --dataset cannot be combined (vision-RAG is not yet supported)".into(),
            ));
        }
        if let Some(plan) = plan_vision(&mut client, &resolved, &path, &args.message).await? {
            plan
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

/// Plan an image-bearing chat (PR-B2): upload the image, resolve the `kx/recipes/vision`
/// form, and assemble `{ prompt, image_ref, model }`. Returns `None` when no
/// image-capable model is served (the form is absent / lacks the `image_ref` slot), so
/// the caller can honest-degrade to plain chat instead of faking an answer.
async fn plan_vision(
    client: &mut kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
    path: &std::path::Path,
    message: &str,
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

    // Resolve the vision recipe form (the SAME form-gate the SDK/console use). An
    // absent form / RPC error ⇒ honest-degrade (`None`).
    let Ok(form_resp) = client
        .get_recipe_form(resolved.request(proto::GetRecipeFormRequest {
            handle: VISION_CHAT_HANDLE.to_string(),
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
    eprintln!("· image attached — binding the vision model (image→text / OCR)");
    Ok(Some((
        VISION_CHAT_HANDLE.to_string(),
        serde_json::Value::Object(obj),
    )))
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
    fn image_and_dataset_both_parse_execute_rejects() {
        // Parse accepts both; the mutual-exclusion (vision-RAG unsupported) is enforced
        // in `execute` after connect (covered by the live e2e). This pins that parse
        // does not pre-reject the combination.
        let a = parse_v(&["hi", "--image", "/tmp/x.png", "--dataset", "docs"]).unwrap();
        assert!(a.image.is_some() && a.dataset.is_some());
    }
}
