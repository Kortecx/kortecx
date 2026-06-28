//! `kx approvals list | grant | deny` — the HITL pre-action approval gate's operator
//! control plane (D114) over the gateway RPCs (`ListPendingApprovals` /
//! `GrantApproval` / `DenyApproval`). Tri-surface parity with the UI + SDK.
//!
//! A world-mutating tool call on a chain that requires approval is held
//! staged-not-committed until an operator GRANTS it (it then fires exactly once) or
//! DENIES it (the chain dead-letters fail-closed). SN-8: grant/deny are operator
//! decisions over a SERVER-derived `request_id` (the bytes shown by `list`); they
//! never mint a client warrant.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::hex;

/// The `approvals` subcommand.
#[derive(Debug)]
pub enum ApprovalsSub {
    /// List the world-mutating actions withheld awaiting approval.
    List,
    /// Grant a pending approval (releases the staged action to fire exactly once).
    Grant {
        /// The hex `request_id` (from `list`).
        request_id: String,
        /// Optional operator note.
        reason: String,
    },
    /// Deny a pending approval (the gated chain dead-letters fail-closed).
    Deny {
        /// The hex `request_id` (from `list`).
        request_id: String,
        /// Optional operator note.
        reason: String,
    },
}

/// Parsed `approvals` arguments.
#[derive(Debug)]
pub struct ApprovalsArgs {
    /// The subcommand.
    pub sub: ApprovalsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `approvals` args (the verb already consumed). The first token selects the
/// subcommand (`list` / `grant` / `deny`); `grant`/`deny` take a positional
/// `<REQUEST_ID>` (hex) + an optional `--reason`.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ApprovalsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("approvals requires a subcommand: list | grant | deny".into())
    })?;

    let mut request_id: Option<String> = None;
    let mut reason = String::new();
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--request-id" => request_id = Some(next_value(&mut args, "--request-id")?),
            "--reason" => reason = next_value(&mut args, "--reason")?,
            // A bare token is the positional request id (the `kx runs rerun <id>` shape).
            other if !other.starts_with("--") && request_id.is_none() => {
                request_id = Some(other.to_string());
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let require_id = |id: Option<String>, verb: &str| -> Result<String, CliError> {
        id.filter(|s| !s.is_empty()).ok_or_else(|| {
            CliError::Usage(format!("approvals {verb} requires a <REQUEST_ID> (hex)"))
        })
    };

    let sub = match kw.as_str() {
        "list" => ApprovalsSub::List,
        "grant" => ApprovalsSub::Grant {
            request_id: require_id(request_id, "grant")?,
            reason,
        },
        "deny" => ApprovalsSub::Deny {
            request_id: require_id(request_id, "deny")?,
            reason,
        },
        other => {
            return Err(CliError::Usage(format!(
                "unknown approvals subcommand {other:?} (expected list | grant | deny)"
            )))
        }
    };
    Ok(ApprovalsArgs { sub, common })
}

/// Execute `approvals`.
pub async fn execute(args: ApprovalsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        ApprovalsSub::List => {
            let req = proto::ListPendingApprovalsRequest { limit: 0 };
            let resp = client
                .list_pending_approvals(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                let rows: Vec<_> = resp
                    .approvals
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "request_id": hex::encode(&a.request_id),
                            "instance_id": hex::encode(&a.instance_id),
                            "mote_id": hex::encode(&a.mote_id),
                            "tool_id": a.tool_id,
                            "tool_version": a.tool_version,
                            "intent": a.intent,
                            "created_unix_ms": a.created_unix_ms,
                        })
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(rows));
            } else if resp.approvals.is_empty() {
                println!("no pending approvals");
            } else {
                for a in &resp.approvals {
                    println!(
                        "{}  {}@{}  {}",
                        hex::encode(&a.request_id),
                        a.tool_id,
                        a.tool_version,
                        a.intent
                    );
                }
            }
        }
        ApprovalsSub::Grant { request_id, reason } => {
            let req = proto::GrantApprovalRequest {
                request_id: hex::decode_fixed::<16>(&request_id)?.to_vec(),
                reason,
            };
            let resp = client
                .grant_approval(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            print_decision("granted", resp.granted, json);
        }
        ApprovalsSub::Deny { request_id, reason } => {
            let req = proto::DenyApprovalRequest {
                request_id: hex::decode_fixed::<16>(&request_id)?.to_vec(),
                reason,
            };
            let resp = client
                .deny_approval(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            print_decision("denied", resp.denied, json);
        }
    }
    Ok(())
}

fn print_decision(verb: &str, ok: bool, json: bool) {
    if json {
        println!("{}", serde_json::json!({ verb: ok }));
    } else if ok {
        println!("{verb}");
    } else {
        println!("not {verb} (unknown or already-resolved request)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ApprovalsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_list_grant_deny() {
        assert!(matches!(p(&["list"]).unwrap().sub, ApprovalsSub::List));
        let g = p(&["grant", "abcd", "--reason", "ok"]).unwrap();
        let ApprovalsSub::Grant { request_id, reason } = g.sub else {
            panic!("expected Grant");
        };
        assert_eq!(request_id, "abcd");
        assert_eq!(reason, "ok");
        assert!(matches!(
            p(&["deny", "--request-id", "ef01"]).unwrap().sub,
            ApprovalsSub::Deny { .. }
        ));
    }

    #[test]
    fn rejects_missing_id_and_unknown() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["grant"]).is_err(), "grant needs a request id");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = p(&["list", "--endpoint", "http://h:1", "--json"]).unwrap();
        assert_eq!(a.common.endpoint, "http://h:1");
        assert!(a.common.json);
    }
}
