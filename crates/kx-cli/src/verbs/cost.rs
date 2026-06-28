//! `kx cost <INSTANCE_ID>` (alias `kx runs cost <INSTANCE_ID>`) — a run's DISPLAY-ONLY
//! local spend estimate (M11) over the gateway `GetRunCost` RPC. Tri-surface parity
//! with the UI + SDK.
//!
//! The estimate prices the run's durable turn/tool counters at the operator's
//! micro-USD rates (`KX_PRICING_PER_TURN_MICRO_USD` / `KX_PRICING_PER_TOOL_CALL_MICRO_USD`).
//! It is a BUDGET GUARDRAIL readout, NOT Cloud per-expert billing.

use kx_proto::proto;

use crate::client::ClientCommon;
use crate::error::CliError;
use crate::hex;

/// Parsed `cost` arguments.
#[derive(Debug)]
pub struct CostArgs {
    /// The hex `instance_id` (16 bytes).
    pub instance_id: String,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `cost` args: a positional `<INSTANCE_ID>` (hex) + the common flags.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<CostArgs, CliError> {
    let mut instance_id: Option<String> = None;
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" | "--instance-id" => {
                instance_id = Some(crate::client::next_value(&mut args, "--instance")?);
            }
            other if !other.starts_with("--") && instance_id.is_none() => {
                instance_id = Some(other.to_string());
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }
    let instance_id = instance_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CliError::Usage("cost requires a <INSTANCE_ID> (hex)".into()))?;
    Ok(CostArgs {
        instance_id,
        common,
    })
}

/// Execute `cost`.
pub async fn execute(args: CostArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    let req = proto::GetRunCostRequest {
        instance_id: hex::decode_fixed::<16>(&args.instance_id)?.to_vec(),
    };
    let c = client
        .get_run_cost(resolved.request(req)?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "instance_id": hex::encode(&c.instance_id),
                "turns": c.turns,
                "tool_calls": c.tool_calls,
                "estimated_micro_usd": c.estimated_micro_usd,
                "ceiling_micro_usd": c.ceiling_micro_usd,
                "per_turn_micro_usd": c.per_turn_micro_usd,
                "per_tool_call_micro_usd": c.per_tool_call_micro_usd,
                "over_ceiling": c.over_ceiling,
            })
        );
    } else {
        // Integer dollar formatting (no lossy u64→f64 cast): dollars . micro-remainder.
        println!(
            "turns={}  tool_calls={}  estimated=${}.{:06}  (rates: {}/{} µ$ per turn/call)",
            c.turns,
            c.tool_calls,
            c.estimated_micro_usd / 1_000_000,
            c.estimated_micro_usd % 1_000_000,
            c.per_turn_micro_usd,
            c.per_tool_call_micro_usd,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<CostArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_positional_and_flag_instance() {
        assert_eq!(p(&["abcd1234"]).unwrap().instance_id, "abcd1234");
        assert_eq!(
            p(&["--instance", "ef019999"]).unwrap().instance_id,
            "ef019999"
        );
    }

    #[test]
    fn rejects_missing_instance() {
        assert!(p(&[]).is_err());
        assert!(p(&["--json"]).is_err());
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = p(&["abcd", "--endpoint", "http://h:1", "--json"]).unwrap();
        assert_eq!(a.common.endpoint, "http://h:1");
        assert!(a.common.json);
    }
}
