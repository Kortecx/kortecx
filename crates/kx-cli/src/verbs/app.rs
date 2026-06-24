//! `kx app new | save | list | get | run | export` — author, persist, browse, and
//! RUN a durable App (a `kortecx.app/v1` envelope: a portable blueprint wrapped
//! with by-reference context/tool/connection/dataset references, a minimal
//! prompt/rule/skill/memory rail, a 4-axis steering config, and per-step replay
//! intent). Tri-surface parity with the SDK + UI.
//!
//! The catalog lives in an off-journal `apps.db` sidecar; the server derives
//! `app_ref` (SN-8) and scopes every App to the authoring party. The envelope
//! carries NO authority — `kx app run` re-compiles the blueprint through the same
//! `to_request` path as `kx blueprint run`, and the server re-resolves EVERY
//! warrant from the caller's own grants (SN-8 / BLOCKER #5). There is NO
//! cross-instance import entrypoint (a sharing feature, deferred post-POC).
//!
//! - `new <name> --from-blueprint <file>` authors an envelope locally (offline; no
//!   gateway) — steering + tags + an optional `--branch` handle — and writes it.
//! - `save <file>` validates + canonicalizes the envelope and `SaveApp`s it.
//! - `list` / `get <handle>` browse the catalog; `export <handle> --output` writes
//!   the pretty envelope back out (the round-trip artifact).
//! - `run <handle>` compiles the envelope's blueprint and submits it (exactly-once).

use std::path::PathBuf;
use std::time::Duration;

use kx_app::AppEnvelope;
use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::verbs::blueprint::{to_request, DagSpec};
use crate::{format, verbs, wait};

const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The `app` subcommand.
#[derive(Debug)]
pub enum AppSub {
    /// Author an envelope locally (offline) from a blueprint file + steering.
    New(NewSpec),
    /// Read an envelope JSON file and `SaveApp` it (handle defaults from its name).
    Save {
        /// The envelope JSON file.
        file: PathBuf,
        /// Optional catalog handle (`namespace/collection/name`); default derived.
        handle: Option<String>,
    },
    /// List the caller's App catalog.
    List,
    /// Show one App's summary, or `--output <file>` to write the pretty envelope.
    Get {
        /// The catalog handle.
        handle: String,
        /// Write the pretty envelope JSON here.
        output: Option<PathBuf>,
    },
    /// Compile the App's blueprint and run it (exactly-once; the server warrants).
    Run {
        /// The catalog handle.
        handle: String,
        /// Block for a terminal result.
        wait: bool,
        /// Wait timeout.
        timeout_secs: u64,
        /// Write the terminal result body here.
        out: Option<PathBuf>,
    },
    /// Write one App's pretty envelope JSON to `--output` (the round-trip artifact).
    Export {
        /// The catalog handle.
        handle: String,
        /// The destination file.
        output: PathBuf,
    },
}

/// A `app new` request, assembled from the flags (offline authoring).
#[derive(Debug)]
pub struct NewSpec {
    /// The App name.
    pub name: String,
    /// The blueprint JSON file (a `DagSpec` — from `kx chain run --emit-blueprint`
    /// or the SDK `.to_blueprint()` / `.export()`).
    pub from_blueprint: PathBuf,
    /// Optional model route to record in the steering config.
    pub model: Option<String>,
    /// Optional react turn budget.
    pub max_turns: Option<u32>,
    /// Optional react tool-call budget.
    pub max_tool_calls: Option<u32>,
    /// Catalog tags.
    pub tags: Vec<String>,
    /// Advisory description.
    pub description: Option<String>,
    /// Optional per-App project branch handle (reserved; never created here).
    pub branch: Option<String>,
    /// Write the pretty envelope JSON here (else stdout).
    pub output: Option<PathBuf>,
}

/// Parsed `app` arguments.
#[derive(Debug)]
pub struct AppArgs {
    /// The subcommand.
    pub sub: AppSub,
    /// Common client flags.
    pub common: ClientCommon,
}

fn parse_u32(raw: &str, flag: &str) -> Result<u32, CliError> {
    raw.parse::<u32>()
        .map_err(|_| CliError::Usage(format!("{flag} expects an integer, got {raw:?}")))
}

