// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx scripts register | list | get | deregister` — the script registry's
//! operator surface, at parity with the console and the SDKs.
//!
//! ## Registration grants nothing
//!
//! The `--mount` and `--net-host` flags declare what the script SAYS it needs.
//! That declaration becomes the tool's requirement, and the broker still refuses
//! any dispatch whose requirement is not a subset of the granting warrant. The
//! client never supplies a warrant and never names an id: `script_id` is
//! server-derived. Declaring `rw:/` here does not grant `rw:/` — it guarantees
//! the script is refused everywhere that authority is absent.
//!
//! ## `--argv` and `--env` are fixed at registration, deliberately
//!
//! A model calling a script controls exactly one thing: the `input` string on
//! stdin. argv and the environment are frozen here, and the child's environment
//! is CLEARED before the fixed pairs are set. That asymmetry is the point — argv
//! and env are where a shell script is easiest to subvert.
//!
//! ## The interpreter is probed, not assumed
//!
//! Registration refuses if the host cannot run the declared interpreter, or if no
//! sandbox shim shipped with this serve. A script that cannot be sandboxed does
//! not register; there is no configuration in which one runs unconfined.

use std::path::PathBuf;

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::hex;

/// The `scripts` subcommand.
#[derive(Debug)]
pub enum ScriptsSub {
    /// Register a script from a source file.
    Register(Box<RegisterArgs>),
    /// List registered scripts.
    List {
        /// Page size; 0 = server default (100, clamped to 256).
        limit: u32,
        /// Exclusive cursor: last row's name.
        after_name: String,
        /// Cursor tiebreak.
        after_version: String,
    },
    /// Fetch one script, including its registered source bytes.
    Get {
        /// Script name.
        name: String,
        /// Script version.
        version: String,
        /// Write the source here instead of stdout.
        output: Option<PathBuf>,
    },
    /// Deregister an exact `(name, version)`.
    Deregister {
        /// Script name.
        name: String,
        /// Script version.
        version: String,
    },
}

/// `scripts register` arguments.
#[derive(Debug, Default)]
pub struct RegisterArgs {
    /// Identity half.
    pub name: String,
    /// Identity half.
    pub version: String,
    /// Advisory; never parsed for enforcement.
    pub description: String,
    /// Validated against the host's closed allowlist.
    pub interpreter: String,
    /// The source file.
    pub source: PathBuf,
    /// Fixed args; never model-controlled.
    pub argv: Vec<String>,
    /// Fixed environment; never model-controlled.
    pub env: Vec<(String, String)>,
    /// Declared filesystem needs, as `mode:path`.
    pub mounts: Vec<(String, String)>,
    /// Declared egress hosts; empty = no egress.
    pub net_hosts: Vec<String>,
    /// 0 = host default.
    pub wall_clock_ms: u64,
    /// 0 = unset.
    pub mem_bytes: u64,
    /// 0 = host default; exceeding REFUSES rather than truncating.
    pub max_output_bytes: u64,
}

