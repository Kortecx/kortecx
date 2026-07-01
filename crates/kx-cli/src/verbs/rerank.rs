//! `kx rerank list` — LLM-rerank-turn observability over the gateway
//! (`ListReRankTurns`, RC4c-2). The durable `ReRankRound` facts the live RAG
//! chain commits when it reorders retrieved candidates with a model: each turn's
//! run-salted rerank Mote id, the resolved model, the frozen outcome
//! (`pending` | `reranked` | `failed_closed`), how many candidates were ranked,
//! and — for a `reranked` outcome — the exact permutation the runtime enforced
//! (SN-8: a permutation, never a similarity score). Read-only, newest-first;
//! `--instance` scopes to one run (serve's journal is shared).

use kx_proto::proto;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `rerank` arguments.
#[derive(Debug)]
pub struct ReRankArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// Scope to one run (16B instance id).
    pub instance: Option<[u8; 16]>,
    /// Page size (server clamps to its max page).
    pub limit: Option<u32>,
}

/// Parse `rerank` args (the verb already consumed). The first token selects the
/// subcommand (only `list` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ReRankArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("rerank requires a subcommand: list".into()))?;
    if kw != "list" {
        return Err(CliError::Usage(format!(
            "unknown rerank subcommand {kw:?} (expected: list)"
        )));
    }
    let mut common = ClientCommon::default();
    let mut instance = None;
    let mut limit = None;
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--limit" => {
                let v = next_value(&mut args, "--limit")?;
                limit = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--limit must be a positive integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(ReRankArgs {
        common,
        instance,
        limit,
    })
}

/// Execute `rerank list`.
pub async fn execute(args: ReRankArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_re_rank_turns(resolved.request(proto::ListReRankTurnsRequest {
            limit: args.limit,
            instance_id: args.instance.map(|b| b.to_vec()),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_rerank_turns(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ReRankArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn list_parses_with_instance_limit_and_json() {
        let a = p(&["list"]).unwrap();
        assert!(a.instance.is_none() && a.limit.is_none());
        let a = p(&[
            "list",
            "--instance",
            &"ab".repeat(16),
            "--limit",
            "8",
            "--json",
        ])
        .unwrap();
        assert_eq!(a.instance, Some([0xab; 16]));
        assert_eq!(a.limit, Some(8));
        assert!(a.common.json);
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["turns"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--limit"]).is_err(), "missing value");
        assert!(p(&["list", "--limit", "many"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
        // Wrong hex length / non-hex --instance is rejected.
        assert!(p(&["list", "--instance", &"ab".repeat(32)]).is_err());
        assert!(p(&["list", "--instance", "zz"]).is_err());
    }
}
