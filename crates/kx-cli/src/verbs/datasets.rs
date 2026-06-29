//! `kx datasets` — browse, populate, and search the RAG data-plane (the T3.7
//! `ListDatasets` / `IngestDocuments` / `QueryDataset` path).
//!
//! - `kx datasets list [--json]` — every dataset on this serve (name, doc count, dim).
//! - `kx datasets ingest <dataset> (--text <s> | --file <path>)... [--json]` — add
//!   documents to `dataset` (created on first ingest). The CLI uses the SERVER-EMBED
//!   path (each `--text`/`--file` payload is embedded server-side), so it needs
//!   `kx serve --features inference` with a model; without one the gateway answers
//!   `FAILED_PRECONDITION` honestly. The client-vector (FFI-free) ingest path is an
//!   SDK surface (vectors over the wire), not a CLI one.
//! - `kx datasets query <dataset> --text <query> [--k N] [--mode dense|hybrid] [--json]`
//!   — top-k semantic search. `--mode` (RC4a) selects dense-only vs hybrid (BM25 +
//!   dense, RRF-fused); omitted ⇒ the server's configured default. Each hit's `score`
//!   is DISPLAY-ONLY (SN-8) — a ranking aid, never an identity input; the durable
//!   result is the ordered content-ref SET. A chunked corpus shows each hit's passage
//!   position within its parent document.
//!
//! A pre-T3.7 / `hnsw`-less gateway has no dataset view and answers
//! `Unimplemented`, rendered honestly. There is no `delete` subcommand (the OSS
//! dataset store is append-only + content-dedup; a test pins its absence).

use std::path::PathBuf;

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The default top-k when `--k` is omitted (the server clamps to its own max).
const DEFAULT_K: u32 = 10;

/// Parsed `datasets` arguments.
#[derive(Debug)]
pub enum DatasetsArgs {
    /// `datasets list`.
    List(ListArgs),
    /// `datasets ingest <dataset> …`.
    Ingest(IngestArgs),
    /// `datasets query <dataset> --text …`.
    Query(QueryArgs),
}

/// Parsed `datasets list` arguments.
#[derive(Debug)]
pub struct ListArgs {
    /// Common client flags.
    pub common: ClientCommon,
}

/// One ingest document source (resolved to bytes at execute time, so parsing
/// stays pure + testable — no filesystem touch in `parse`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocSource {
    /// An inline `--text` payload.
    Text(String),
    /// A `--file` payload, read at execute time.
    File(PathBuf),
}

/// Parsed `datasets ingest` arguments.
#[derive(Debug)]
pub struct IngestArgs {
    /// The dataset NAME (created on first ingest).
    pub dataset: String,
    /// The document sources, in order (one document each).
    pub sources: Vec<DocSource>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parsed `datasets query` arguments.
#[derive(Debug)]
pub struct QueryArgs {
    /// The dataset NAME to search.
    pub dataset: String,
    /// The query text (server-embedded).
    pub text: String,
    /// Top-k (absent ⇒ `DEFAULT_K`; the server clamps to a sane max).
    pub k: Option<u32>,
    /// RC4a: retrieval mode wire value (0 = server default, 1 = dense, 2 = hybrid).
    pub mode: i32,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `datasets` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<DatasetsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("datasets requires a subcommand: list | ingest | query".into())
    })?;
    match kw.as_str() {
        "list" => parse_list(args).map(DatasetsArgs::List),
        "ingest" => parse_ingest(args).map(DatasetsArgs::Ingest),
        "query" => parse_query(args).map(DatasetsArgs::Query),
        other => Err(CliError::Usage(format!(
            "unknown datasets subcommand {other:?} (expected: list | ingest | query)"
        ))),
    }
}

fn parse_list(mut args: impl Iterator<Item = String>) -> Result<ListArgs, CliError> {
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("unknown flag {flag:?}")));
    }
    Ok(ListArgs { common })
}