/// Parsed `scripts` arguments.
#[derive(Debug)]
pub struct ScriptsArgs {
    /// The subcommand.
    pub sub: ScriptsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

fn split_once_or(s: &str, sep: char, what: &str) -> Result<(String, String), CliError> {
    s.split_once(sep)
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .ok_or_else(|| CliError::Usage(format!("{what} expects `{}`", what.replace("--", ""))))
}

/// Parse `scripts` args (the verb already consumed).
#[allow(clippy::too_many_lines)]
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ScriptsArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("scripts requires a subcommand: register | list | get | deregister".into())
    })?;

    let mut common = ClientCommon::default();
    let mut positional: Vec<String> = Vec::new();
    let mut r = RegisterArgs::default();
    let mut output: Option<PathBuf> = None;
    let mut limit: u32 = 0;
    let mut after_name = String::new();
    let mut after_version = String::new();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--name" => r.name = next_value(&mut args, "--name")?,
            "--version" => r.version = next_value(&mut args, "--version")?,
            "--description" => r.description = next_value(&mut args, "--description")?,
            "--interpreter" => r.interpreter = next_value(&mut args, "--interpreter")?,
            "--source" => r.source = PathBuf::from(next_value(&mut args, "--source")?),
            "--argv" => r.argv.push(next_value(&mut args, "--argv")?),
            "--env" => {
                let v = next_value(&mut args, "--env")?;
                r.env.push(split_once_or(&v, '=', "--env KEY=VALUE")?);
            }
            "--mount" => {
                let v = next_value(&mut args, "--mount")?;
                // `mode:path`, e.g. `ro:/srv/data`.
                let (mode, path) = split_once_or(&v, ':', "--mount MODE:PATH")?;
                r.mounts.push((mode, path));
            }
            "--net-host" => r.net_hosts.push(next_value(&mut args, "--net-host")?),
            "--wall-clock-ms" => {
                r.wall_clock_ms = next_value(&mut args, "--wall-clock-ms")?
                    .parse()
                    .map_err(|_| CliError::Usage("--wall-clock-ms expects a number".into()))?;
            }
            "--mem-bytes" => {
                r.mem_bytes = next_value(&mut args, "--mem-bytes")?
                    .parse()
                    .map_err(|_| CliError::Usage("--mem-bytes expects a number".into()))?;
            }
            "--max-output-bytes" => {
                r.max_output_bytes = next_value(&mut args, "--max-output-bytes")?
                    .parse()
                    .map_err(|_| CliError::Usage("--max-output-bytes expects a number".into()))?;
            }
            "--output" => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--after-name" => after_name = next_value(&mut args, "--after-name")?,
            "--after-version" => after_version = next_value(&mut args, "--after-version")?,
            "--limit" => {
                limit = next_value(&mut args, "--limit")?
                    .parse()
                    .map_err(|_| CliError::Usage("--limit expects a number".into()))?;
            }
            other if !other.starts_with("--") => positional.push(other.to_string()),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let name_version = |p: &[String], verb: &str| -> Result<(String, String), CliError> {
        match (p.first(), p.get(1)) {
            (Some(n), Some(v)) if !n.is_empty() && !v.is_empty() => Ok((n.clone(), v.clone())),
            _ => Err(CliError::Usage(format!(
                "scripts {verb} requires <NAME> <VERSION>"
            ))),
        }
    };

    let sub = match kw.as_str() {
        "register" => {
            if r.name.is_empty() {
                r.name = positional.first().cloned().unwrap_or_default();
            }
            if r.version.is_empty() {
                r.version = positional.get(1).cloned().unwrap_or_default();
            }
            for (what, v) in [
                ("--name", &r.name),
                ("--version", &r.version),
                ("--interpreter", &r.interpreter),
            ] {
                if v.is_empty() {
                    return Err(CliError::Usage(format!("scripts register requires {what}")));
                }
            }
            if r.source.as_os_str().is_empty() {
                return Err(CliError::Usage(
                    "scripts register requires --source <FILE>".into(),
                ));
            }
            ScriptsSub::Register(Box::new(r))
        }
        "list" => ScriptsSub::List {
            limit,
            after_name,
            after_version,
        },
        "get" => {
            let (name, version) = name_version(&positional, "get")?;
            ScriptsSub::Get {
                name,
                version,
                output,
            }
        }
        "deregister" => {
            let (name, version) = name_version(&positional, "deregister")?;
            ScriptsSub::Deregister { name, version }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown scripts subcommand {other:?} \
                 (expected register | list | get | deregister)"
            )))
        }
    };
    Ok(ScriptsArgs { sub, common })
}

