//! `kx memory` — the durable agentic MEMORY surface (RC5a: `StoreMemory` /
//! `ListMemories` / `RecallMemory` / `ForgetMemory`).
//!
//! - `kx memory add <text> [--kind semantic|episodic] [--json]` — remember a fact.
//!   The CLI uses the SERVER-EMBED path, so it needs `kx serve --features
//!   inference,hnsw` with a model AND `KX_SERVE_MEMORY=1`; without one the gateway
//!   answers `FAILED_PRECONDITION` / `Unimplemented` honestly.
//! - `kx memory list [--instance <hex16>] [--limit N] [--json]` — the episodic log,
//!   newest-first, optionally scoped to one run.
//! - `kx memory recall --text <query> [--k N] [--json]` — the top-k most-similar
//!   memories. Each hit's `score` is DISPLAY-ONLY (SN-8) — a ranking aid, never an
//!   identity input; the durable result is the ordered content-ref SET.
//! - `kx memory forget <memory_id_hex> [--json]` — erase a memory by its content id.
//!
//! Every memory is scoped to the caller's own principal (server-derived) — a client
//! never reaches another principal's memories. A gateway without memory enabled
//! answers `Unimplemented`, rendered honestly.

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The default top-k when `--k` is omitted (the server clamps to its own max).
const DEFAULT_K: u32 = 5;

/// Parsed `memory` arguments.
#[derive(Debug)]
pub enum MemoryArgs {
    /// `memory add <text> …`.
    Add(AddArgs),
    /// `memory list …`.
    List(ListArgs),
    /// `memory recall --text …`.
    Recall(RecallArgs),
    /// `memory forget <memory_id>`.
    Forget(ForgetArgs),
}

