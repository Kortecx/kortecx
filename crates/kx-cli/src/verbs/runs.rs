//! `kx runs list | rerun` — durable run history + "Re-run with changes" over the
//! gateway. `list` (Batch B `ListRuns`): every registered run, newest-first, from
//! one server-side journal fold. `rerun` (PR-D `GetRunInputs` → `Invoke`): fetch
//! the args a run was submitted with, overlay `--set k=v` edits, and re-invoke —
//! only the changed sub-DAG recomputes (the kernel's exact-equality dedup), an
//! unchanged re-run returns the existing result. Tri-surface parity with the
//! console Workflows drawer + the SDK `getRunInputs`/`get_run_inputs`. A re-run is
//! just a new `Invoke` (same admission, NEVER `SubmitRun`).

use std::path::PathBuf;

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;
use crate::verbs::invoke::{self, InvokeArgs};

/// Default `--wait` timeout (mirrors `kx invoke`).
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The `runs` subcommand.
#[derive(Debug)]
pub enum RunsSub {
    /// Durable run history (newest-first, paginated).
    List(ListSpec),
    /// Re-run a prior run with edited args ("Re-run with changes").
    Rerun(RerunSpec),
}

/// A `runs list` request.
#[derive(Debug)]
pub struct ListSpec {
    /// Page size (server clamps to its max page of 500).
    pub limit: Option<u32>,
    /// Pagination cursor: only runs with `registered_seq < before_seq`.
    pub before_seq: Option<u64>,
}

/// A `runs rerun` request.
#[derive(Debug)]
pub struct RerunSpec {
    /// The run to fork (16B instance id; its captured args are the baseline).
    pub instance: [u8; 16],
    /// Repeatable `--set k=v` arg overrides (applied over the captured args).
    pub set: Vec<(String, String)>,
    /// Run to completion and print the committed result (`--wait`).
    pub wait: bool,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Write the committed result bytes to this file instead of inlining them.
    pub out: Option<PathBuf>,
}

/// Parsed `runs` arguments.
#[derive(Debug)]
pub struct RunsArgs {
    /// The subcommand.
    pub sub: RunsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `runs` args (the verb already consumed). The first token selects the
/// subcommand (`list` | `rerun`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<RunsArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("runs requires a subcommand: list | rerun".into()))?;

    let mut common = ClientCommon::default();
    let mut limit: Option<u32> = None;
    let mut before_seq: Option<u64> = None;
    let mut instance: Option<[u8; 16]> = None;
    let mut set: Vec<(String, String)> = Vec::new();
    let mut wait = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut out: Option<PathBuf> = None;

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
            "--before-seq" => {
                let v = next_value(&mut args, "--before-seq")?;
                before_seq = Some(v.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!("--before-seq must be an integer, got {v:?}"))
                })?);
            }
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--set" => set.push(parse_set(&next_value(&mut args, "--set")?)?),
            "--wait" => wait = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            other => {
                // The rerun instance id may be given positionally (parity with
                // `kx mote show <instance-hex16> ...`).
                if other.starts_with("--") {
                    return Err(CliError::Usage(format!("unknown flag {other:?}")));
                }
                if instance.is_some() {
                    return Err(CliError::Usage(format!("unexpected argument {other:?}")));
                }
                instance = Some(crate::hex::decode_fixed::<16>(other)?);
            }
        }
    }

    let sub = match kw.as_str() {
        "list" => RunsSub::List(ListSpec { limit, before_seq }),
        "rerun" => {
            let instance = instance.ok_or_else(|| {
                CliError::Usage(
                    "runs rerun requires an instance id (hex16, positional or --instance)".into(),
                )
            })?;
            RunsSub::Rerun(RerunSpec {
                instance,
                set,
                wait,
                timeout_secs,
                out,
            })
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown runs subcommand {other:?} (expected: list | rerun)"
            )))
        }
    };
    Ok(RunsArgs { sub, common })
}

/// Split a `--set k=v` token on the FIRST `=` (the value may itself contain `=`).
fn parse_set(raw: &str) -> Result<(String, String), CliError> {
    match raw.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(CliError::Usage(format!(
            "--set expects key=value (got {raw:?})"
        ))),
    }
}

/// Execute `runs`.
pub async fn execute(args: RunsArgs) -> Result<(), CliError> {
    match args.sub {
        RunsSub::List(spec) => execute_list(args.common, spec).await,
        RunsSub::Rerun(spec) => execute_rerun(args.common, spec).await,
    }
}

