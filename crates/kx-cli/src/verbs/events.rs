//! `kx events --instance <hex16> [--since N] [--follow]` — print a run's event
//! deltas. Without `--follow` it reads one snapshot (`since_seq` → the current
//! journal boundary) and exits. With `--follow` (R5) it consumes the server's
//! LIVE TAIL: one open `StreamEvents` stream that keeps delivering frames as the
//! journal advances, until Ctrl-C. If the server drops a slow consumer with
//! `resource_exhausted` (CatchupRequired), the client transparently reconnects
//! from its last `next_seq` — no lost or duplicated delta.
//!
//! `kx events --all` (Batch C) streams the operator-global cross-run tail
//! instead (`StreamAllEvents`): the same snapshot/follow/resume contract, with
//! each delta stamped with its run's `instance_id` (watermark attribution —
//! EMPTY before any registration) plus the `run_registered` "run started"
//! marker the per-run cursor never carries. Mutually exclusive with
//! `--instance`; the frozen per-run path is untouched.

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::Code;

use crate::client::{next_value, take_fixed, ClientCommon, Resolved};
use crate::error::CliError;
use crate::format;

/// What to stream: one run's frozen per-run cursor, or the global tail.
#[derive(Debug)]
pub enum EventsTarget {
    /// One run's deltas (`StreamEvents`, 16B instance id).
    Run([u8; 16]),
    /// The operator-global cross-run tail (`StreamAllEvents`, Batch C).
    All,
}

/// Parsed `events` arguments.
#[derive(Debug)]
pub struct EventsArgs {
    /// The stream to consume (`--instance <hex16>` xor `--all`).
    pub target: EventsTarget,
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
    let mut all = false;
    let mut since: u64 = 0;
    let mut follow = false;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--all" => all = true,
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

