//! `kx eval run | score` — the measure-first eval surface (RC1, D172).
//!
//! `kx eval run` runs the golden gate **locally**: it scores the embedded `golden-v1`
//! corpus against the committed baseline (no gateway, no model, cannot flake) and exits
//! non-zero on any regression — the same ratchet `just eval` enforces, reachable from the
//! single `kx` entry point. `kx eval score <INSTANCE_ID>` reads a live run's
//! expectation-free quality summary (terminal reached, turns / tool-calls, budget burn,
//! rejections) via the `ScoreRun` gateway RPC.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::hex;

/// The `eval` subcommand.
#[derive(Debug)]
pub enum EvalSub {
    /// Run the golden gate locally (embedded corpus vs committed baseline).
    Run {
        /// Allowed per-mille slack below baseline before a Gate counts as a regression.
        tolerance: u32,
    },
    /// Read a live run's expectation-free quality summary (the `ScoreRun` RPC).
    Score {
        /// The hex 16-byte run `instance_id`.
        instance_id: String,
    },
}

/// Parsed `eval` arguments.
#[derive(Debug)]
pub struct EvalArgs {
    /// The subcommand.
    pub sub: EvalSub,
    /// Common client flags (`--json`; `--endpoint`/auth for `score`).
    pub common: ClientCommon,
}

/// Parse `eval` args (the verb already consumed). First token selects `run` | `score`;
/// `run` takes an optional `--tolerance <per_mille>`; `score` takes a positional
/// `<INSTANCE_ID>` (hex).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<EvalArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("eval requires a subcommand: run | score".into()))?;

    let mut tolerance: u32 = 0;
    let mut instance_id: Option<String> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--tolerance" => {
                let v = next_value(&mut args, "--tolerance")?;
                tolerance = v
                    .parse()
                    .map_err(|_| CliError::Usage(format!("invalid --tolerance {v:?}")))?;
            }
            other if !other.starts_with("--") && instance_id.is_none() => {
                instance_id = Some(other.to_string());
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let sub = match kw.as_str() {
        "run" => EvalSub::Run { tolerance },
        "score" => EvalSub::Score {
            instance_id: instance_id.filter(|s| !s.is_empty()).ok_or_else(|| {
                CliError::Usage("eval score requires an <INSTANCE_ID> (hex)".into())
            })?,
        },
        other => {
            return Err(CliError::Usage(format!(
                "unknown eval subcommand {other:?} (expected run | score)"
            )))
        }
    };
    Ok(EvalArgs { sub, common })
}

/// Execute `eval`.
pub async fn execute(args: EvalArgs) -> Result<(), CliError> {
    let json = args.common.json;
    match args.sub {
        EvalSub::Run { tolerance } => run_gate(tolerance, json),
        EvalSub::Score { instance_id } => score_run(args.common, &instance_id).await,
    }
}

/// The LOCAL golden gate — no gateway, no model.
fn run_gate(tolerance: u32, json: bool) -> Result<(), CliError> {
    let env_label = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    let report = kx_eval::score_golden_v1(env_label, "unknown".to_string())
        .map_err(|e| CliError::Runtime(format!("eval scoring failed: {e}")))?;
    let baseline = kx_eval::embedded_baseline()
        .map_err(|e| CliError::Runtime(format!("eval baseline: {e}")))?;
    let cmp = kx_eval::compare_to_baseline(&report, &baseline, tolerance)
        .map_err(|e| CliError::Runtime(format!("eval compare: {e}")))?;

    if json {
        let gates: Vec<_> = report
            .gates
            .iter()
            .map(|g| serde_json::json!({ "id": g.id, "per_mille": g.per_mille }))
            .collect();
        let regressions: Vec<_> = cmp
            .regressions
            .iter()
            .map(|r| {
                serde_json::json!({
                    "metric_id": r.metric_id,
                    "current_per_mille": r.current_per_mille,
                    "baseline_per_mille": r.baseline_per_mille,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "suite_id": report.suite_id,
                "suite_digest": report.suite_digest,
                "gates": gates,
                "ok": cmp.ok,
                "regressions": regressions,
            })
        );
    } else {
        for g in &report.gates {
            println!("  {:<18} {:>4} / 1000", g.id, g.per_mille);
        }
        if cmp.ok {
            println!(
                "eval: PASS — all {} gate(s) >= baseline",
                report.gates.len()
            );
        } else {
            println!("eval: FAIL — {} regression(s):", cmp.regressions.len());
            for r in &cmp.regressions {
                println!(
                    "  - {}: {} < baseline {}",
                    r.metric_id, r.current_per_mille, r.baseline_per_mille
                );
            }
        }
    }

    if cmp.ok {
        Ok(())
    } else {
        Err(CliError::Failed)
    }
}

/// The per-run quality readout via the `ScoreRun` gateway RPC.
async fn score_run(common: ClientCommon, instance_id: &str) -> Result<(), CliError> {
    let resolved = common.resolve()?;
    let json = common.json;
    let mut client = resolved.connect().await?;
    let req = proto::ScoreRunRequest {
        instance_id: hex::decode_fixed::<16>(instance_id)?.to_vec(),
    };
    let resp = client
        .score_run(resolved.request(req)?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "instance_id": hex::encode(&resp.instance_id),
                "terminal": resp.terminal,
                "reached_answer": resp.reached_answer,
                "turns_used": resp.turns_used,
                "tool_calls_used": resp.tool_calls_used,
                "max_turns": resp.max_turns,
                "max_tool_calls": resp.max_tool_calls,
                "rejections": resp.rejections,
                "turn_budget_used_per_mille": resp.turn_budget_used_per_mille,
                "tool_budget_used_per_mille": resp.tool_budget_used_per_mille,
            })
        );
    } else {
        let answered = if resp.reached_answer {
            " (answered)"
        } else {
            ""
        };
        println!("run {}", hex::encode(&resp.instance_id));
        println!("  terminal     {}{answered}", resp.terminal);
        println!("  turns        {} / {}", resp.turns_used, resp.max_turns);
        println!(
            "  tool calls   {} / {}",
            resp.tool_calls_used, resp.max_tool_calls
        );
        println!("  rejections   {}", resp.rejections);
        println!(
            "  budget used  turns {}‰ · tools {}‰",
            resp.turn_budget_used_per_mille, resp.tool_budget_used_per_mille
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<EvalArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_run_and_score() {
        assert!(matches!(
            p(&["run"]).unwrap().sub,
            EvalSub::Run { tolerance: 0 }
        ));
        assert!(matches!(
            p(&["run", "--tolerance", "20"]).unwrap().sub,
            EvalSub::Run { tolerance: 20 }
        ));
        let s = p(&["score", "00112233445566778899aabbccddeeff"]).unwrap();
        let EvalSub::Score { instance_id } = s.sub else {
            panic!("expected Score");
        };
        assert_eq!(instance_id, "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn rejects_missing_and_unknown() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["score"]).is_err(), "score needs an instance id");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(p(&["run", "--tolerance", "nope"]).is_err(), "bad tolerance");
    }

    #[test]
    fn common_json_flag_is_consumed() {
        let a = p(&["run", "--json"]).unwrap();
        assert!(a.common.json);
    }
}
