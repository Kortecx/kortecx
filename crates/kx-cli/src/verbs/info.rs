//! `kx info` — POC-1 Settings "Workspace": the NON-SECRET server configuration
//! (resolved model, dirs, ports, feature flags, auth/CORS/TLS posture). Governed by
//! an AUTHENTICATED caller (the gateway refuses an unresolved party). Never prints a
//! secret — no bearer token, no TLS key (only an `auth_mode` label + a `tls` posture).

use kx_proto::proto;
use serde_json::json;

use crate::client::ClientCommon;
use crate::error::CliError;

/// Parsed `kx info` arguments — just the common client flags.
#[derive(Debug)]
pub struct InfoArgs {
    /// Common client flags (`--endpoint` / `--token` / `--tls-ca` / `--json`).
    pub common: ClientCommon,
}

/// Parse `info` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<InfoArgs, CliError> {
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("info: unknown flag {flag:?}")));
    }
    Ok(InfoArgs { common })
}

/// Execute `info`: fetch + render the non-secret server configuration.
pub async fn execute(args: InfoArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let info = client
        .get_server_info(resolved.request(proto::GetServerInfoRequest {})?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    if args.common.json {
        println!("{}", render_json(&info));
    } else {
        print_human(&info);
    }
    Ok(())
}

/// A non-empty value, or `dim`med "—" for an empty/absent string field.
fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn print_human(info: &proto::GetServerInfoResponse) {
    let model = if info.model_id.is_empty() {
        "(none — model-less serve)".to_string()
    } else if info.model_path.is_empty() {
        info.model_id.clone()
    } else {
        format!("{} ({})", info.model_id, info.model_path)
    };
    let cors = if info.cors_origins.is_empty() {
        "(none — deny-by-default)".to_string()
    } else {
        info.cors_origins.join(", ")
    };
    println!("kortecx server");
    println!("  model      {model}");
    if !info.embed_model_id.is_empty() {
        // PR-B: the configured datasets/RAG embed model (operator-config else primary).
        println!("  embed      {} (datasets/RAG)", info.embed_model_id);
    }
    println!(
        "  endpoints  grpc {} · ws {} · console {} · metrics {}",
        or_dash(&info.listen_addr),
        or_dash(&info.ws_addr),
        or_dash(&info.console_addr),
        or_dash(&info.metrics_addr),
    );
    println!(
        "  storage    content {} · journal {} · catalog {}",
        or_dash(&info.content_root),
        or_dash(&info.journal_path),
        or_dash(&info.catalog_dir),
    );
    println!(
        "  limits     max_lease {} · content_max_bytes {}",
        info.max_lease, info.content_max_bytes,
    );
    println!(
        "  security   auth {} · tls {} · cors {cors}",
        or_dash(&info.auth_mode),
        on_off(info.tls_enabled),
    );
    println!(
        "  features   inference {} · hnsw {} · console {} · vision {}",
        on_off(info.feature_inference),
        on_off(info.feature_hnsw),
        on_off(info.feature_console),
        on_off(info.feature_vision),
    );
    println!("  audit      {}", on_off(info.audit_log_enabled));
    println!(
        "  agentic    max_turns {} · max_tool_calls {} (default; per-run overridable)",
        info.react_max_turns, info.react_max_tool_calls,
    );
}

fn render_json(info: &proto::GetServerInfoResponse) -> String {
    json!({
        "model_id": info.model_id,
        "embed_model_id": info.embed_model_id,
        "model_path": info.model_path,
        "listen_addr": info.listen_addr,
        "ws_addr": info.ws_addr,
        "console_addr": info.console_addr,
        "metrics_addr": info.metrics_addr,
        "content_root": info.content_root,
        "journal_path": info.journal_path,
        "catalog_dir": info.catalog_dir,
        "max_lease": info.max_lease,
        "content_max_bytes": info.content_max_bytes,
        "cors_origins": info.cors_origins,
        "tls_enabled": info.tls_enabled,
        "auth_mode": info.auth_mode,
        "feature_hnsw": info.feature_hnsw,
        "feature_inference": info.feature_inference,
        "feature_console": info.feature_console,
        "feature_vision": info.feature_vision,
        "audit_log_enabled": info.audit_log_enabled,
        "react_max_turns": info.react_max_turns,
        "react_max_tool_calls": info.react_max_tool_calls,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_flags_only() {
        let a = parse(
            ["--json", "--endpoint", "http://h:1"]
                .iter()
                .map(|s| (*s).to_string()),
        )
        .unwrap();
        assert!(a.common.json);
        assert_eq!(a.common.endpoint, "http://h:1");
    }

    #[test]
    fn unknown_flag_is_usage() {
        assert!(parse(["--nope"].iter().map(|s| (*s).to_string())).is_err());
    }

    #[test]
    fn json_render_omits_no_secret_fields() {
        // The response type carries no secret field; the render is a pure projection.
        let info = proto::GetServerInfoResponse {
            model_id: "m".into(),
            auth_mode: "token".into(),
            tls_enabled: true,
            react_max_turns: 8,
            react_max_tool_calls: 20,
            ..Default::default()
        };
        let s = render_json(&info);
        assert!(s.contains("\"auth_mode\":\"token\""));
        assert!(
            !s.contains("token-value"),
            "no secret token value is ever rendered"
        );
        // T-MULTI-ELEMENT-TOOLCALLS: the agentic budget is projected read-only.
        assert!(s.contains("\"react_max_tool_calls\":20"));
    }
}