/// Execute `scripts`.
#[allow(clippy::too_many_lines)]
pub async fn execute(args: ScriptsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        ScriptsSub::Register(r) => {
            let source = std::fs::read(&r.source)
                .map_err(|e| CliError::Io(format!("{}: {e}", r.source.display())))?;
            let req = proto::RegisterScriptRequest {
                script_name: r.name.clone(),
                script_version: r.version.clone(),
                description: r.description,
                interpreter: r.interpreter,
                source,
                argv: r.argv,
                env: r
                    .env
                    .into_iter()
                    .map(|(key, value)| proto::ScriptEnv { key, value })
                    .collect(),
                fs_mounts: r
                    .mounts
                    .into_iter()
                    .map(|(mode, path)| proto::ScriptMount { path, mode })
                    .collect(),
                net_hosts: r.net_hosts,
                wall_clock_ms: r.wall_clock_ms,
                mem_bytes: r.mem_bytes,
                max_output_bytes: r.max_output_bytes,
            };
            let resp = client
                .register_script(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "script_id": hex::encode(&resp.script_id),
                        "script_name": r.name,
                        "script_version": r.version,
                    })
                );
            } else {
                println!(
                    "registered {}@{}  id={}",
                    r.name,
                    r.version,
                    hex::encode(&resp.script_id)
                );
            }
            Ok(())
        }

        ScriptsSub::List {
            limit,
            after_name,
            after_version,
        } => {
            let req = proto::ListScriptsRequest {
                limit,
                after_name,
                after_version,
            };
            let resp = client
                .list_scripts(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                let rows: Vec<_> = resp.scripts.iter().map(render_script_json).collect();
                println!(
                    "{}",
                    serde_json::json!({ "scripts": rows, "has_more": resp.has_more })
                );
            } else if resp.scripts.is_empty() {
                println!("no scripts");
            } else {
                for s in &resp.scripts {
                    println!(
                        "{}@{}  {}  fs={}  net={}",
                        s.script_name,
                        s.script_version,
                        s.interpreter,
                        s.fs_scope_summary,
                        s.net_scope_summary
                    );
                }
                if resp.has_more {
                    println!("… more (use --after-name/--after-version)");
                }
            }
            Ok(())
        }

        ScriptsSub::Get {
            name,
            version,
            output,
        } => {
            let req = proto::GetScriptRequest {
                script_name: name.clone(),
                script_version: version.clone(),
            };
            let resp = client
                .get_script(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if !resp.found {
                return Err(CliError::Runtime(format!(
                    "script {name}@{version}: not found"
                )));
            }
            if let Some(path) = output {
                std::fs::write(&path, &resp.source)
                    .map_err(|e| CliError::Io(format!("--output {}: {e}", path.display())))?;
                println!("wrote {} ({} bytes)", path.display(), resp.source.len());
            } else if json {
                let mut obj = resp
                    .script
                    .as_ref()
                    .map(render_script_json)
                    .unwrap_or_default();
                if let Some(map) = obj.as_object_mut() {
                    map.insert(
                        "source".into(),
                        serde_json::Value::String(String::from_utf8_lossy(&resp.source).into()),
                    );
                }
                println!("{obj}");
            } else {
                if let Some(s) = &resp.script {
                    println!(
                        "{}@{}  {}  source_ref={}",
                        s.script_name, s.script_version, s.interpreter, s.source_ref_hex
                    );
                }
                println!("{}", String::from_utf8_lossy(&resp.source));
            }
            Ok(())
        }

        ScriptsSub::Deregister { name, version } => {
            let req = proto::DeregisterScriptRequest {
                script_name: name.clone(),
                script_version: version.clone(),
            };
            let resp = client
                .deregister_script(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "script_name": name, "script_version": version, "removed": resp.removed
                    })
                );
            } else if resp.removed {
                println!("deregistered {name}@{version}");
            } else {
                println!("{name}@{version}: nothing to deregister");
            }
            Ok(())
        }
    }
}

fn render_script_json(s: &proto::RegisteredScript) -> serde_json::Value {
    serde_json::json!({
        "script_id": hex::encode(&s.script_id),
        "script_name": s.script_name,
        "script_version": s.script_version,
        "interpreter": s.interpreter,
        "description": s.description,
        "source_ref": s.source_ref_hex,
        "fs_scope": s.fs_scope_summary,
        "net_scope": s.net_scope_summary,
        "wall_clock_ms": s.wall_clock_ms,
        "max_output_bytes": s.max_output_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse, ScriptsSub};

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn register_requires_the_identity_and_a_source() {
        assert!(parse(args(&["register", "--name", "a"])).is_err());
        assert!(parse(args(&["register", "--name", "a", "--version", "1"])).is_err());
        let ok = parse(args(&[
            "register",
            "--name",
            "tidy",
            "--version",
            "1",
            "--interpreter",
            "sh",
            "--source",
            "t.sh",
        ]));
        assert!(ok.is_ok(), "{:?}", ok.err());
    }

    #[test]
    fn mounts_and_env_parse_into_pairs() {
        let a = parse(args(&[
            "register",
            "--name",
            "t",
            "--version",
            "1",
            "--interpreter",
            "sh",
            "--source",
            "t.sh",
            "--mount",
            "ro:/srv/data",
            "--env",
            "TZ=UTC",
            "--argv",
            "--strict",
        ]))
        .unwrap();
        match a.sub {
            ScriptsSub::Register(r) => {
                assert_eq!(r.mounts, vec![("ro".into(), "/srv/data".into())]);
                assert_eq!(r.env, vec![("TZ".into(), "UTC".into())]);
                assert_eq!(r.argv, vec!["--strict".to_string()]);
            }
            other => panic!("expected Register, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_mount_is_a_usage_error() {
        let e = parse(args(&[
            "register",
            "--name",
            "t",
            "--version",
            "1",
            "--interpreter",
            "sh",
            "--source",
            "t.sh",
            "--mount",
            "/no/mode",
        ]));
        assert!(e.is_err());
    }

    #[test]
    fn get_and_deregister_take_name_and_version_positionally() {
        assert!(parse(args(&["get", "tidy"])).is_err());
        let a = parse(args(&["deregister", "tidy", "2"])).unwrap();
        match a.sub {
            ScriptsSub::Deregister { name, version } => {
                assert_eq!(name, "tidy");
                assert_eq!(version, "2");
            }
            other => panic!("expected Deregister, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_subcommand_names_the_alternatives() {
        let msg = format!("{}", parse(args(&["run", "x"])).unwrap_err());
        assert!(msg.contains("register"), "got {msg}");
    }
}
