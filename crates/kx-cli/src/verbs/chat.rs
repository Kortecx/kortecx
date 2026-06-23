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

use std::time::Duration;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{verbs, wait};

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
        k: k.clamp(1, MAX_K),
        timeout_secs,
        common,
    })
}

/// Execute `chat`.
pub async fn execute(args: ChatArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

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

    let (handle, args_json) = if grounded {
        let name = args.dataset.as_deref().unwrap_or_default();
        (
            RAG_CHAT_HANDLE,
            serde_json::json!({ "prompt": args.message, "dataset": name, "k": args.k }),
        )
    } else {
        (
            PLAIN_CHAT_HANDLE,
            serde_json::json!({ "prompt": args.message }),
        )
    };
    let args_bytes =
        serde_json::to_vec(&args_json).map_err(|e| CliError::Io(format!("chat args: {e}")))?;

    let resp = client
        .invoke(resolved.request(proto::InvokeRequest {
            handle: handle.to_string(),
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
}