/// Parsed `memory add` arguments.
#[derive(Debug)]
pub struct AddArgs {
    /// The fact to remember.
    pub content: String,
    /// The wire kind value (0 = unspecified ⇒ semantic, 1 = semantic, 2 = episodic).
    pub kind: i32,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory list` arguments.
#[derive(Debug)]
pub struct ListArgs {
    /// Optional 16-byte run filter (32 hex chars).
    pub instance_id: Option<Vec<u8>>,
    /// Optional page size (absent ⇒ the server default).
    pub limit: Option<u32>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory recall` arguments.
#[derive(Debug)]
pub struct RecallArgs {
    /// The query text (server-embedded).
    pub text: String,
    /// Top-k (absent ⇒ `DEFAULT_K`; the server clamps).
    pub k: Option<u32>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory forget` arguments.
#[derive(Debug)]
pub struct ForgetArgs {
    /// The 32-byte memory id (64 hex chars).
    pub memory_id: Vec<u8>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Decode a hex string to exactly `want_bytes` bytes, fail-closed on a bad length /
/// non-hex digit (so a malformed id is a usage error, never silently coerced).
fn parse_hex(s: &str, want_bytes: usize, what: &str) -> Result<Vec<u8>, CliError> {
    let bytes =
        crate::hex::decode(s).map_err(|_| CliError::Usage(format!("{what} is not valid hex")))?;
    if bytes.len() != want_bytes {
        return Err(CliError::Usage(format!(
            "{what} must be {} hex chars, got {}",
            want_bytes * 2,
            s.len()
        )));
    }
    Ok(bytes)
}

/// Parse `memory` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<MemoryArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("memory requires a subcommand: add | list | recall | forget".into())
    })?;
    match kw.as_str() {
        "add" => parse_add(args).map(MemoryArgs::Add),
        "list" => parse_list(args).map(MemoryArgs::List),
        "recall" => parse_recall(args).map(MemoryArgs::Recall),
        "forget" => parse_forget(args).map(MemoryArgs::Forget),
        other => Err(CliError::Usage(format!(
            "unknown memory subcommand {other:?} (expected: add | list | recall | forget)"
        ))),
    }
}

fn parse_add(mut args: impl Iterator<Item = String>) -> Result<AddArgs, CliError> {
    let mut content: Option<String> = None;
    let mut kind = proto::MemoryKind::Unspecified as i32;
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--kind" => {
                let v = next_value(&mut args, "--kind")?;
                kind = match v.to_ascii_lowercase().as_str() {
                    "semantic" => proto::MemoryKind::Semantic as i32,
                    "episodic" => proto::MemoryKind::Episodic as i32,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--kind must be semantic|episodic, got {other:?}"
                        )))
                    }
                };
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            _ if content.is_none() => content = Some(tok),
            _ => {
                return Err(CliError::Usage(
                    "memory add takes exactly one <text> argument".into(),
                ))
            }
        }
    }
    let content = content
        .ok_or_else(|| CliError::Usage("memory add requires a <text> to remember".into()))?;
    Ok(AddArgs {
        content,
        kind,
        common,
    })
}

fn parse_list(mut args: impl Iterator<Item = String>) -> Result<ListArgs, CliError> {
    let mut instance_id: Option<Vec<u8>> = None;
    let mut limit: Option<u32> = None;
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--instance" => {
                let v = next_value(&mut args, "--instance")?;
                instance_id = Some(parse_hex(&v, 16, "--instance")?);
            }
            "--limit" => {
                let v = next_value(&mut args, "--limit")?;
                limit = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--limit must be a positive integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(ListArgs {
        instance_id,
        limit,
        common,
    })
}

fn parse_recall(mut args: impl Iterator<Item = String>) -> Result<RecallArgs, CliError> {
    let mut text: Option<String> = None;
    let mut k: Option<u32> = None;
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--text" => text = Some(next_value(&mut args, "--text")?),
            "--k" => {
                let v = next_value(&mut args, "--k")?;
                k = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--k must be a positive integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    let text =
        text.ok_or_else(|| CliError::Usage("memory recall requires --text <query>".into()))?;
    Ok(RecallArgs { text, k, common })
}

fn parse_forget(mut args: impl Iterator<Item = String>) -> Result<ForgetArgs, CliError> {
    let mut memory_id: Option<Vec<u8>> = None;
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            _ if memory_id.is_none() => memory_id = Some(parse_hex(&tok, 32, "memory_id")?),
            _ => {
                return Err(CliError::Usage(
                    "memory forget takes exactly one <memory_id> argument".into(),
                ))
            }
        }
    }
    let memory_id = memory_id
        .ok_or_else(|| CliError::Usage("memory forget requires a <memory_id> (64 hex)".into()))?;
    Ok(ForgetArgs { memory_id, common })
}

/// Map a memory RPC status to an honest CLI error — a gateway without memory enabled
/// answers `Unimplemented`.
fn map_memory_status(status: tonic::Status) -> CliError {
    if status.code() == Code::Unimplemented {
        CliError::Rpc {
            code: Code::Unimplemented,
            message: "memory is not wired on this gateway (run `kx serve --features \
                      inference,hnsw` with `KX_SERVE_MEMORY=1`)"
                .into(),
            refusal_code: None,
        }
    } else {
        CliError::from_status(status)
    }
}

/// Execute `memory`.
pub async fn execute(args: MemoryArgs) -> Result<(), CliError> {
    match args {
        MemoryArgs::Add(a) => execute_add(a).await,
        MemoryArgs::List(a) => execute_list(a).await,
        MemoryArgs::Recall(a) => execute_recall(a).await,
        MemoryArgs::Forget(a) => execute_forget(a).await,
    }
}

async fn execute_add(args: AddArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .store_memory(resolved.request(proto::StoreMemoryRequest {
            content: args.content.into_bytes(),
            embedding: Vec::new(), // server-embed
            kind: args.kind,
            namespace: String::new(), // server-derived from the caller principal
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_store_memory(&resp, args.common.json));
    Ok(())
}

async fn execute_list(args: ListArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_memories(resolved.request(proto::ListMemoriesRequest {
            limit: args.limit,
            instance_id: args.instance_id,
            namespace: String::new(),
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_memories(&resp, args.common.json));
    Ok(())
}

async fn execute_recall(args: RecallArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .recall_memory(resolved.request(proto::RecallMemoryRequest {
            query_text: args.text.clone(),
            query_embedding: Vec::new(),
            k: args.k.unwrap_or(DEFAULT_K),
            namespace: String::new(),
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_memory_hits(&resp, args.common.json));
    Ok(())
}

async fn execute_forget(args: ForgetArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .forget_memory(resolved.request(proto::ForgetMemoryRequest {
            memory_id: args.memory_id,
            namespace: String::new(),
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_forget_memory(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<MemoryArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn add_parses_text_and_kind() {
        let MemoryArgs::Add(a) = p(&["add", "the deadline is march 3rd"]).unwrap() else {
            panic!("expected add")
        };
        assert_eq!(a.content, "the deadline is march 3rd");
        assert_eq!(a.kind, proto::MemoryKind::Unspecified as i32);
        let MemoryArgs::Add(b) =
            p(&["add", "an event happened", "--kind", "episodic", "--json"]).unwrap()
        else {
            panic!("expected add")
        };
        assert_eq!(b.kind, proto::MemoryKind::Episodic as i32);
        assert!(b.common.json);
        assert!(p(&["add", "x", "--kind", "bogus"]).is_err());
    }

    #[test]
    fn list_parses_instance_and_limit() {
        let MemoryArgs::List(a) = p(&["list"]).unwrap() else {
            panic!("expected list")
        };
        assert!(a.instance_id.is_none() && a.limit.is_none());
        let hex16 = "0".repeat(32);
        let MemoryArgs::List(b) = p(&["list", "--instance", &hex16, "--limit", "10"]).unwrap()
        else {
            panic!("expected list")
        };
        assert_eq!(b.instance_id.unwrap().len(), 16);
        assert_eq!(b.limit, Some(10));
        // a bad-length instance is a usage error.
        assert!(p(&["list", "--instance", "abcd"]).is_err());
    }

    #[test]
    fn recall_parses_text_and_k() {
        let MemoryArgs::Recall(a) = p(&["recall", "--text", "deadline", "--k", "3"]).unwrap()
        else {
            panic!("expected recall")
        };
        assert_eq!(a.text, "deadline");
        assert_eq!(a.k, Some(3));
        assert!(p(&["recall"]).is_err(), "recall needs --text");
    }

    #[test]
    fn forget_parses_a_64_hex_id() {
        let hex32 = "a".repeat(64);
        let MemoryArgs::Forget(a) = p(&["forget", &hex32]).unwrap() else {
            panic!("expected forget")
        };
        assert_eq!(a.memory_id.len(), 32);
        assert!(p(&["forget", "short"]).is_err(), "bad hex length");
        assert!(p(&["forget"]).is_err(), "forget needs an id");
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(p(&["add"]).is_err(), "add needs text");
    }
}
