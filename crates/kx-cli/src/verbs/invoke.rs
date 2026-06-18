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
    /// Stream the terminal model mote's ADVISORY tokens to stdout as they
    /// generate (`--stream`, PR-4.2 / T-STREAM1), then resolve the committed
    /// result. Implies awaiting the run; safe on a token-less terminal.
    pub stream: bool,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Write the committed result bytes to this file instead of inlining them.
    pub out: Option<PathBuf>,
    /// PR-7: context-bundle handles to attach (`--context <handle>`, repeatable).
    /// The server resolves each to its item refs and injects them into the entry
    /// Mote's identity-bearing context (a different context ⇒ a different run).
    pub context_bundles: Vec<String>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `invoke` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<InvokeArgs, CliError> {
    let mut handle: Option<String> = None;
    let mut args_json: Option<Vec<u8>> = None;
    let mut wait = false;
    let mut stream = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut out: Option<PathBuf> = None;
    let mut context_bundles: Vec<String> = Vec::new();
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
            "--stream" => stream = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            "--context" => context_bundles.push(next_value(&mut args, "--context")?),
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
        stream,
        timeout_secs,
        out,
        context_bundles,
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
            context_bundles: args.context_bundles,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if args.stream {
        // PR-4.2 (T-STREAM1): print the terminal model mote's ADVISORY tokens
        // live while CONCURRENTLY awaiting the committed result. The run settling
        // ends the loop even when the terminal is token-less (a non-model mote /
        // broker-unwired serve), so `--stream` never hangs. The committed result
        // stays the authority — surfaced via `finish_wait` for --json / --out.
        use std::io::Write;
        let mut tokens = client
            .stream_model_tokens(resolved.request(proto::StreamModelTokensRequest {
                instance_id: resp.instance_id.clone(),
                mote_id: resp.terminal_mote_id.clone(),
                since_seq: 0,
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner();
        let wait_fut = wait::await_result(
            &mut client,
            &resolved,
            resp.instance_id.clone(),
            resp.terminal_mote_id.clone(),
            Duration::from_secs(args.timeout_secs),
        );
        tokio::pin!(wait_fut);
        let mut stdout = std::io::stdout();
        let outcome = loop {
            tokio::select! {
                msg = tokens.message() => match msg.map_err(CliError::from_status)? {
                    Some(chunk) if !chunk.done => {
                        let _ = stdout.write_all(&chunk.text_piece);
                        let _ = stdout.flush();
                    }
                    // `done` or end-of-stream: keep awaiting the committed result.
                    _ => {}
                },
                res = &mut wait_fut => break res?,
            }
        };
        let _ = writeln!(stdout);
        return if args.common.json || args.out.is_some() {
            verbs::finish_wait(&outcome, args.common.json, args.out.as_deref())
        } else {
            // The live tokens were the human-facing output; don't re-dump.
            Ok(())
        };
    }

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
    fn parses_stream_flag() {
        let a = parse_v(&["kx/recipes/chat", "--args", "{}", "--stream"]).unwrap();
        assert!(a.stream);
        assert!(!a.wait, "--stream does not require --wait");
        // --stream composes with --wait / --json.
        let b = parse_v(&["kx/recipes/chat", "--args", "{}", "--stream", "--json"]).unwrap();
        assert!(b.stream && b.common.json);
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