    let target =
        match (instance, all) {
            (Some(id), false) => EventsTarget::Run(id),
            (None, true) => EventsTarget::All,
            (Some(_), true) => return Err(CliError::Usage(
                "--instance and --all are mutually exclusive: use `kx events --instance <hex16>` \
                 for one run, or `kx events --all` for the global cross-run tail"
                    .into(),
            )),
            (None, false) => return Err(CliError::Usage(
                "events requires --instance <hex16> (one run) or --all (the global cross-run tail)"
                    .into(),
            )),
        };
    Ok(EventsArgs {
        target,
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

    match args.target {
        EventsTarget::Run(instance) => {
            if !args.follow {
                let cursor =
                    drain_once(&mut client, &resolved, &instance, args.since, json).await?;
                if !json {
                    println!("-- caught up at seq {cursor} --");
                }
                return Ok(());
            }
            follow_live(&mut client, &resolved, &instance, args.since, json).await
        }
        EventsTarget::All => {
            if !args.follow {
                let cursor = drain_all_once(&mut client, &resolved, args.since, json).await?;
                if !json {
                    println!("-- caught up at seq {cursor} --");
                }
                return Ok(());
            }
            follow_all_live(&mut client, &resolved, args.since, json).await
        }
    }
}

/// Consume the server's live tail: ONE open `StreamEvents` stream, printing deltas
/// as they arrive until Ctrl-C. On a `resource_exhausted` (CatchupRequired) drop,
/// reconnect from the last `next_seq` (resume — no lost/duplicated delta).
async fn follow_live(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    instance: &[u8; 16],
    mut cursor: u64,
    json: bool,
) -> Result<(), CliError> {
    loop {
        let mut stream = client
            .stream_events(resolved.request(proto::StreamEventsRequest {
                instance_id: instance.to_vec(),
                since_seq: cursor,
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner();

        loop {
            tokio::select! {
                message = stream.message() => match message {
                    Ok(Some(frame)) => {
                        for delta in &frame.deltas {
                            if let Some(line) = format::render_delta(delta, json) {
                                println!("{line}");
                            }
                        }
                        cursor = frame.next_seq;
                    }
                    // The live tail does not end on its own; a clean end means the
                    // server is snapshot-only — we are done.
                    Ok(None) => return Ok(()),
                    // CatchupRequired: the server dropped a slow consumer. Resume
                    // from the last acked cursor.
                    Err(status) if status.code() == Code::ResourceExhausted => break,
                    Err(status) => return Err(CliError::from_status(status)),
                },
                _ = tokio::signal::ctrl_c() => return Ok(()),
            }
        }
    }
}

/// The `--all` twin of [`drain_once`]: one global snapshot (`since_seq` →
/// head), printing each attributed delta; return the caught-up cursor.
async fn drain_all_once(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    since: u64,
    json: bool,
) -> Result<u64, CliError> {
    let mut stream = client
        .stream_all_events(resolved.request(proto::StreamAllEventsRequest { since_seq: since })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    let mut cursor = since;
    while let Some(frame) = stream.message().await.map_err(CliError::from_status)? {
        for delta in &frame.deltas {
            println!("{}", format::render_global_delta(delta, json));
        }
        cursor = frame.next_seq;
        if frame.journal_boundary {
            break;
        }
    }
    Ok(cursor)
}

/// The `--all` twin of [`follow_live`]: one open `StreamAllEvents` stream,
/// printing attributed deltas until Ctrl-C; on a `resource_exhausted`
/// slow-consumer drop, resume from the last `next_seq` (the server's own
/// catch-up instruction) — no lost or duplicated delta.
async fn follow_all_live(
    client: &mut KxGatewayClient<Channel>,
    resolved: &Resolved,
    mut cursor: u64,
    json: bool,
) -> Result<(), CliError> {
    loop {
        let mut stream = client
            .stream_all_events(
                resolved.request(proto::StreamAllEventsRequest { since_seq: cursor })?,
            )
            .await
            .map_err(CliError::from_status)?
            .into_inner();

        loop {
            tokio::select! {
                message = stream.message() => match message {
                    Ok(Some(frame)) => {
                        for delta in &frame.deltas {
                            println!("{}", format::render_global_delta(delta, json));
                        }
                        cursor = frame.next_seq;
                    }
                    // The live tail does not end on its own; a clean end means the
                    // server is snapshot-only — we are done.
                    Ok(None) => return Ok(()),
                    // CatchupRequired: the server dropped a slow consumer. Resume
                    // from the last acked cursor.
                    Err(status) if status.code() == Code::ResourceExhausted => break,
                    Err(status) => return Err(CliError::from_status(status)),
                },
                _ = tokio::signal::ctrl_c() => return Ok(()),
            }
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
        assert!(matches!(a.target, EventsTarget::Run(id) if id == [0xab; 16]));
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
    fn parses_all_with_since_follow_and_json() {
        let a = p(&["--all"]).unwrap();
        assert!(matches!(a.target, EventsTarget::All));
        assert_eq!(a.since, 0);
        assert!(!a.follow && !a.common.json);
        let a = p(&["--all", "--since", "9", "--follow", "--json"]).unwrap();
        assert!(matches!(a.target, EventsTarget::All));
        assert_eq!(a.since, 9);
        assert!(a.follow && a.common.json);
    }

    #[test]
    fn all_and_instance_are_mutually_exclusive() {
        let err = p(&["--all", "--instance", &"ab".repeat(16)]).unwrap_err();
        assert!(
            err.to_string().contains("mutually exclusive"),
            "the error points at the right form: {err}"
        );
        // Order does not matter.
        assert!(p(&["--instance", &"ab".repeat(16), "--all"]).is_err());
    }

    #[test]
    fn missing_target_bad_since_unknown_flag_are_usage() {
        let err = p(&["--follow"]).unwrap_err();
        assert!(
            err.to_string().contains("--instance") && err.to_string().contains("--all"),
            "the no-target error names both forms: {err}"
        );
        assert!(p(&["--instance", &"ab".repeat(16), "--since", "x"]).is_err());
        assert!(p(&["--instance", &"ab".repeat(16), "--nope"]).is_err());
        // A wrong-length instance is rejected (32 hex chars = 16 bytes required).
        assert!(p(&["--instance", &"ab".repeat(32)]).is_err());
    }
}