fn parse_ingest(mut args: impl Iterator<Item = String>) -> Result<IngestArgs, CliError> {
    let mut dataset: Option<String> = None;
    let mut sources: Vec<DocSource> = Vec::new();
    let mut common = ClientCommon::default();
    while let Some(tok) = args.next() {
        if common.try_consume(&tok, &mut args)? {
            continue;
        }
        match tok.as_str() {
            "--text" => sources.push(DocSource::Text(next_value(&mut args, "--text")?)),
            "--file" => sources.push(DocSource::File(PathBuf::from(next_value(
                &mut args, "--file",
            )?))),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            _ if dataset.is_none() => dataset = Some(tok),
            _ => {
                return Err(CliError::Usage(
                    "datasets ingest takes exactly one <dataset> argument".into(),
                ))
            }
        }
    }
    let dataset = dataset
        .ok_or_else(|| CliError::Usage("datasets ingest requires a <dataset> name".into()))?;
    if sources.is_empty() {
        return Err(CliError::Usage(
            "datasets ingest requires at least one --text <s> or --file <path>".into(),
        ));
    }
    Ok(IngestArgs {
        dataset,
        sources,
        common,
    })
}

fn parse_query(mut args: impl Iterator<Item = String>) -> Result<QueryArgs, CliError> {
    let mut dataset: Option<String> = None;
    let mut text: Option<String> = None;
    let mut k: Option<u32> = None;
    let mut mode: i32 = proto::RetrievalMode::Unspecified as i32;
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
            "--mode" => {
                let v = next_value(&mut args, "--mode")?;
                mode = match v.to_ascii_lowercase().as_str() {
                    "dense" => proto::RetrievalMode::Dense as i32,
                    "hybrid" => proto::RetrievalMode::Hybrid as i32,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--mode must be dense|hybrid, got {other:?}"
                        )))
                    }
                };
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            _ if dataset.is_none() => dataset = Some(tok),
            _ => {
                return Err(CliError::Usage(
                    "datasets query takes exactly one <dataset> argument".into(),
                ))
            }
        }
    }
    let dataset = dataset
        .ok_or_else(|| CliError::Usage("datasets query requires a <dataset> name".into()))?;
    let text =
        text.ok_or_else(|| CliError::Usage("datasets query requires --text <query>".into()))?;
    Ok(QueryArgs {
        dataset,
        text,
        k,
        mode,
        common,
    })
}

/// Map a dataset RPC status to an honest CLI error — a pre-T3.7 / `hnsw`-less
/// gateway has no dataset view and answers `Unimplemented`.
fn map_datasets_status(status: tonic::Status) -> CliError {
    if status.code() == Code::Unimplemented {
        CliError::Rpc {
            code: Code::Unimplemented,
            message: "datasets are not wired on this gateway (run `kx serve --features hnsw`)"
                .into(),
            refusal_code: None,
        }
    } else {
        CliError::from_status(status)
    }
}

/// Execute `datasets`.
pub async fn execute(args: DatasetsArgs) -> Result<(), CliError> {
    match args {
        DatasetsArgs::List(a) => execute_list(a).await,
        DatasetsArgs::Ingest(a) => execute_ingest(a).await,
        DatasetsArgs::Query(a) => execute_query(a).await,
    }
}

async fn execute_list(args: ListArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_datasets(resolved.request(proto::ListDatasetsRequest {})?)
        .await
        .map_err(map_datasets_status)?
        .into_inner();
    println!("{}", format::render_datasets(&resp, args.common.json));
    Ok(())
}

async fn execute_ingest(args: IngestArgs) -> Result<(), CliError> {
    // Resolve every source to bytes (read files now) — server-embed path: the
    // wire `embedding` stays empty so the gateway embeds each document.
    let mut documents = Vec::with_capacity(args.sources.len());
    for src in &args.sources {
        let content = match src {
            DocSource::Text(s) => s.clone().into_bytes(),
            DocSource::File(p) => {
                std::fs::read(p).map_err(|e| CliError::Io(format!("read {}: {e}", p.display())))?
            }
        };
        documents.push(proto::IngestDocument {
            content,
            embedding: Vec::new(),
            doc_id: None,
            metadata: std::collections::HashMap::new(),
        });
    }
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    // RC4a: warn (stderr) when the serve embeds with a decoder LLM (human runs only —
    // never pollutes the --json stdout contract).
    if !args.common.json {
        warn_if_decoder_embed(&mut client, &resolved).await;
    }
    let resp = client
        .ingest_documents(resolved.request(proto::IngestDocumentsRequest {
            dataset: args.dataset.clone(),
            documents,
        })?)
        .await
        .map_err(map_datasets_status)?
        .into_inner();
    println!("{}", format::render_ingest(&resp, args.common.json));
    Ok(())
}