/// Parse `kx app <sub> ...`.
///
/// # Errors
/// [`CliError::Usage`] on an unknown subcommand / flag or a missing required argument.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<AppArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("app needs a subcommand (new|save|list|get|run|export)".into())
    })?;
    let mut common = ClientCommon::default();
    let mut positional: Option<String> = None;
    let mut from_blueprint: Option<PathBuf> = None;
    let mut handle: Option<String> = None;
    let mut output: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut model: Option<String> = None;
    let mut max_turns: Option<u32> = None;
    let mut max_tool_calls: Option<u32> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut description: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut wait_flag = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--from-blueprint" => {
                from_blueprint = Some(PathBuf::from(next_value(&mut args, "--from-blueprint")?));
            }
            "--handle" => handle = Some(next_value(&mut args, "--handle")?),
            "--output" => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            "--model" => model = Some(next_value(&mut args, "--model")?),
            "--max-turns" => {
                max_turns = Some(parse_u32(
                    &next_value(&mut args, "--max-turns")?,
                    "--max-turns",
                )?);
            }
            "--max-tool-calls" => {
                max_tool_calls = Some(parse_u32(
                    &next_value(&mut args, "--max-tool-calls")?,
                    "--max-tool-calls",
                )?);
            }
            "--tag" => tags.push(next_value(&mut args, "--tag")?),
            "--description" => description = Some(next_value(&mut args, "--description")?),
            "--branch" => branch = Some(next_value(&mut args, "--branch")?),
            "--wait" => wait_flag = true,
            "--timeout-secs" => {
                timeout_secs = u64::from(parse_u32(
                    &next_value(&mut args, "--timeout-secs")?,
                    "--timeout-secs",
                )?);
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            other if positional.is_none() => positional = Some(other.to_string()),
            other => return Err(CliError::Usage(format!("unexpected argument {other:?}"))),
        }
    }

    let sub = assemble_sub(
        &kw,
        Flags {
            positional,
            from_blueprint,
            handle,
            output,
            out,
            model,
            max_turns,
            max_tool_calls,
            tags,
            description,
            branch,
            wait_flag,
            timeout_secs,
        },
    )?;
    Ok(AppArgs { sub, common })
}

/// The accumulated `app` flags, dispatched to a subcommand by [`assemble_sub`].
struct Flags {
    positional: Option<String>,
    from_blueprint: Option<PathBuf>,
    handle: Option<String>,
    output: Option<PathBuf>,
    out: Option<PathBuf>,
    model: Option<String>,
    max_turns: Option<u32>,
    max_tool_calls: Option<u32>,
    tags: Vec<String>,
    description: Option<String>,
    branch: Option<String>,
    wait_flag: bool,
    timeout_secs: u64,
}

/// Validate the accumulated flags against the verb and build the subcommand.
fn assemble_sub(kw: &str, f: Flags) -> Result<AppSub, CliError> {
    let require_pos = |p: Option<String>, what: &str| -> Result<String, CliError> {
        p.filter(|s| !s.is_empty())
            .ok_or_else(|| CliError::Usage(format!("app {kw} requires {what}")))
    };
    match kw {
        "new" => Ok(AppSub::New(NewSpec {
            name: require_pos(f.positional, "a <name>")?,
            from_blueprint: f.from_blueprint.ok_or_else(|| {
                CliError::Usage("app new requires --from-blueprint <file>".into())
            })?,
            model: f.model,
            max_turns: f.max_turns,
            max_tool_calls: f.max_tool_calls,
            tags: f.tags,
            description: f.description,
            branch: f.branch,
            output: f.output,
        })),
        "save" => Ok(AppSub::Save {
            file: PathBuf::from(require_pos(f.positional, "a <file> (the envelope JSON)")?),
            handle: f.handle,
        }),
        "list" => Ok(AppSub::List),
        "get" => Ok(AppSub::Get {
            handle: require_pos(f.positional, "a <handle>")?,
            output: f.output,
        }),
        "run" => Ok(AppSub::Run {
            handle: require_pos(f.positional, "a <handle>")?,
            wait: f.wait_flag,
            timeout_secs: f.timeout_secs,
            out: f.out,
        }),
        "export" => Ok(AppSub::Export {
            handle: require_pos(f.positional, "a <handle>")?,
            output: f
                .output
                .ok_or_else(|| CliError::Usage("app export requires --output <file>".into()))?,
        }),
        other => Err(CliError::Usage(format!(
            "unknown app subcommand {other:?} (expected new | save | list | get | run | export)"
        ))),
    }
}

