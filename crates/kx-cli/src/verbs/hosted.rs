// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx hosted start | stop | status | list` — the hosted-App lifecycle, at
//! parity with the console.
//!
//! ## A hosted App is a confined host subprocess, not a Mote
//!
//! Start materializes the App's branch file tree into a working dir, installs,
//! and spawns the framework's server on a LOOPBACK port. There is no reverse
//! proxy: the reported `url` is the absolute `http://127.0.0.1:<port>/` origin
//! the app actually answers on. Nothing here is journaled, and none of it can
//! move the canonical digest.
//!
//! ## Start is asynchronous, and this verb says so
//!
//! `start` returns as soon as the supervisor has accepted the request, with the
//! app typically still `MATERIALIZING` or `INSTALLING`. `--wait` polls `status`
//! until the app is RUNNING or FAILED rather than making the caller write that
//! loop — and it reports the FAILED detail rather than timing out silently,
//! because "it never came up" and "it came up and died" need different fixes.

use std::time::{Duration, Instant};

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// How long `--wait` polls before giving up, when no `--timeout-secs` is given.
const DEFAULT_WAIT_SECS: u64 = 180;

/// How often `--wait` re-checks. An install is tens of seconds; polling faster
/// buys nothing and costs the supervisor a round trip per tick.
const POLL_INTERVAL_MS: u64 = 1_000;

/// The `hosted` subcommand.
#[derive(Debug)]
pub enum HostedSub {
    /// Start (or restart) the hosted server for a saved App.
    Start {
        /// The saved App handle.
        handle: String,
        /// Force a fresh install/build even if the working dir looks current.
        rebuild: bool,
        /// Poll until RUNNING or FAILED.
        wait: bool,
        /// `--wait` ceiling in seconds.
        timeout_secs: u64,
    },
    /// Stop the hosted server (kills and reaps the child process group).
    Stop {
        /// The saved App handle.
        handle: String,
    },
    /// Report one hosted App's state, URL and recent logs.
    Status {
        /// The saved App handle.
        handle: String,
    },
    /// List every hosted App the caller owns.
    List,
}

/// Parsed `hosted` arguments.
#[derive(Debug)]
pub struct HostedArgs {
    /// The subcommand.
    pub sub: HostedSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `hosted` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<HostedArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("hosted requires a subcommand: start | stop | status | list".into())
    })?;

    let mut common = ClientCommon::default();
    let mut positional: Vec<String> = Vec::new();
    let mut rebuild = false;
    let mut wait = false;
    let mut timeout_secs = DEFAULT_WAIT_SECS;

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--rebuild" => rebuild = true,
            "--wait" => wait = true,
            "--timeout-secs" => {
                let v = next_value(&mut args, "--timeout-secs")?;
                timeout_secs = v.parse().map_err(|_| {
                    CliError::Usage(format!("--timeout-secs expects an integer, got {v:?}"))
                })?;
            }
            other if !other.starts_with("--") => positional.push(other.to_string()),
            other => return Err(CliError::Usage(format!("unknown flag {other}"))),
        }
    }

    let handle = |verb: &str, p: &mut Vec<String>| -> Result<String, CliError> {
        if p.is_empty() {
            return Err(CliError::Usage(format!(
                "hosted {verb} requires a <handle> (namespace/collection/name)"
            )));
        }
        Ok(p.remove(0))
    };

    let sub = match kw.as_str() {
        "start" => HostedSub::Start {
            handle: handle("start", &mut positional)?,
            rebuild,
            wait,
            timeout_secs,
        },
        "stop" => HostedSub::Stop {
            handle: handle("stop", &mut positional)?,
        },
        "status" => HostedSub::Status {
            handle: handle("status", &mut positional)?,
        },
        "list" => HostedSub::List,
        other => {
            return Err(CliError::Usage(format!(
                "unknown hosted subcommand {other:?}: expected start | stop | status | list"
            )))
        }
    };

    Ok(HostedArgs { sub, common })
}