async fn execute_query(args: QueryArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .query_dataset(resolved.request(proto::QueryDatasetRequest {
            dataset: args.dataset.clone(),
            query_text: args.text.clone(),
            query_embedding: Vec::new(),
            k: args.k.unwrap_or(DEFAULT_K),
            retrieval_mode: args.mode,
        })?)
        .await
        .map_err(map_datasets_status)?
        .into_inner();
    println!("{}", format::render_dataset_hits(&resp, args.common.json));
    Ok(())
}

/// Best-effort decoder-as-embedder advisory (RC4a, `T-RAG-EMBED-QUALITY`): a single
/// `GetServerInfo` read; if the serve's embedder is a generative decoder LLM, print a
/// one-line stderr note recommending a dedicated embed model. Never fails the command
/// (an old / unauthenticated / model-less serve just skips the advisory).
async fn warn_if_decoder_embed(
    client: &mut kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
) {
    let Ok(req) = resolved.request(proto::GetServerInfoRequest {}) else {
        return;
    };
    if let Ok(resp) = client.get_server_info(req).await {
        let info = resp.into_inner();
        if info.embed_model_is_decoder {
            eprintln!(
                "note: embedding with a decoder model ({}) — for better retrieval set \
                 KX_SERVE_EMBED_MODEL to a dedicated embedder (e.g. embeddinggemma)",
                info.embed_model_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<DatasetsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    fn ingest(parts: &[&str]) -> IngestArgs {
        match p(parts).unwrap() {
            DatasetsArgs::Ingest(a) => a,
            other => panic!("expected ingest, got {other:?}"),
        }
    }

    fn query(parts: &[&str]) -> QueryArgs {
        match p(parts).unwrap() {
            DatasetsArgs::Query(a) => a,
            other => panic!("expected query, got {other:?}"),
        }
    }

    #[test]
    fn list_parses_bare_and_json() {
        assert!(matches!(p(&["list"]).unwrap(), DatasetsArgs::List(_)));
        let DatasetsArgs::List(a) = p(&["list", "--json"]).unwrap() else {
            panic!("expected list")
        };
        assert!(a.common.json);
    }

    #[test]
    fn ingest_collects_text_and_file_sources_in_order() {
        let a = ingest(&[
            "ingest",
            "corpus",
            "--text",
            "hello",
            "--file",
            "/tmp/a.md",
            "--text",
            "world",
        ]);
        assert_eq!(a.dataset, "corpus");
        assert_eq!(
            a.sources,
            vec![
                DocSource::Text("hello".into()),
                DocSource::File(PathBuf::from("/tmp/a.md")),
                DocSource::Text("world".into()),
            ]
        );
    }

    #[test]
    fn query_parses_text_and_k() {
        let a = query(&["query", "corpus", "--text", "needle", "--k", "5", "--json"]);
        assert_eq!(a.dataset, "corpus");
        assert_eq!(a.text, "needle");
        assert_eq!(a.k, Some(5));
        assert!(a.common.json);
        // --k defaults to None (execute applies DEFAULT_K).
        let b = query(&["query", "c", "--text", "x"]);
        assert_eq!(b.k, None);
        // --mode defaults to UNSPECIFIED (the server default).
        assert_eq!(b.mode, proto::RetrievalMode::Unspecified as i32);
    }

    #[test]
    fn query_parses_mode() {
        assert_eq!(
            query(&["query", "c", "--text", "x", "--mode", "hybrid"]).mode,
            proto::RetrievalMode::Hybrid as i32
        );
        assert_eq!(
            query(&["query", "c", "--text", "x", "--mode", "DENSE"]).mode,
            proto::RetrievalMode::Dense as i32
        );
        assert!(
            p(&["query", "c", "--text", "x", "--mode", "bogus"]).is_err(),
            "--mode must be dense|hybrid"
        );
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        // delete is NOT an OSS subcommand (append-only store).
        assert!(p(&["delete", "corpus"]).is_err(), "delete is not a verb");
        assert!(p(&["ingest"]).is_err(), "ingest needs a dataset");
        assert!(p(&["ingest", "c"]).is_err(), "ingest needs a source");
        assert!(p(&["ingest", "c", "d"]).is_err(), "two positionals");
        assert!(p(&["ingest", "c", "--text"]).is_err(), "missing value");
        assert!(p(&["ingest", "c", "--bogus", "x"]).is_err());
        assert!(p(&["query", "c"]).is_err(), "query needs --text");
        assert!(
            p(&["query", "--text", "x"]).is_err(),
            "query needs a dataset"
        );
        assert!(p(&["query", "c", "--text", "x", "--k", "lots"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
    }
}