/// Sanitise an App name into a 3-segment default catalog handle `apps/local/<name>`.
fn default_handle(name: &str) -> String {
    let mut san: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    san = san.trim_matches(['.', '-']).to_string();
    san.truncate(128);
    if san.is_empty() {
        san = "app".to_string();
    }
    format!("apps/local/{san}")
}

/// Author an envelope offline from `--from-blueprint` + steering, write it out.
fn execute_new(spec: NewSpec) -> Result<(), CliError> {
    let raw = std::fs::read(&spec.from_blueprint).map_err(|e| {
        CliError::Usage(format!(
            "cannot read blueprint {}: {e}",
            spec.from_blueprint.display()
        ))
    })?;
    let blueprint: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| CliError::Usage(format!("invalid blueprint JSON: {e}")))?;
    // Validate the blueprint compiles (kinds / edges / tool args / reserved `exec`)
    // BEFORE wrapping it — fail at authoring, not at a later `run`.
    let dag: DagSpec = serde_json::from_value(blueprint.clone())
        .map_err(|e| CliError::Usage(format!("blueprint is not a valid DagSpec: {e}")))?;
    let _ = to_request(dag)?;

    let mut env = AppEnvelope::new(spec.name, blueprint);
    if let Some(d) = spec.description {
        env.description = d;
    }
    env.tags = spec.tags;
    if let Some(m) = spec.model {
        env.steering_config.model.model_route = m;
    }
    env.steering_config.guards.max_turns = spec.max_turns;
    env.steering_config.guards.max_tool_calls = spec.max_tool_calls;
    if let Some(b) = spec.branch {
        env.branch_handle = b;
    }
    env.validate()
        .map_err(|e| CliError::Usage(format!("authored envelope is invalid: {e}")))?;
    let pretty = env
        .to_pretty_json()
        .map_err(|e| CliError::Usage(format!("serialize envelope: {e}")))?;
    if let Some(path) = spec.output {
        std::fs::write(&path, pretty.as_bytes())
            .map_err(|e| CliError::Usage(format!("write {}: {e}", path.display())))?;
        println!("wrote {}", path.display());
    } else {
        print!("{pretty}");
    }
    Ok(())
}

