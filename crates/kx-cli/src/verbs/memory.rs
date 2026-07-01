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
//! RC5b adds the ADAPTIVE surface:
//! - `kx memory decay [--dry-run|--apply] [--ttl-days N] [--min-access N] [--json]` —
//!   preview (default) or apply a reversible TTL+salience decay sweep (evictions are
//!   soft-tombstones; the row is never deleted — restorable).
//! - `kx memory stats [--json]` — namespace counts (by kind), tombstoned, dim, age range.
//! - `kx memory restore <memory_id_hex> [--json]` — un-decay a soft-tombstoned memory.
//! - `kx memory consolidate [--query q] [--k N] [--window-hours H] [--dry-run|--apply]` —
//!   preview (default; model-free) or DRIVE a react-memory chain that distills recent
//!   episodic memories into ONE durable semantic fact (`--apply`).
//!
//! Every memory is scoped to the caller's own principal (server-derived) — a client
//! never reaches another principal's memories. A gateway without memory enabled
//! answers `Unimplemented`, rendered honestly.

use std::time::Duration;

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, verbs, wait};

/// The default top-k when `--k` is omitted (the server clamps to its own max).
const DEFAULT_K: u32 = 5;
/// The react-memory recipe the consolidation trigger drives (RC5b `--apply`).
const REACT_MEMORY_HANDLE: &str = "kx/recipes/react-memory";
/// The default number of episodic memories the consolidation bundle previews / distills.
const DEFAULT_CONSOLIDATE_K: u32 = 16;
/// The default decay age threshold (mirrors the server default).
const DEFAULT_TTL_DAYS: u32 = 90;
/// The default decay salience floor (a memory recalled >= this is protected).
const DEFAULT_MIN_ACCESS: u32 = 1;
/// The default `--apply` consolidation-chain wait timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

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
    /// `memory decay …` (RC5b).
    Decay(DecayArgs),
    /// `memory stats` (RC5b).
    Stats(StatsArgs),
    /// `memory restore <memory_id>` (RC5b).
    Restore(RestoreArgs),
    /// `memory consolidate …` (RC5b).
    Consolidate(ConsolidateArgs),
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
    /// RC5b: surface decayed (tombstoned) memories too (the decayed view).
    pub include_tombstoned: bool,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory decay` arguments (RC5b).
#[derive(Debug)]
pub struct DecayArgs {
    /// Age threshold in days (a memory older AND under-recalled is a candidate).
    pub ttl_days: u32,
    /// Salience floor — a memory recalled >= this many times is protected.
    pub min_access: u32,
    /// `true` = preview only (the DEFAULT); `--apply` flips it to evict.
    pub dry_run: bool,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory stats` arguments (RC5b).
