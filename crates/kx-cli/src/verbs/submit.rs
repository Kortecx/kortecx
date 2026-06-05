//! `kx submit --demo [--wait] ...` — submit a built-in PURE demo run via the
//! low-level SubmitRun path (the request shape comes from
//! [`kx_gateway::demo_submit_run_request`], the single source of truth shared
//! with the gateway e2e). Arbitrary run authoring is not a v0.1.0 CLI capability
//! (recipes + SDKs are the authoring path); `invoke` is the real recipe path.

use std::path::PathBuf;
use std::time::Duration;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::{format, verbs, wait};

/// Default `--wait` timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Parsed `submit` arguments.
#[derive(Debug)]
pub struct SubmitArgs {
    /// Run to completion and print the committed result (`--wait`).
    pub wait: bool,
    /// `--wait` timeout in seconds.
    pub timeout_secs: u64,
    /// Write the committed result bytes to this file instead of inlining them.
    pub out: Option<PathBuf>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `submit` args (the verb already consumed). `--demo` is currently the
/// only mode (stated honestly: a richer SubmitRun authoring path arrives with
/// recipes/SDKs).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<SubmitArgs, CliError> {
    let mut demo = false;
    let mut wait = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut out: Option<PathBuf> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--demo" => demo = true,
            "--wait" => wait = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    if !demo {
        return Err(CliError::Usage(
            "submit currently supports only --demo (use `invoke` to run a published recipe)".into(),
        ));
    }
    Ok(SubmitArgs {
        wait,
        timeout_secs,
        out,
        common,
    })
}

/// Execute `submit --demo`.
pub async fn execute(args: SubmitArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let handle = client
        .submit_run(resolved.request(kx_gateway::demo_submit_run_request())?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    if args.wait {
        let outcome = wait::await_any_result(
            &mut client,
            &resolved,
            handle.instance_id,
            Duration::from_secs(args.timeout_secs),
        )
        .await?;
        verbs::finish_wait(&outcome, args.common.json, args.out.as_deref())
    } else {
        println!("{}", format::render_submit(&handle, args.common.json));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<SubmitArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn requires_demo() {
        assert!(p(&[]).is_err(), "submit without --demo is a usage error");
    }

    #[test]
    fn parses_demo_wait_and_common() {
        let a = p(&["--demo", "--wait", "--json", "--timeout-secs", "30"]).unwrap();
        assert!(a.wait && a.common.json);
        assert_eq!(a.timeout_secs, 30);
    }

    #[test]
    fn unknown_flag_and_bad_timeout_are_usage() {
        assert!(p(&["--demo", "--nope"]).is_err());
        assert!(p(&["--demo", "--timeout-secs", "soon"]).is_err());
    }
}
