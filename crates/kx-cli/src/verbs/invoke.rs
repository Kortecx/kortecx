//! `kx invoke <handle> --args <json> [--wait] ...` — bind a PUBLISHED blueprint
//! (wire-legacy: recipe) by handle to JSON args and run it. The CLI sends only
//! `handle` + raw args bytes; the server does resolve → validate → bind →
//! intersect → submit (fail-closed).

use std::path::PathBuf;
use std::time::Duration;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, verbs, wait};

/// Default `--wait` timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Parsed `invoke` arguments.
#[derive(Debug)]
pub struct InvokeArgs {
    /// The blueprint handle (`namespace/collection/name`; wire-legacy: recipe).
    pub handle: String,
    /// Opaque JSON args bytes (validated server-side; sanity-checked client-side).
    pub args_json: Vec<u8>,
    /// Run to completion and print the committed result (`--wait`).
    pub wait: bool,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Write the committed result bytes to this file instead of inlining them.
    pub out: Option<PathBuf>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `invoke` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<InvokeArgs, CliError> {
    let mut handle: Option<String> = None;
    let mut args_json: Option<Vec<u8>> = None;
    let mut wait = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut out: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--args" => {
                reject_duplicate_args(args_json.is_some())?;
                args_json = Some(next_value(&mut args, "--args")?.into_bytes());
            }
            "--args-file" => {
                reject_duplicate_args(args_json.is_some())?;
                let p = next_value(&mut args, "--args-file")?;
                args_json = Some(
                    std::fs::read(&p).map_err(|e| CliError::Io(format!("--args-file {p}: {e}")))?,
                );
            }
            "--wait" => wait = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            other => {
                if other.starts_with("--") {
                    return Err(CliError::Usage(format!("unknown flag {other:?}")));
                }
                if handle.is_some() {
                    return Err(CliError::Usage(format!("unexpected argument {other:?}")));
                }
                handle = Some(other.to_string());
            }
        }
    }

    let handle = handle.ok_or_else(|| {
        CliError::Usage("invoke requires a blueprint handle (e.g. kx/recipes/echo)".into())
    })?;
    let args_json = args_json.ok_or_else(|| {
        CliError::Usage("invoke requires --args <json> (or --args-file <path>)".into())
    })?;
    // Fail fast on client-side-invalid JSON (before the round trip). Valid-but-
    // wrong JSON (e.g. a missing field) is still the server's fail-closed call.
    // The message names neither flag (the bytes may have come from either source).
    if serde_json::from_slice::<serde_json::Value>(&args_json).is_err() {
        return Err(CliError::Usage("the args are not valid JSON".into()));
    }

    Ok(InvokeArgs {
        handle,
        args_json,
        wait,
        timeout_secs,
        out,
        common,
    })
}

/// Reject a second args source: `--args` and `--args-file` set the same slot, so
/// silently last-wins would be a footgun (mirrors the `--token`/`--token-file`
/// guard in [`crate::client`]).
fn reject_duplicate_args(already_set: bool) -> Result<(), CliError> {
    if already_set {
        return Err(CliError::Usage(
            "--args and --args-file are mutually exclusive (give exactly one)".into(),
        ));
    }
    Ok(())
}

/// Execute `invoke`.
pub async fn execute(args: InvokeArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let resp = client
        .invoke(resolved.request(proto::InvokeRequest {
            handle: args.handle,
            args: args.args_json,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if args.wait {
        let outcome = wait::await_result(
            &mut client,
            &resolved,
            resp.instance_id,
            resp.terminal_mote_id,
            Duration::from_secs(args.timeout_secs),
        )
        .await?;
        verbs::finish_wait(&outcome, args.common.json, args.out.as_deref())
    } else {
        println!("{}", format::render_invoke(&resp, args.common.json));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_v(parts: &[&str]) -> Result<InvokeArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_handle_and_args() {
        let a = parse_v(&["kx/recipes/echo", "--args", r#"{"topic":"x"}"#]).unwrap();
        assert_eq!(a.handle, "kx/recipes/echo");
        assert_eq!(a.args_json, br#"{"topic":"x"}"#);
        assert!(!a.wait && a.out.is_none());
    }

    #[test]
    fn handle_can_follow_flags() {
        // The positional handle is recognized wherever it appears.
        let a = parse_v(&["--args", "{}", "kx/recipes/echo", "--wait"]).unwrap();
        assert_eq!(a.handle, "kx/recipes/echo");
        assert!(a.wait);
    }

    #[test]
    fn missing_handle_or_args_is_usage() {
        assert!(parse_v(&["--args", "{}"]).is_err(), "no handle");
        assert!(parse_v(&["kx/recipes/echo"]).is_err(), "no args");
    }

    #[test]
    fn args_and_args_file_are_mutually_exclusive() {
        // The fix for the reviewed defect: a second args source is rejected, not
        // silently shadowed.
        let err =
            parse_v(&["kx/recipes/echo", "--args", "{}", "--args-file", "/tmp/x"]).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        // A duplicate --args is likewise rejected.
        assert!(parse_v(&["kx/recipes/echo", "--args", "{}", "--args", "{}"]).is_err());
    }

    #[test]
    fn client_side_invalid_json_is_usage() {
        assert!(parse_v(&["kx/recipes/echo", "--args", "{"]).is_err());
    }

    #[test]
    fn unknown_flag_and_extra_positional_are_usage() {
        assert!(parse_v(&["kx/recipes/echo", "--args", "{}", "--nope"]).is_err());
        assert!(
            parse_v(&["a/b/c", "x/y/z", "--args", "{}"]).is_err(),
            "two handles"
        );
    }

    #[test]
    fn bad_timeout_is_usage() {
        assert!(parse_v(&["kx/recipes/echo", "--args", "{}", "--timeout-secs", "soon"]).is_err());
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = parse_v(&[
            "kx/recipes/echo",
            "--args",
            "{}",
            "--endpoint",
            "http://h:1",
            "--json",
        ])
        .unwrap();
        assert_eq!(a.common.endpoint, "http://h:1");
        assert!(a.common.json);
    }
}