#[derive(Debug)]
pub struct StatsArgs {
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory restore` arguments (RC5b).
#[derive(Debug)]
pub struct RestoreArgs {
    /// The 32-byte memory id (64 hex chars).
    pub memory_id: Vec<u8>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `memory consolidate` arguments (RC5b).
#[derive(Debug)]
pub struct ConsolidateArgs {
    /// Optional semantic focus for the bundle.
    pub query: Option<String>,
    /// Number of episodic memories to bundle / preview.
    pub k: u32,
    /// Optional recency window in hours.
    pub window_hours: Option<u32>,
    /// `true` = preview only (the DEFAULT; model-free); `--apply` drives a react chain.
    pub dry_run: bool,
    /// The `--apply` chain wait timeout (seconds).
    pub timeout_secs: u64,
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
        CliError::Usage(
            "memory requires a subcommand: add | list | recall | forget | decay | stats | \
             restore | consolidate"
                .into(),
        )
    })?;
    match kw.as_str() {
        "add" => parse_add(args).map(MemoryArgs::Add),
        "list" => parse_list(args).map(MemoryArgs::List),
        "recall" => parse_recall(args).map(MemoryArgs::Recall),
        "forget" => parse_forget(args).map(MemoryArgs::Forget),
        "decay" => parse_decay(args).map(MemoryArgs::Decay),
        "stats" => parse_stats(args).map(MemoryArgs::Stats),
        "restore" => parse_restore(args).map(MemoryArgs::Restore),
        "consolidate" => parse_consolidate(args).map(MemoryArgs::Consolidate),
        other => Err(CliError::Usage(format!(
            "unknown memory subcommand {other:?} (expected: add | list | recall | forget | \
             decay | stats | restore | consolidate)"
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
    let mut include_tombstoned = false;
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
            "--include-tombstoned" => include_tombstoned = true,
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(ListArgs {
        instance_id,
        limit,
        include_tombstoned,
        common,
    })
}

fn parse_decay(mut args: impl Iterator<Item = String>) -> Result<DecayArgs, CliError> {
    let mut ttl_days = DEFAULT_TTL_DAYS;
    let mut min_access = DEFAULT_MIN_ACCESS;
    // Decay defaults to a PREVIEW (dry run) — evicting nothing until `--apply`.
    let mut dry_run = true;
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--ttl-days" => {
                let v = next_value(&mut args, "--ttl-days")?;
                ttl_days = v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--ttl-days must be a positive integer, got {v:?}"))
                })?;
            }
            "--min-access" => {
                let v = next_value(&mut args, "--min-access")?;
                min_access = v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--min-access must be an integer, got {v:?}"))
                })?;
            }
            "--dry-run" => dry_run = true,
            "--apply" => dry_run = false,
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(DecayArgs {
        ttl_days,
        min_access,
        dry_run,
        common,
    })
}

fn parse_stats(mut args: impl Iterator<Item = String>) -> Result<StatsArgs, CliError> {
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("unknown flag {tok:?}")));
    }
    Ok(StatsArgs { common })
}

fn parse_restore(mut args: impl Iterator<Item = String>) -> Result<RestoreArgs, CliError> {
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
                    "memory restore takes exactly one <memory_id> argument".into(),
                ))
            }
        }
    }
    let memory_id = memory_id
        .ok_or_else(|| CliError::Usage("memory restore requires a <memory_id> (64 hex)".into()))?;
    Ok(RestoreArgs { memory_id, common })
}

fn parse_consolidate(mut args: impl Iterator<Item = String>) -> Result<ConsolidateArgs, CliError> {
    let mut query: Option<String> = None;
    let mut k = DEFAULT_CONSOLIDATE_K;
    let mut window_hours: Option<u32> = None;
    // Consolidate defaults to a model-free PREVIEW; `--apply` drives the react chain.
    let mut dry_run = true;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--query" => query = Some(next_value(&mut args, "--query")?),
            "--k" => {
                let v = next_value(&mut args, "--k")?;
                k = v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--k must be a positive integer, got {v:?}"))
                })?;
            }
            "--window-hours" => {
                let v = next_value(&mut args, "--window-hours")?;
                window_hours = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--window-hours must be an integer, got {v:?}"))
                })?);
            }
            "--dry-run" => dry_run = true,
            "--apply" => dry_run = false,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs must be an integer, got {v:?}"))
                })?;
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(ConsolidateArgs {
        query,
        k: k.clamp(1, 64),
        window_hours,
        dry_run,
        timeout_secs,
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
        MemoryArgs::Decay(a) => execute_decay(a).await,
        MemoryArgs::Stats(a) => execute_stats(a).await,
        MemoryArgs::Restore(a) => execute_restore(a).await,
        MemoryArgs::Consolidate(a) => execute_consolidate(a).await,
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
            include_tombstoned: args.include_tombstoned,
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

async fn execute_decay(args: DecayArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .decay_memory(resolved.request(proto::DecayMemoryRequest {
            namespace: String::new(),
            ttl_days: args.ttl_days,
            min_access: args.min_access,
            dry_run: args.dry_run,
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_decay_memory(&resp, args.common.json));
    Ok(())
}

async fn execute_stats(args: StatsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .memory_stats(resolved.request(proto::MemoryStatsRequest {
            namespace: String::new(),
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_memory_stats(&resp, args.common.json));
    Ok(())
}

async fn execute_restore(args: RestoreArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .restore_memory(resolved.request(proto::RestoreMemoryRequest {
            memory_id: args.memory_id,
            namespace: String::new(),
        })?)
        .await
        .map_err(map_memory_status)?
        .into_inner();
    println!("{}", format::render_restore_memory(&resp, args.common.json));
    Ok(())
}

/// Build the consolidation instruction the react-memory chain follows (`--apply`).
/// Phrased to FORCE tool use: an OSS model will otherwise answer from guesswork at turn 0
/// (it cannot see its episodic memories until it calls `consolidate`).
fn consolidation_instruction(query: Option<&str>, window_hours: Option<u32>) -> String {
    let focus = query.map_or(String::new(), |q| format!(" about \"{q}\""));
    let window = window_hours.map_or(String::new(), |h| format!(" from the last {h} hours"));
    format!(
        "You have episodic memories from earlier that you CANNOT see until you retrieve them. \
         FIRST call the `consolidate` tool to bundle your recent episodic memories{focus}{window}. \
         THEN distill the key durable facts and call `remember` with kind=\"semantic\" to save ONE \
         concise summary. Only AFTER remembering, report what you consolidated. Do NOT answer from \
         guesswork — you must use the tools."
    )
}

async fn execute_consolidate(args: ConsolidateArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    if args.dry_run {
        // Model-free preview: list the episodic memories that WOULD be consolidated
        // (newest-first, optionally windowed), so an operator can inspect before running.
        let resp = client
            .list_memories(resolved.request(proto::ListMemoriesRequest {
                limit: Some(args.k.saturating_mul(4)),
                instance_id: None,
                namespace: String::new(),
                include_tombstoned: false,
            })?)
            .await
            .map_err(map_memory_status)?
            .into_inner();
        let now_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(i64::MAX);
        let cutoff = args
            .window_hours
            .map(|h| now_ms - i64::from(h).saturating_mul(3_600_000));
        let episodics: Vec<&proto::MemorySummary> = resp
            .memories
            .iter()
            .filter(|m| m.kind == "episodic")
            .filter(|m| cutoff.is_none_or(|c| m.created_ms >= c))
            .take(args.k as usize)
            .collect();
        println!(
            "{}",
            format::render_consolidate_preview(&episodics, args.query.as_deref(), args.common.json)
        );
        return Ok(());
    }

    // Live: drive a react-memory chain that bundles → distills → remembers.
    let instruction = consolidation_instruction(args.query.as_deref(), args.window_hours);
    let args_json = serde_json::json!({
        "instruction": instruction,
        "max_turns": 6,
        "max_tool_calls": 4,
    });
    let args_bytes = serde_json::to_vec(&args_json)
        .map_err(|e| CliError::Io(format!("consolidate args: {e}")))?;
    let resp = client
        .invoke(resolved.request(proto::InvokeRequest {
            handle: REACT_MEMORY_HANDLE.to_string(),
            args: args_bytes,
            context_bundles: Vec::new(),
            context_refs: Vec::new(),
        })?)
        .await
        .map_err(map_memory_status)?
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

    #[test]
    fn list_parses_include_tombstoned() {
        let MemoryArgs::List(a) = p(&["list", "--include-tombstoned"]).unwrap() else {
            panic!("expected list")
        };
        assert!(a.include_tombstoned);
        let MemoryArgs::List(b) = p(&["list"]).unwrap() else {
            panic!("expected list")
        };
        assert!(!b.include_tombstoned, "default hides decayed memories");
    }

    #[test]
    fn decay_defaults_to_dry_run_and_parses_knobs() {
        let MemoryArgs::Decay(a) = p(&["decay"]).unwrap() else {
            panic!("expected decay")
        };
        assert!(a.dry_run, "decay previews by default");
        assert_eq!(a.ttl_days, DEFAULT_TTL_DAYS);
        assert_eq!(a.min_access, DEFAULT_MIN_ACCESS);
        let MemoryArgs::Decay(b) =
            p(&["decay", "--apply", "--ttl-days", "30", "--min-access", "2"]).unwrap()
        else {
            panic!("expected decay")
        };
        assert!(!b.dry_run, "--apply evicts");
        assert_eq!(b.ttl_days, 30);
        assert_eq!(b.min_access, 2);
        assert!(p(&["decay", "--ttl-days", "lots"]).is_err());
    }

    #[test]
    fn stats_parses_and_rejects_junk() {
        assert!(matches!(p(&["stats"]).unwrap(), MemoryArgs::Stats(_)));
        assert!(matches!(
            p(&["stats", "--json"]).unwrap(),
            MemoryArgs::Stats(_)
        ));
        assert!(p(&["stats", "--nope"]).is_err());
    }

    #[test]
    fn restore_parses_a_64_hex_id() {
        let hex32 = "b".repeat(64);
        let MemoryArgs::Restore(a) = p(&["restore", &hex32]).unwrap() else {
            panic!("expected restore")
        };
        assert_eq!(a.memory_id.len(), 32);
        assert!(p(&["restore", "short"]).is_err());
        assert!(p(&["restore"]).is_err(), "restore needs an id");
    }

    #[test]
    fn consolidate_defaults_to_dry_run_and_parses_flags() {
        let MemoryArgs::Consolidate(a) = p(&["consolidate"]).unwrap() else {
            panic!("expected consolidate")
        };
        assert!(a.dry_run, "consolidate previews by default (model-free)");
        assert_eq!(a.k, DEFAULT_CONSOLIDATE_K);
        let MemoryArgs::Consolidate(b) = p(&[
            "consolidate",
            "--apply",
            "--query",
            "launch",
            "--k",
            "8",
            "--window-hours",
            "48",
        ])
        .unwrap() else {
            panic!("expected consolidate")
        };
        assert!(!b.dry_run);
        assert_eq!(b.query.as_deref(), Some("launch"));
        assert_eq!(b.k, 8);
        assert_eq!(b.window_hours, Some(48));
        // k is clamped to the server bound.
        let MemoryArgs::Consolidate(c) = p(&["consolidate", "--k", "999"]).unwrap() else {
            panic!("expected consolidate")
        };
        assert_eq!(c.k, 64);
    }

    #[test]
    fn consolidation_instruction_includes_focus_and_window() {
        let s = consolidation_instruction(Some("Q3 launch"), Some(24));
        assert!(s.contains("Q3 launch"));
        assert!(s.contains("24 hours"));
        assert!(s.contains("consolidate") && s.contains("remember"));
    }
}
