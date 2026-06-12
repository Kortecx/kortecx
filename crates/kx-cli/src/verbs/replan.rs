//! `kx replan list` — re-plan-round observability over the gateway
//! (`ListReplanRounds`, PR-2c-2). The durable `ReplanRound` facts the live
//! re-plan-on-failure loop commits: round index (0 = the initial-plan anchor),
//! the shaper Mote, the resolved model, the failed steps that triggered the
//! round, and the escalation flag. Read-only, newest-first, operator-global.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `replan` arguments.
#[derive(Debug)]
pub struct ReplanArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// Page size (server clamps to its max page).
    pub limit: Option<u32>,
}

/// Parse `replan` args (the verb already consumed). The first token selects the
/// subcommand (only `list` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ReplanArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("replan requires a subcommand: list".into()))?;
    if kw != "list" {
        return Err(CliError::Usage(format!(
            "unknown replan subcommand {kw:?} (expected: list)"
        )));
    }
    let mut common = ClientCommon::default();
    let mut limit = None;
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--limit" => {
                let v = next_value(&mut args, "--limit")?;
                limit = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--limit must be a positive integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(ReplanArgs { common, limit })
}

/// Execute `replan list`.
pub async fn execute(args: ReplanArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_replan_rounds(resolved.request(proto::ListReplanRoundsRequest { limit: args.limit })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_replan_rounds(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ReplanArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn list_parses_with_limit_and_json() {
        let a = p(&["list"]).unwrap();
        assert!(a.limit.is_none() && !a.common.json);
        let a = p(&["list", "--limit", "25", "--json"]).unwrap();
        assert_eq!(a.limit, Some(25));
        assert!(a.common.json);
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["rounds"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--limit"]).is_err(), "missing value");
        assert!(p(&["list", "--limit", "many"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
    }
}
