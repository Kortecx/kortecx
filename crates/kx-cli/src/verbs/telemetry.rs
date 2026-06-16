//! `kx telemetry list` — mote execution telemetry over the gateway
//! (`ListMoteTelemetry`, Batch C). Host-recorded exhaust as motes actually ran:
//! wall-clock, model usage, the fired tool. Newest-first, cursor pagination
//! (`--before-seq` = the last page's lowest `seq`). Read-only; the rows live in
//! a rebuildable-to-empty sidecar — AUDIT/DISPLAY ONLY, never truth/identity.
//!
//! `kx telemetry summary` — the EXACT, cross-page per-model token-economy
//! rollup (`ListTelemetrySummary`, W1a-3): output tokens + wall-clock summed
//! `GROUP BY model_id` server-side, optionally scoped to one run
//! (`--instance`). No cost/$ — billing is CLOUD. A pre-Batch-C / pre-W1a-3
//! gateway answers `Unimplemented` (rendered honestly).

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Which telemetry view the verb runs.
#[derive(Debug)]
pub enum TelemetrySub {
    /// `list` — a newest-first page of per-mote exec rows.
    List {
        /// Scope to one Mote (32B mote id).
        mote: Option<[u8; 32]>,
        /// Page size (server clamps 1..=500; absent = 200).
        limit: Option<u32>,
        /// Pagination cursor: only rows with `seq < before_seq`.
        before_seq: Option<u64>,
    },
    /// `summary` — the exact per-model token rollup (W1a-3).
    Summary,
}

/// Parsed `telemetry` arguments.
#[derive(Debug)]
pub struct TelemetryArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// Scope to one run (16B instance id) — shared by both subcommands.
    pub instance: Option<[u8; 16]>,
    /// The selected subcommand.
    pub sub: TelemetrySub,
}

/// Parse `telemetry` args (the verb already consumed). The first token selects
/// the subcommand (`list` or `summary`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<TelemetryArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("telemetry requires a subcommand: list | summary".into()))?;
    match kw.as_str() {
        "list" => parse_list(args),
        "summary" => parse_summary(args),
        other => Err(CliError::Usage(format!(
            "unknown telemetry subcommand {other:?} (expected: list | summary)"
        ))),
    }
}

fn parse_list(mut args: impl Iterator<Item = String>) -> Result<TelemetryArgs, CliError> {
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
        sub: TelemetrySub::List {
            mote,
            limit,
            before_seq,
        },
    })
}

fn parse_summary(mut args: impl Iterator<Item = String>) -> Result<TelemetryArgs, CliError> {
    let mut common = ClientCommon::default();
    let mut instance = None;
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    Ok(TelemetryArgs {
        common,
        instance,
        sub: TelemetrySub::Summary,
    })
}

/// Forward-compat degrade: a gateway without the sidecar answers Unimplemented
/// — say so honestly instead of leaking the raw status.
fn map_unimplemented(status: tonic::Status) -> CliError {
    if status.code() == Code::Unimplemented {
        CliError::Rpc {
            code: Code::Unimplemented,
            message: "mote telemetry is not wired on this gateway (upgrade the serve)".into(),
            refusal_code: None,
        }
    } else {
        CliError::from_status(status)
    }
}

/// Execute `telemetry list|summary`.
pub async fn execute(args: TelemetryArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;
    match args.sub {
        TelemetrySub::List {
            mote,
            limit,
            before_seq,
        } => {
            let resp = client
                .list_mote_telemetry(resolved.request(proto::ListMoteTelemetryRequest {
                    limit,
                    instance_id: args.instance.map(|b| b.to_vec()),
                    mote_id: mote.map(|b| b.to_vec()),
                    before_seq,
                })?)
                .await
                .map_err(map_unimplemented)?
                .into_inner();
            println!("{}", format::render_telemetry(&resp, json));
        }
        TelemetrySub::Summary => {
            let resp = client
                .list_telemetry_summary(resolved.request(proto::ListTelemetrySummaryRequest {
                    instance_id: args.instance.map(|b| b.to_vec()),
                })?)
                .await
                .map_err(map_unimplemented)?
                .into_inner();
            println!("{}", format::render_telemetry_summary(&resp, json));
        }
    }
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
        assert!(a.instance.is_none());
        assert!(matches!(
            a.sub,
            TelemetrySub::List {
                mote: None,
                limit: None,
                before_seq: None
            }
        ));
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
        assert!(a.common.json);
        match a.sub {
            TelemetrySub::List {
                mote,
                limit,
                before_seq,
            } => {
                assert_eq!(mote, Some([0xcd; 32]));
                assert_eq!(limit, Some(50));
                assert_eq!(before_seq, Some(99));
            }
            TelemetrySub::Summary => panic!("expected list"),
        }
    }

    #[test]
    fn summary_parses_with_optional_instance() {
        let a = p(&["summary"]).unwrap();
        assert!(a.instance.is_none());
        assert!(matches!(a.sub, TelemetrySub::Summary));
        let a = p(&["summary", "--instance", &"ab".repeat(16), "--json"]).unwrap();
        assert_eq!(a.instance, Some([0xab; 16]));
        assert!(a.common.json);
        assert!(matches!(a.sub, TelemetrySub::Summary));
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
        // summary takes no list-only flags.
        assert!(p(&["summary", "--mote", &"cd".repeat(32)]).is_err());
        assert!(p(&["summary", "--limit", "5"]).is_err());
        assert!(p(&["summary", "--instance", &"ab".repeat(32)]).is_err());
    }
}
