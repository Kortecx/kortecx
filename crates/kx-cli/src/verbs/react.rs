//! `kx react list` — ReAct-turn observability over the gateway
//! (`ListReactTurns`, PR-2d-1). The durable `ReactRound` facts the live ReAct
//! chain commits: each turn's run-salted Mote id, its settled branch
//! (`pending` | `answer` | `tool` | `dead_lettered`), the fired tool for a
//! `tool` branch, and the run's durable budget caps. Read-only, newest-first;
//! `--instance` scopes to one run's chain.

use kx_proto::proto;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `react` arguments.
#[derive(Debug)]
pub struct ReactArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// Scope to one run's chain (16B instance id).
    pub instance: Option<[u8; 16]>,
    /// Page size (server clamps to its max page).
    pub limit: Option<u32>,
}

/// Parse `react` args (the verb already consumed). The first token selects the
/// subcommand (only `list` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ReactArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("react requires a subcommand: list".into()))?;
    if kw != "list" {
        return Err(CliError::Usage(format!(
            "unknown react subcommand {kw:?} (expected: list)"
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
    Ok(ReactArgs {
        common,
        instance,
        limit,
    })
}

/// Execute `react list`.
pub async fn execute(args: ReactArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_react_turns(resolved.request(proto::ListReactTurnsRequest {
            limit: args.limit,
            instance_id: args.instance.map(|b| b.to_vec()),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_react_turns(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ReactArgs, CliError> {
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