/// Run the parsed `hosted` subcommand.
pub async fn execute(args: HostedArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        HostedSub::Start {
            handle,
            rebuild,
            wait,
            timeout_secs,
        } => {
            let status = client
                .start_hosted_app(resolved.request(proto::StartHostedAppRequest {
                    handle: handle.clone(),
                    rebuild,
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();

            if !wait {
                println!("{}", format::render_hosted_status(&status, json));
                return Ok(());
            }

            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            let mut last = status;
            loop {
                if is_terminal(last.state) {
                    println!("{}", format::render_hosted_status(&last, json));
                    // A FAILED app is a FAILED command. Printing the detail and
                    // exiting 0 would make `kx hosted start --wait && …` run the
                    // next step against an app that is not there.
                    if last.state == proto::HostedAppState::HostedFailed as i32 {
                        return Err(CliError::Runtime(format!(
                            "hosted app {handle:?} failed to start: {}",
                            if last.detail.is_empty() {
                                "no detail reported"
                            } else {
                                &last.detail
                            }
                        )));
                    }
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    // Report the state it was STUCK in — "timed out" alone does
                    // not distinguish a slow install from a server that never
                    // bound its port.
                    return Err(CliError::Runtime(format!(
                        "hosted app {handle:?} still {} after {timeout_secs}s",
                        format::hosted_state_name(last.state)
                    )));
                }
                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
                last = client
                    .get_hosted_app_status(resolved.request(proto::GetHostedAppStatusRequest {
                        handle: handle.clone(),
                    })?)
                    .await
                    .map_err(CliError::from_status)?
                    .into_inner();
            }
        }

        HostedSub::Stop { handle } => {
            let resp = client
                .stop_hosted_app(resolved.request(proto::StopHostedAppRequest { handle })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!("{}", serde_json::json!({ "stopped": resp.stopped }));
            } else if resp.stopped {
                println!("stopped");
            } else {
                println!("not running");
            }
            Ok(())
        }

        HostedSub::Status { handle } => {
            let status = client
                .get_hosted_app_status(
                    resolved.request(proto::GetHostedAppStatusRequest { handle })?,
                )
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_hosted_status(&status, json));
            Ok(())
        }

        HostedSub::List => {
            let resp = client
                .list_hosted_apps(resolved.request(proto::ListHostedAppsRequest {})?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_hosted_list(&resp, json));
            Ok(())
        }
    }
}

/// RUNNING and FAILED are the only states `--wait` stops on. Everything else is
/// a stage of coming up.
fn is_terminal(state: i32) -> bool {
    state == proto::HostedAppState::HostedRunning as i32
        || state == proto::HostedAppState::HostedFailed as i32
}

#[cfg(test)]
mod tests {
    use super::{is_terminal, parse, HostedSub};
    use kx_proto::proto;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn start_takes_a_handle_and_flags() {
        let a = parse(v(&["start", "ns/coll/app", "--rebuild", "--wait"]).into_iter()).unwrap();
        match a.sub {
            HostedSub::Start {
                handle,
                rebuild,
                wait,
                ..
            } => {
                assert_eq!(handle, "ns/coll/app");
                assert!(rebuild);
                assert!(wait);
            }
            other => panic!("expected Start, got {other:?}"),
        }
    }

    #[test]
    fn list_takes_no_handle() {
        let a = parse(v(&["list"]).into_iter()).unwrap();
        assert!(matches!(a.sub, HostedSub::List));
    }

    #[test]
    fn a_missing_handle_is_a_usage_error_that_names_the_shape() {
        let err = parse(v(&["status"]).into_iter()).unwrap_err();
        assert!(err.to_string().contains("namespace/collection/name"));
    }

    #[test]
    fn an_unknown_subcommand_names_the_alternatives() {
        let err = parse(v(&["restart"]).into_iter()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("restart"));
        assert!(msg.contains("start | stop | status | list"));
    }

    /// `--wait` must stop on exactly the two settled states. Treating a
    /// mid-flight state as terminal would report "started" during `npm install`.
    #[test]
    fn only_running_and_failed_are_terminal() {
        assert!(is_terminal(proto::HostedAppState::HostedRunning as i32));
        assert!(is_terminal(proto::HostedAppState::HostedFailed as i32));
        for mid in [
            proto::HostedAppState::HostedStopped,
            proto::HostedAppState::HostedMaterializing,
            proto::HostedAppState::HostedInstalling,
            proto::HostedAppState::HostedStarting,
            proto::HostedAppState::HostedBuilding,
        ] {
            assert!(!is_terminal(mid as i32), "{mid:?} is not terminal");
        }
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = parse(v(&["list", "--json"]).into_iter()).unwrap();
        assert!(a.common.json);
    }
}
