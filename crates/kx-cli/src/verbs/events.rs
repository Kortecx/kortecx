//! `kx events --instance <hex16> [--since N] [--follow]` — print a run's event
//! deltas. `StreamEvents` is snapshot-to-head today (it catches up to the
//! current journal boundary and ends); `--follow` re-polls from the last cursor
//! on a bounded backoff until Ctrl-C. True live-tail arrives with the R5
//! WebSocket bridge — the verb surface is unchanged when it does.

use std::time::Duration;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use crate::client::{next_value, take_fixed, ClientCommon, Resolved};
use crate::error::CliError;
use crate::format;

/// Re-poll cadence under `--follow` (bounded backoff — never a busy-spin).
const FOLLOW_POLL: Duration = Duration::from_millis(250);

/// Parsed `events` arguments.
#[derive(Debug)]
pub struct EventsArgs {
    /// The run to stream (16B instance id).
    pub instance: [u8; 16],
    /// Resume cursor (0 = from start).
    pub since: u64,
    /// Keep polling from the last cursor until Ctrl-C.
    pub follow: bool,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `events` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<EventsArgs, CliError> {
    let mut instance: Option<[u8; 16]> = None;
    let mut since: u64 = 0;
    let mut follow = false;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--since" => {
                let v = next_value(&mut args, "--since")?;
                since = v.parse().map_err(|_| {
                    CliError::Usage(format!("--since expects an integer, got {v:?}"))
                })?;
            }
            "--follow" => follow = true,
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let instance =
        instance.ok_or_else(|| CliError::Usage("events requires --instance <hex16>".into()))?;
    Ok(EventsArgs {
        instance,
        since,
        follow,
        common,
    })
}

/// Read one snapshot (`since_seq` → head), printing each delta; return the
/// caught-up cursor (`next_seq` at the journal boundary).
async fn drain_once(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance: &[u8; 16],
    since: u64,
    json: bool,
) -> Result<u64, CliError> {
    let mut stream = client
        .stream_events(resolved.request(proto::StreamEventsRequest {
            instance_id: instance.to_vec(),
            since_seq: since,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    let mut cursor = since;
    while let Some(frame) = stream.message().await.map_err(CliError::from_status)? {
        for delta in &frame.deltas {
            if let Some(line) = format::render_delta(delta, json) {
                println!("{line}");
            }
        }
        cursor = frame.next_seq;
        if frame.journal_boundary {
            break;
        }
    }
    Ok(cursor)
}

/// Execute `events`.
pub async fn execute(args: EventsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    let mut cursor = args.since;
    loop {
        cursor = drain_once(&mut client, &resolved, &args.instance, cursor, json).await?;
        if !args.follow {
            if !json {
                println!("-- caught up at seq {cursor} --");
            }
            return Ok(());
        }
        // --follow: back off, then re-poll from the cursor; a clean Ctrl-C exits.
        tokio::select! {
            () = tokio::time::sleep(FOLLOW_POLL) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<EventsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_instance_since_follow() {
        let a = p(&["--instance", &"ab".repeat(16), "--since", "7", "--follow"]).unwrap();
        assert_eq!(a.instance, [0xab; 16]);
        assert_eq!(a.since, 7);
        assert!(a.follow);
    }

    #[test]
    fn defaults_since_zero_no_follow() {
        let a = p(&["--instance", &"ab".repeat(16)]).unwrap();
        assert_eq!(a.since, 0);
        assert!(!a.follow);
    }

    #[test]
    fn missing_instance_bad_since_unknown_flag_are_usage() {
        assert!(p(&["--follow"]).is_err(), "no --instance");
        assert!(p(&["--instance", &"ab".repeat(16), "--since", "x"]).is_err());
        assert!(p(&["--instance", &"ab".repeat(16), "--nope"]).is_err());
    }
}
