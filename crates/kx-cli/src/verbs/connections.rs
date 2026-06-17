//! `kx connections add | list | test | remove | discover` — govern the EXTERNAL
//! MCP gateway (PR-6b-1) over the gateway RPCs (`RegisterMcpServer` /
//! `ListMcpServers` / `TestMcpServer` / `DeregisterMcpServer` /
//! `DiscoverServerTools`). Tri-surface parity with the UI + SDK.
//!
//! The runtime is a SECURE GATEWAY (D132/D159/GR19): registering a server DIALS
//! it (the live untrusted-egress surface — admission + dial-time SSRF vetting +
//! per-server rate-limit). SN-8: the server derives the connection/tool ids; the
//! CLI never sends a warrant, and a credential is referenced by NAME only (D81).

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `connections` subcommand.
#[derive(Debug)]
pub enum ConnectionsSub {
    /// Register an external MCP server (DIALS it; discovers + registers its tools).
    Add(AddSpec),
    /// List the registered external MCP servers + their health.
    List,
    /// Test a server's reachability (dial + `initialize` only).
    Test {
        /// The server name.
        name: String,
    },
    /// Remove a server + deregister the tools it contributed.
    Remove {
        /// The server name.
        name: String,
    },
    /// Re-dial a server and re-discover its tools (lists the registered tools).
    Discover {
        /// The server name.
        name: String,
    },
}

/// A `connections add` request, assembled from the flags.
#[derive(Debug)]
pub struct AddSpec {
    /// The unique operator handle (namespaces the discovered tool ids).
    pub name: String,
    /// `stdio` | `http`.
    pub transport: String,
    /// stdio: the program path; http: the endpoint URL.
    pub endpoint: String,
    /// stdio command-line args (repeatable `--arg`).
    pub args: Vec<String>,
    /// http: refuse plaintext `http://` (`--tls-required`).
    pub tls_required: bool,
    /// OPTIONAL secret-less credential ref NAME (env var / vault key).
    pub credential_ref: String,
    /// PR-6b-3 firing posture: `"stateful"` | `"stateless"` (default stateless).
    pub session_mode: String,
}

/// Parsed `connections` arguments.
#[derive(Debug)]
pub struct ConnectionsArgs {
    /// The subcommand.
    pub sub: ConnectionsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Resolve the firing posture (PR-6b-3): `--stateful` wins; else a validated
/// `--session-mode`; else the stateless-first default.
fn resolve_session_mode(stateful_flag: bool, mode: Option<&str>) -> Result<String, CliError> {
    if stateful_flag {
        return Ok("stateful".to_string());
    }
    match mode {
        None | Some("stateless") => Ok("stateless".to_string()),
        Some("stateful") => Ok("stateful".to_string()),
        Some(other) => Err(CliError::Usage(format!(
            "--session-mode must be stateful | stateless, got {other:?}"
        ))),
    }
}

/// Parse `connections` args (the verb already consumed). The first token selects
/// the subcommand (`add` / `list` / `test` / `remove` / `discover`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ConnectionsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage(
            "connections requires a subcommand: add | list | test | remove | discover".into(),
        )
    })?;

    let mut name: Option<String> = None;
    // `--command` (stdio program) and `--url` (http endpoint) are kept distinct
    // from the client's own `--endpoint` (the GATEWAY address, consumed by
    // ClientCommon) so they never collide. The transport is inferred from which
    // is given (or pinned by the optional `--transport`).
    let mut command: Option<String> = None;
    let mut url: Option<String> = None;
    let mut transport: Option<String> = None;
    let mut server_args: Vec<String> = Vec::new();
    let mut tls_required = false;
    let mut credential_ref = String::new();
    // PR-6b-3: the firing posture. `--stateful` is sugar for
    // `--session-mode stateful`; default (neither) is stateless.
    let mut session_mode: Option<String> = None;
    let mut stateful_flag = false;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--name" => name = Some(next_value(&mut args, "--name")?),
            "--transport" => transport = Some(next_value(&mut args, "--transport")?),
            "--command" => command = Some(next_value(&mut args, "--command")?),
            "--url" => url = Some(next_value(&mut args, "--url")?),
            "--arg" => server_args.push(next_value(&mut args, "--arg")?),
            "--tls-required" => tls_required = true,
            "--credential-ref" => credential_ref = next_value(&mut args, "--credential-ref")?,
            "--session-mode" => session_mode = Some(next_value(&mut args, "--session-mode")?),
            "--stateful" => stateful_flag = true,
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let require_name = |name: Option<String>, verb: &str| -> Result<String, CliError> {
        name.filter(|s| !s.is_empty())
            .ok_or_else(|| CliError::Usage(format!("connections {verb} requires --name <server>")))
    };

    let sub = match kw.as_str() {
        "list" => ConnectionsSub::List,
        "test" => ConnectionsSub::Test {
            name: require_name(name, "test")?,
        },
        "remove" => ConnectionsSub::Remove {
            name: require_name(name, "remove")?,
        },
        "discover" => ConnectionsSub::Discover {
            name: require_name(name, "discover")?,
        },
        "add" => {
            let name = require_name(name, "add")?;
            // Infer the transport from the endpoint flag given, unless pinned.
            let transport = match transport {
                Some(t) if t == "stdio" || t == "http" => t,
                Some(t) => {
                    return Err(CliError::Usage(format!(
                        "--transport must be stdio | http, got {t:?}"
                    )))
                }
                None => match (&command, &url) {
                    (Some(_), None) => "stdio".to_string(),
                    (None, Some(_)) => "http".to_string(),
                    _ => {
                        return Err(CliError::Usage(
                            "connections add requires exactly one of --command <path> (stdio) or --url <url> (http)"
                                .into(),
                        ))
                    }
                },
            };
            let endpoint = match transport.as_str() {
                "stdio" => command.filter(|s| !s.is_empty()).ok_or_else(|| {
                    CliError::Usage("connections add --transport stdio requires --command <path>".into())
                })?,
                _ => url.filter(|s| !s.is_empty()).ok_or_else(|| {
                    CliError::Usage("connections add --transport http requires --url <url>".into())
                })?,
            };
            let session_mode = resolve_session_mode(stateful_flag, session_mode.as_deref())?;
            ConnectionsSub::Add(AddSpec {
                name,
                transport,
                endpoint,
                args: server_args,
                tls_required,
                credential_ref,
                session_mode,
            })
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown connections subcommand {other:?} (expected add | list | test | remove | discover)"
            )))
        }
    };
    Ok(ConnectionsArgs { sub, common })
}

