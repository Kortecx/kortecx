//! `kx telemetry list` — mote execution telemetry over the gateway
//! (`ListMoteTelemetry`, Batch C). Host-recorded exhaust as motes actually ran:
//! wall-clock, model usage, the fired tool. Newest-first, cursor pagination
//! (`--before-seq` = the last page's lowest `seq`). Read-only; the rows live in
//! a rebuildable-to-empty sidecar — AUDIT/DISPLAY ONLY, never truth/identity.
//! A pre-Batch-C gateway answers `Unimplemented` (rendered honestly).

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `telemetry` arguments.
#[derive(Debug)]
pub struct TelemetryArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// Scope to one run (16B instance id).
    pub instance: Option<[u8; 16]>,
    /// Scope to one Mote (32B mote id).
    pub mote: Option<[u8; 32]>,
    /// Page size (server clamps 1..=500; absent = 200).
    pub limit: Option<u32>,
    /// Pagination cursor: only rows with `seq < before_seq`.
    pub before_seq: Option<u64>,
}

/// Parse `telemetry` args (the verb already consumed). The first token selects
/// the subcommand (only `list` today).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<TelemetryArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("telemetry requires a subcommand: list".into()))?;
    if kw != "list" {
        return Err(CliError::Usage(format!(
            "unknown telemetry subcommand {kw:?} (expected: list)"
        )));
    }
    let mut common = ClientCommon::default();
    let mut instance = None;
    let mut mote = None;
    let mut limit = None;
    let mut before_seq = None;
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--mote" => mote = Some(take_fixed::<_, 32>(&mut args, "--mote")?),
            "--limit" => {
                let v = next_value(&mut args, "--limit")?;
                limit = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--limit must be a positive integer, got {v:?}"))
                })?);
            }
            "--before-seq" => {
                let v = next_value(&mut args, "--before-seq")?;
                before_seq = Some(v.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!("--before-seq must be an integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(TelemetryArgs {
        common,
        instance,
        mote,
        limit,
        before_seq,
    })
}

/// Execute `telemetry list`.
pub async fn execute(args: TelemetryArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_mote_telemetry(resolved.request(proto::ListMoteTelemetryRequest {
            limit: args.limit,
            instance_id: args.instance.map(|b| b.to_vec()),
            mote_id: args.mote.map(|b| b.to_vec()),
            before_seq: args.before_seq,
        })?)
        .await
        .map_err(|status| {
            // Forward-compat degrade: a pre-Batch-C serve has no telemetry
            // sidecar and answers Unimplemented — say so honestly.
            if status.code() == Code::Unimplemented {
                CliError::Rpc {
                    code: Code::Unimplemented,
                    message: "mote telemetry is not wired on this gateway (upgrade the serve)"
                        .into(),
                    refusal_code: None,
                }
            } else {
                CliError::from_status(status)
            }
        })?
        .into_inner();
    println!("{}", format::render_telemetry(&resp, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<TelemetryArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn list_parses_with_filters_and_pagination() {
        let a = p(&["list"]).unwrap();
        assert!(a.instance.is_none() && a.mote.is_none());
        assert!(a.limit.is_none() && a.before_seq.is_none());
        let a = p(&[
            "list",
            "--instance",
            &"ab".repeat(16),
            "--mote",
            &"cd".repeat(32),
            "--limit",
            "50",
            "--before-seq",
            "99",
            "--json",
        ])
        .unwrap();
        assert_eq!(a.instance, Some([0xab; 16]));
        assert_eq!(a.mote, Some([0xcd; 32]));
        assert_eq!(a.limit, Some(50));
        assert_eq!(a.before_seq, Some(99));
        assert!(a.common.json);
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["history"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--limit"]).is_err(), "missing value");
        assert!(p(&["list", "--limit", "many"]).is_err());
        assert!(p(&["list", "--before-seq", "-1"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
        // Wrong hex lengths: a 32B value in --instance, a 16B value in --mote.
        assert!(p(&["list", "--instance", &"ab".repeat(32)]).is_err());
        assert!(p(&["list", "--mote", &"cd".repeat(16)]).is_err());
        assert!(p(&["list", "--instance", "zz"]).is_err(), "non-hex");
    }
}
