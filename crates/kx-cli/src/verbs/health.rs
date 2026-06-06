//! `kx health` — the gateway liveness/readiness probe (A2).
//!
//! Calls the standard `grpc.health.v1.Health/Check` and exits 0 when the gateway
//! reports SERVING, non-zero otherwise — a purpose-built endpoint, unlike the
//! side-effectful `signatures list` the compose healthcheck used before. The
//! health service is NOT behind the auth interceptor (a health probe is
//! unauthenticated by design), so no token is needed; `--endpoint`/`--tls-ca` are
//! honored so it works against a TLS gateway too.

use serde_json::json;
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

use crate::client::ClientCommon;
use crate::error::CliError;

/// Parsed `kx health` arguments — just the common client flags.
#[derive(Debug)]
pub struct HealthArgs {
    /// Common client flags (`--endpoint` / `--tls-ca` / `--json`). A bearer token
    /// is accepted but not required (the health probe is unauthenticated).
    pub common: ClientCommon,
}

/// Parse `health` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<HealthArgs, CliError> {
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("health: unknown flag {flag:?}")));
    }
    Ok(HealthArgs { common })
}

/// Execute `health`: Check the overall ("") serving status; exit 0 iff SERVING.
pub async fn execute(args: HealthArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let channel = resolved.channel().await?;
    let mut client = HealthClient::new(channel);
    let status = client
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await
        .map_err(CliError::from_status)?
        .into_inner()
        .status();
    let serving = status == ServingStatus::Serving;
    if args.common.json {
        println!(
            "{}",
            json!({ "status": status.as_str_name(), "serving": serving })
        );
    } else {
        println!("{}", status.as_str_name());
    }
    if serving {
        Ok(())
    } else {
        Err(CliError::Failed)
    }
}