/// Execute `connections`.
pub async fn execute(args: ConnectionsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        ConnectionsSub::Add(spec) => {
            let req = proto::RegisterMcpServerRequest {
                server_name: spec.name,
                transport: spec.transport,
                endpoint: spec.endpoint,
                args: spec.args,
                tls_required: spec.tls_required,
                credential_ref: spec.credential_ref,
                session_mode: spec.session_mode,
            };
            let resp = client
                .register_mcp_server(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_register_server(&resp, json));
        }
        ConnectionsSub::List => {
            let req = proto::ListMcpServersRequest {
                limit: 0,
                after_name: String::new(),
            };
            let resp = client
                .list_mcp_servers(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_connections_list(&resp, json));
        }
        ConnectionsSub::Test { name } => {
            let req = proto::TestMcpServerRequest { server_name: name };
            let resp = client
                .test_mcp_server(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_test_server(&resp, json));
        }
        ConnectionsSub::Remove { name } => {
            let req = proto::DeregisterMcpServerRequest { server_name: name };
            let resp = client
                .deregister_mcp_server(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_deregister_server(&resp, json));
        }
        ConnectionsSub::Discover { name } => {
            let req = proto::DiscoverServerToolsRequest { server_name: name };
            let resp = client
                .discover_server_tools(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_discover_server(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ConnectionsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_add_http_with_credential() {
        let a = p(&[
            "add",
            "--name",
            "github",
            "--url",
            "https://mcp.github.example/rpc",
            "--tls-required",
            "--credential-ref",
            "GH_MCP_TOKEN",
        ])
        .unwrap();
        let ConnectionsSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.name, "github");
        assert_eq!(spec.transport, "http"); // inferred from --url
        assert_eq!(spec.endpoint, "https://mcp.github.example/rpc");
        assert!(spec.tls_required);
        assert_eq!(spec.credential_ref, "GH_MCP_TOKEN");
        assert_eq!(spec.session_mode, "stateless", "default is stateless-first");
    }

    #[test]
    fn parses_session_mode_flags() {
        // `--stateful` sugar.
        let a = p(&["add", "--name", "s", "--command", "x", "--stateful"]).unwrap();
        let ConnectionsSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.session_mode, "stateful");
        // explicit `--session-mode stateless`.
        let a = p(&[
            "add",
            "--name",
            "s",
            "--command",
            "x",
            "--session-mode",
            "stateless",
        ])
        .unwrap();
        let ConnectionsSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.session_mode, "stateless");
        // a bad value is rejected.
        assert!(
            p(&[
                "add",
                "--name",
                "s",
                "--command",
                "x",
                "--session-mode",
                "weird"
            ])
            .is_err(),
            "bad session-mode"
        );
    }

    #[test]
    fn parses_add_stdio_with_args_inferred_transport() {
        let a = p(&[
            "add",
            "--name",
            "local",
            "--command",
            "my-server",
            "--arg",
            "--stdio",
            "--arg",
            "-v",
        ])
        .unwrap();
        let ConnectionsSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.transport, "stdio"); // inferred from --command
        assert_eq!(spec.endpoint, "my-server");
        assert_eq!(spec.args, vec!["--stdio".to_string(), "-v".to_string()]);
        assert!(!spec.tls_required);
    }

    #[test]
    fn parses_list_test_remove_discover() {
        assert!(matches!(p(&["list"]).unwrap().sub, ConnectionsSub::List));
        assert!(matches!(
            p(&["test", "--name", "x"]).unwrap().sub,
            ConnectionsSub::Test { .. }
        ));
        assert!(matches!(
            p(&["remove", "--name", "x"]).unwrap().sub,
            ConnectionsSub::Remove { .. }
        ));
        assert!(matches!(
            p(&["discover", "--name", "x"]).unwrap().sub,
            ConnectionsSub::Discover { .. }
        ));
    }

    #[test]
    fn rejects_missing_required_and_bad_transport() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(
            p(&["add", "--name", "x"]).is_err(),
            "add needs --command/--url"
        );
        assert!(p(&["add", "--command", "x"]).is_err(), "add needs --name");
        assert!(p(&["test"]).is_err(), "test needs --name");
        assert!(
            p(&["add", "--name", "x", "--command", "y", "--transport", "ftp"]).is_err(),
            "bad transport"
        );
        assert!(
            p(&["add", "--name", "x", "--command", "y", "--url", "z"]).is_err(),
            "exactly one endpoint"
        );
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
    }
}