/// Execute `runs list`.
async fn execute_list(common: ClientCommon, spec: ListSpec) -> Result<(), CliError> {
    let resolved = common.resolve()?;
    let mut client = resolved.connect().await?;
    let resp = client
        .list_runs(resolved.request(proto::ListRunsRequest {
            limit: spec.limit,
            before_seq: spec.before_seq,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_runs(&resp, common.json));
    Ok(())
}

/// Execute `runs rerun`: fetch the captured args, overlay `--set` edits, then
/// delegate to `kx invoke` (same admission, same `--wait`/`--out` handling).
async fn execute_rerun(common: ClientCommon, spec: RerunSpec) -> Result<(), CliError> {
    let resolved = common.resolve()?;
    let mut client = resolved.connect().await?;
    let inputs = client
        .get_run_inputs(resolved.request(proto::GetRunInputsRequest {
            instance_id: spec.instance.to_vec(),
        })?)
        .await
        .map_err(degrade)?
        .into_inner();

    let args_json = apply_overrides(&inputs.args, &spec.set)?;

    // A re-run is just a new Invoke with edited args — the SAME admission path
    // (never SubmitRun). Unchanged args dedup to the existing result; a changed
    // arg recomputes only the affected sub-DAG.
    invoke::execute(InvokeArgs {
        handle: inputs.handle,
        args_json,
        wait: spec.wait,
        // Re-run focuses on the dedup proof, not a live chat view — no streaming.
        stream: false,
        timeout_secs: spec.timeout_secs,
        out: spec.out,
        common,
    })
    .await
}

/// Overlay `--set k=v` edits onto the captured args JSON object. Each value is
/// parsed as JSON when valid (so `--set count=3` → number 3, `--set on=true` →
/// bool), else taken as a string (`--set topic=hello` → "hello"); the server
/// still validates/coerces against the recipe form fail-closed. Empty captured
/// args start from `{}`.
fn apply_overrides(captured: &[u8], set: &[(String, String)]) -> Result<Vec<u8>, CliError> {
    let mut obj: serde_json::Map<String, serde_json::Value> = if captured.is_empty() {
        serde_json::Map::new()
    } else {
        match serde_json::from_slice::<serde_json::Value>(captured) {
            Ok(serde_json::Value::Object(m)) => m,
            Ok(_) => {
                return Err(CliError::Usage(
                    "captured run args are not a JSON object (cannot --set)".into(),
                ))
            }
            Err(_) => {
                return Err(CliError::Usage(
                    "captured run args are not valid JSON (cannot --set)".into(),
                ))
            }
        }
    };
    for (k, v) in set {
        let value = serde_json::from_str::<serde_json::Value>(v)
            .unwrap_or_else(|_| serde_json::Value::String(v.clone()));
        obj.insert(k.clone(), value);
    }
    serde_json::to_vec(&serde_json::Value::Object(obj))
        .map_err(|e| CliError::Usage(format!("could not serialize edited args: {e}")))
}

/// Forward-compat degrade for `GetRunInputs`: an old gateway (no sidecar) answers
/// `Unimplemented`; a run with nothing captured (pre-PR-D / rebuilt-to-empty
/// sidecar) answers `NotFound` — say each honestly, suggesting `kx invoke`.
fn degrade(status: tonic::Status) -> CliError {
    match status.code() {
        Code::Unimplemented => CliError::Rpc {
            code: Code::Unimplemented,
            message: "re-run is not wired on this gateway (upgrade the serve, or use `kx invoke`)"
                .into(),
            refusal_code: None,
        },
        Code::NotFound => CliError::Rpc {
            code: Code::NotFound,
            message: "no captured args for this run (run it via `kx invoke` to capture them)"
                .into(),
            refusal_code: None,
        },
        _ => CliError::from_status(status),
    }
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
        match a.sub {
            RunsSub::List(s) => assert!(s.limit.is_none() && s.before_seq.is_none()),
            RunsSub::Rerun(_) => panic!("expected list"),
        }
        let a = p(&["list", "--limit", "10", "--before-seq", "42", "--json"]).unwrap();
        match a.sub {
            RunsSub::List(s) => {
                assert_eq!(s.limit, Some(10));
                assert_eq!(s.before_seq, Some(42));
            }
            RunsSub::Rerun(_) => panic!("expected list"),
        }
        assert!(a.common.json);
    }

    #[test]
    fn rerun_parses_instance_and_repeated_set() {
        let id = "ab".repeat(16);
        let a = p(&[
            "rerun", &id, "--set", "topic=hi", "--set", "count=3", "--wait",
        ])
        .unwrap();
        match a.sub {
            RunsSub::Rerun(s) => {
                assert_eq!(s.instance, [0xab; 16]);
                assert_eq!(
                    s.set,
                    vec![
                        ("topic".to_string(), "hi".to_string()),
                        ("count".to_string(), "3".to_string()),
                    ]
                );
                assert!(s.wait);
            }
            RunsSub::List(_) => panic!("expected rerun"),
        }
        // The instance id also parses via --instance.
        let a = p(&["rerun", "--instance", &id]).unwrap();
        assert!(matches!(a.sub, RunsSub::Rerun(s) if s.instance == [0xab; 16]));
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err());
        assert!(p(&["history"]).is_err());
        assert!(p(&["list", "--limit"]).is_err());
        assert!(p(&["list", "--limit", "many"]).is_err());
        assert!(p(&["list", "--bogus"]).is_err());
        assert!(p(&["rerun"]).is_err(), "rerun needs an instance id");
        assert!(
            p(&["rerun", &"ab".repeat(16), "--set", "novalue"]).is_err(),
            "--set needs key=value"
        );
        assert!(
            p(&["rerun", &"ab".repeat(16), "--set", "=bad"]).is_err(),
            "--set needs a non-empty key"
        );
    }

    #[test]
    fn apply_overrides_merges_typed_values() {
        let base = br#"{"topic":"a","count":1}"#;
        let out = apply_overrides(
            base,
            &[
                ("topic".into(), "b".into()),
                ("count".into(), "5".into()),
                ("flag".into(), "true".into()),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["topic"], serde_json::json!("b"));
        assert_eq!(v["count"], serde_json::json!(5));
        assert_eq!(v["flag"], serde_json::json!(true));
    }

    #[test]
    fn apply_overrides_handles_empty_captured() {
        let out = apply_overrides(b"", &[("topic".into(), "hi".into())]).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["topic"], serde_json::json!("hi"));
    }

    #[test]
    fn apply_overrides_rejects_non_object_args() {
        assert!(apply_overrides(b"[1,2,3]", &[]).is_err());
        assert!(apply_overrides(b"not json", &[]).is_err());
    }
}
