//! `kx runs list` — durable run history over the gateway (`ListRuns`, Batch B).
//! Tri-surface parity with the console Workflows list + the SDK `listRuns` /
//! `list_runs`. Read-only: one journal fold server-side, newest-first, cursor
//! pagination (`--before-seq` = the last page's lowest `registered_seq`).

use kx_proto::proto;

use crate::client::ClientCommon;
use crate::error::CliError;
use crate::format;

/// Parsed `runs` arguments.
#[derive(Debug)]
pub struct RunsArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// Page size (server clamps to its max page of 500).
    pub limit: Option<u32>,
    /// Pagination cursor: only runs with `registered_seq < before_seq`.
    pub before_seq: Option<u64>,
}

/// Parse `runs` args (the verb already consumed). The first token selects the
/// subcommand (only `list` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<RunsArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("runs requires a subcommand: list".into()))?;
    if kw != "list" {
        return Err(CliError::Usage(format!(
            "unknown runs subcommand {kw:?} (expected: list)"
        )));
    }
    let mut common = ClientCommon::default();
    let mut limit = None;
    let mut before_seq = None;
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--limit" => {
                let v = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--limit requires a value".into()))?;
                limit = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--limit must be a positive integer, got {v:?}"))
                })?);
            }
            "--before-seq" => {
                let v = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--before-seq requires a value".into()))?;
                before_seq = Some(v.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!("--before-seq must be an integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(RunsArgs {
        common,
        limit,
        before_seq,
    })
}

/// Execute `runs list`.
pub async fn execute(args: RunsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_runs(resolved.request(proto::ListRunsRequest {
            limit: args.limit,
            before_seq: args.before_seq,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_runs(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<RunsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn list_parses_with_pagination_flags() {
        let a = p(&["list"]).unwrap();
        assert!(a.limit.is_none() && a.before_seq.is_none());
        let a = p(&["list", "--limit", "10", "--before-seq", "42", "--json"]).unwrap();
        assert_eq!(a.limit, Some(10));
        assert_eq!(a.before_seq, Some(42));
        assert!(a.common.json);
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err());
        assert!(p(&["history"]).is_err());
        assert!(p(&["list", "--limit"]).is_err());
        assert!(p(&["list", "--limit", "many"]).is_err());
        assert!(p(&["list", "--before-seq", "-1"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
    }
}