/// Execute `app new | save | list | get | run | export`.
///
/// # Errors
/// [`CliError`] on a transport / status / usage failure.
pub async fn execute(args: AppArgs) -> Result<(), CliError> {
    // `new` is offline — no gateway contact.
    if let AppSub::New(spec) = args.sub {
        return execute_new(spec);
    }
    let json = args.common.json;
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    match args.sub {
        AppSub::New(_) => unreachable!("handled above"),
        AppSub::Save { file, handle } => {
            let raw = std::fs::read(&file)
                .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", file.display())))?;
            // Validate + canonicalize client-side (the same canonical bytes the
            // server re-derives); a bad envelope fails here with a clear message.
            let env = AppEnvelope::from_json_slice(&raw)
                .map_err(|e| CliError::Usage(format!("invalid app envelope: {e}")))?;
            let handle = handle.unwrap_or_else(|| default_handle(&env.name));
            let canonical = env
                .to_canonical_json()
                .map_err(|e| CliError::Usage(format!("serialize envelope: {e}")))?;
            let resp = client
                .save_app(resolved.request(proto::SaveAppRequest {
                    handle,
                    envelope_json: canonical,
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_save_app(&resp, json));
            Ok(())
        }
        AppSub::List => {
            let resp = client
                .list_apps(resolved.request(proto::ListAppsRequest {
                    limit: 0,
                    after_handle: String::new(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_apps_list(&resp, json));
            Ok(())
        }
        AppSub::Get { handle, output } => {
            let resp = fetch_app(&mut client, &resolved, &handle).await?;
            if let Some(path) = output {
                write_pretty_envelope(&resp.envelope_json, &path)?;
            }
            println!("{}", format::render_get_app(&resp, json));
            Ok(())
        }
        AppSub::Export { handle, output } => {
            let resp = fetch_app(&mut client, &resolved, &handle).await?;
            if !resp.found {
                return Err(CliError::Usage(format!("app {handle:?} not found")));
            }
            write_pretty_envelope(&resp.envelope_json, &output)?;
            println!("wrote {}", output.display());
            Ok(())
        }
        AppSub::Run {
            handle,
            wait: do_wait,
            timeout_secs,
            out,
        } => {
            let resp = fetch_app(&mut client, &resolved, &handle).await?;
            if !resp.found {
                return Err(CliError::Usage(format!("app {handle:?} not found")));
            }
            let env = AppEnvelope::from_json_slice(&resp.envelope_json)
                .map_err(|e| CliError::Usage(format!("stored envelope is invalid: {e}")))?;
            // The blueprint is authoritative; compile it through the ONE canonical
            // path. The server re-resolves every warrant from the caller's grants.
            let dag: DagSpec = serde_json::from_value(env.blueprint)
                .map_err(|e| CliError::Usage(format!("app blueprint is not a DagSpec: {e}")))?;
            let req = to_request(dag)?;
            let submitted = client
                .submit_workflow(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if do_wait {
                let outcome = wait::await_any_result(
                    &mut client,
                    &resolved,
                    submitted.instance_id,
                    Duration::from_secs(timeout_secs),
                )
                .await?;
                verbs::finish_wait(&outcome, json, out.as_deref())
            } else {
                println!("{}", format::render_submit(&submitted, json));
                Ok(())
            }
        }
    }
}

/// `GetApp` for `(handle)` — uniform not-found (no oracle).
async fn fetch_app(
    client: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
    handle: &str,
) -> Result<proto::GetAppResponse, CliError> {
    client
        .get_app(resolved.request(proto::GetAppRequest {
            handle: handle.to_string(),
        })?)
        .await
        .map_err(CliError::from_status)
        .map(tonic::Response::into_inner)
}

/// Write the stored (canonical) envelope bytes back out in the human PRETTY form.
fn write_pretty_envelope(envelope_json: &[u8], path: &std::path::Path) -> Result<(), CliError> {
    let env = AppEnvelope::from_json_slice(envelope_json)
        .map_err(|e| CliError::Usage(format!("stored envelope is invalid: {e}")))?;
    let pretty = env
        .to_pretty_json()
        .map_err(|e| CliError::Usage(format!("serialize envelope: {e}")))?;
    std::fs::write(path, pretty.as_bytes())
        .map_err(|e| CliError::Usage(format!("write {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> AppArgs {
        parse(args.iter().map(ToString::to_string)).unwrap()
    }

    #[test]
    fn parse_list() {
        assert!(matches!(parse_ok(&["list"]).sub, AppSub::List));
    }

    #[test]
    fn parse_save_with_default_handle() {
        let a = parse_ok(&["save", "echo.app.json"]);
        assert!(matches!(a.sub, AppSub::Save { handle: None, .. }));
    }

    #[test]
    fn parse_run_with_wait() {
        match parse_ok(&["run", "apps/local/echo", "--wait", "--timeout-secs", "30"]).sub {
            AppSub::Run {
                handle,
                wait,
                timeout_secs,
                ..
            } => {
                assert_eq!(handle, "apps/local/echo");
                assert!(wait);
                assert_eq!(timeout_secs, 30);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_new_requires_blueprint() {
        let err = parse(["new", "my-app"].iter().map(ToString::to_string)).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn parse_export_requires_output() {
        let err = parse(["export", "apps/local/x"].iter().map(ToString::to_string)).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn unknown_subcommand_is_usage_error() {
        let err = parse(["frobnicate"].iter().map(ToString::to_string)).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn default_handle_sanitizes() {
        assert_eq!(default_handle("My Echo App!"), "apps/local/my-echo-app");
        assert_eq!(default_handle("..."), "apps/local/app");
    }
}
