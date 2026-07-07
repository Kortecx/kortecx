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

use kx_app::{AppEnvelope, SkillRef};
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
    /// Prefers the server-side `RunApp` (G2 — honors the envelope's
    /// `references.connections` + `guards.secret_scope`); falls back to the legacy
    /// client-orchestrated `GetApp` → `SubmitWorkflow` on an older server.
    Run {
        /// The catalog handle.
        handle: String,
        /// Block for a terminal result.
        wait: bool,
        /// Wait timeout.
        timeout_secs: u64,
        /// Write the terminal result body here.
        out: Option<PathBuf>,
        /// Optional `--arg k=v` entry inputs, folded server-side into the App's entry
        /// model step prompt (an "Inputs" block). Requires a RunApp-capable server.
        args: Vec<(String, String)>,
    },
    /// Write one App's pretty envelope JSON to `--output` (the round-trip artifact).
    Export {
        /// The catalog handle.
        handle: String,
        /// The destination file.
        output: PathBuf,
    },
    /// POC-5a: agentically scaffold an EXISTING App's fixed-skeleton project tree
    /// into its CoW branch (server-side; the host is never written). `--wait` polls
    /// the scaffold status until it completes.
    Scaffold {
        /// The catalog handle (its project branch = the same handle).
        handle: String,
        /// Optional authoring goal/intent (defaults to the App's name server-side).
        goal: Option<String>,
        /// Block + poll the scaffold status until Done/Failed.
        wait: bool,
        /// Wait timeout.
        timeout_secs: u64,
    },
    /// POC-5a: list the files in an App's project branch (the scaffolded tree).
    Files {
        /// The catalog handle.
        handle: String,
    },
    /// POC-5a: print one App project file's body (caller-scoped branch read).
    Cat {
        /// The catalog handle.
        handle: String,
        /// The file path within the App's project branch.
        path: String,
        /// Write the body here instead of stdout.
        out: Option<PathBuf>,
    },
    /// POC-5d: dump the App's blueprint structure (the agentic step DAG the lineage
    /// editor renders) — steps + edges. `--json` emits the raw blueprint JSON.
    Structure {
        /// The catalog handle.
        handle: String,
    },
    /// POC-5d: directly write a file in the App's project branch from a local file
    /// (`PutContent` → `AdvanceBranch`; the host is never read, only the `--from`
    /// file). A locked App refuses the write server-side (LOCKED_BRANCH).
    EditFile {
        /// The catalog handle (its project branch = the same handle).
        handle: String,
        /// The file path within the App's project branch.
        path: String,
        /// The local file whose bytes become the new body.
        from: PathBuf,
    },
    /// POC-5b: lock the App's project branch (agentic in-CAS edits are refused).
    Lock {
        /// The catalog handle.
        handle: String,
    },
    /// POC-5b: unlock the App's project branch (re-enable agentic edits).
    Unlock {
        /// The catalog handle.
        handle: String,
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
    /// Catalog skill names to attach (`--skill`, repeatable). Non-empty
    /// makes `new` CONDITIONALLY ONLINE (each name resolves via `GetSkillForm`
    /// to a `SkillRef` — instructions_ref is server-derived, never hand-typed).
    pub skills: Vec<String>,
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
#[allow(clippy::too_many_lines)] // a flat flag-parsing dispatcher (the verbs' convention)
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<AppArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage(
            "app needs a subcommand (new|save|list|get|run|export|scaffold|files|cat|\
             structure|edit|lock|unlock)"
                .into(),
        )
    })?;
    let mut common = ClientCommon::default();
    let mut positional: Option<String> = None;
    let mut positional2: Option<String> = None;
    let mut goal: Option<String> = None;
    let mut from_blueprint: Option<PathBuf> = None;
    let mut handle: Option<String> = None;
    let mut output: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut from: Option<PathBuf> = None;
    let mut model: Option<String> = None;
    let mut max_turns: Option<u32> = None;
    let mut max_tool_calls: Option<u32> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();
    let mut description: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut wait_flag = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut app_args: Vec<(String, String)> = Vec::new();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--arg" => {
                let kv = next_value(&mut args, "--arg")?;
                let (k, v) = kv
                    .split_once('=')
                    .ok_or_else(|| CliError::Usage(format!("--arg expects k=v, got {kv:?}")))?;
                app_args.push((k.to_string(), v.to_string()));
            }
            "--from-blueprint" => {
                from_blueprint = Some(PathBuf::from(next_value(&mut args, "--from-blueprint")?));
            }
            "--handle" => handle = Some(next_value(&mut args, "--handle")?),
            "--output" => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--out" => out = Some(PathBuf::from(next_value(&mut args, "--out")?)),
            "--from" => from = Some(PathBuf::from(next_value(&mut args, "--from")?)),
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
            "--skill" => skills.push(next_value(&mut args, "--skill")?),
            "--description" => description = Some(next_value(&mut args, "--description")?),
            "--branch" => branch = Some(next_value(&mut args, "--branch")?),
            "--goal" => goal = Some(next_value(&mut args, "--goal")?),
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
            other if positional2.is_none() => positional2 = Some(other.to_string()),
            other => return Err(CliError::Usage(format!("unexpected argument {other:?}"))),
        }
    }

    let sub = assemble_sub(
        &kw,
        Flags {
            positional,
            positional2,
            goal,
            from_blueprint,
            handle,
            output,
            out,
            from,
            model,
            max_turns,
            max_tool_calls,
            tags,
            description,
            branch,
            wait_flag,
            timeout_secs,
            app_args,
            skills,
        },
    )?;
    Ok(AppArgs { sub, common })
}

/// The accumulated `app` flags, dispatched to a subcommand by [`assemble_sub`].
struct Flags {
    positional: Option<String>,
    positional2: Option<String>,
    goal: Option<String>,
    from_blueprint: Option<PathBuf>,
    handle: Option<String>,
    output: Option<PathBuf>,
    out: Option<PathBuf>,
    from: Option<PathBuf>,
    model: Option<String>,
    max_turns: Option<u32>,
    max_tool_calls: Option<u32>,
    tags: Vec<String>,
    description: Option<String>,
    branch: Option<String>,
    wait_flag: bool,
    timeout_secs: u64,
    app_args: Vec<(String, String)>,
    skills: Vec<String>,
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
            skills: f.skills,
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
            args: f.app_args,
        }),
        "export" => Ok(AppSub::Export {
            handle: require_pos(f.positional, "a <handle>")?,
            output: f
                .output
                .ok_or_else(|| CliError::Usage("app export requires --output <file>".into()))?,
        }),
        "scaffold" => Ok(AppSub::Scaffold {
            handle: require_pos(f.positional, "a <handle>")?,
            goal: f.goal,
            wait: f.wait_flag,
            timeout_secs: f.timeout_secs,
        }),
        "files" => Ok(AppSub::Files {
            handle: require_pos(f.positional, "a <handle>")?,
        }),
        "cat" => Ok(AppSub::Cat {
            handle: require_pos(f.positional, "a <handle>")?,
            path: f
                .positional2
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("app cat requires a <path> argument".into()))?,
            out: f.out,
        }),
        "structure" => Ok(AppSub::Structure {
            handle: require_pos(f.positional, "a <handle>")?,
        }),
        "edit" => Ok(AppSub::EditFile {
            handle: require_pos(f.positional, "a <handle>")?,
            path: f
                .positional2
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("app edit requires a <path> argument".into()))?,
            from: f
                .from
                .ok_or_else(|| CliError::Usage("app edit requires --from <file>".into()))?,
        }),
        "lock" => Ok(AppSub::Lock {
            handle: require_pos(f.positional, "a <handle>")?,
        }),
        "unlock" => Ok(AppSub::Unlock {
            handle: require_pos(f.positional, "a <handle>")?,
        }),
        other => Err(CliError::Usage(format!(
            "unknown app subcommand {other:?} (expected new | save | list | get | run | \
             export | scaffold | files | cat | structure | edit | lock | unlock)"
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
fn execute_new(spec: NewSpec, skill_refs: Vec<SkillRef>) -> Result<(), CliError> {
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
    env.references.skills = skill_refs;
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

/// Execute `app new | save | list | get | run | export | scaffold | files | cat | lock | unlock`.
///
/// # Errors
/// [`CliError`] on a transport / status / usage failure.
#[allow(clippy::too_many_lines)]
pub async fn execute(args: AppArgs) -> Result<(), CliError> {
    // `new` is offline — no gateway contact — UNLESS `--skill` names catalog
    // skills to attach: each resolves via `GetSkillForm` to a SkillRef
    // (instructions_ref is server-derived; hand-typing 64-hex is hostile). An
    // old server without the catalog answers UNIMPLEMENTED — surfaced clearly,
    // never silently dropped.
    if let AppSub::New(spec) = args.sub {
        if spec.skills.is_empty() {
            return execute_new(spec, Vec::new());
        }
        let resolved = args.common.resolve()?;
        let mut client = resolved.connect().await?;
        let mut refs = Vec::with_capacity(spec.skills.len());
        for name in &spec.skills {
            let resp = client
                .get_skill_form(
                    resolved.request(proto::GetSkillFormRequest { name: name.clone() })?,
                )
                .await
                .map_err(|s| {
                    if s.code() == tonic::Code::Unimplemented {
                        CliError::Usage(format!(
                            "--skill {name:?}: this server has no skill catalog \
                             (required); author the SkillRef in the envelope \
                             JSON instead, or upgrade the serve"
                        ))
                    } else {
                        CliError::from_status(s)
                    }
                })?
                .into_inner();
            if !resp.found {
                return Err(CliError::Usage(format!(
                    "--skill {name:?}: not in your skill catalog (add it with \
                     `kx skills add`, or `kx skills list` to see what's there)"
                )));
            }
            let summary = resp.summary.unwrap_or_default();
            refs.push(SkillRef {
                name: summary.name,
                instructions_ref: summary.instructions_ref,
                tools: summary.tools.into_iter().collect(),
            });
        }
        return execute_new(spec, refs);
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
            args,
        } => {
            // `--arg k=v` → a canonical JSON object the server folds into the entry
            // model step (empty ⇒ no args). Sorted by BTreeMap for a stable payload.
            let args_bytes = if args.is_empty() {
                Vec::new()
            } else {
                let map: std::collections::BTreeMap<String, String> = args.into_iter().collect();
                serde_json::to_vec(&map)
                    .map_err(|e| CliError::Usage(format!("encode --arg: {e}")))?
            };
            // Prefer the server-side RunApp (G2 — honors references.connections +
            // guards.secret_scope so a credentialed connector can be dialed). Fall back
            // to the legacy client-orchestrated GetApp -> SubmitWorkflow on an older
            // server (UNIMPLEMENTED) — that path drops the references (no secret_scope).
            let submitted = match client
                .run_app(resolved.request(proto::RunAppRequest {
                    handle: handle.clone(),
                    args: args_bytes.clone(),
                })?)
                .await
            {
                Ok(resp) => resp.into_inner(),
                Err(status) if status.code() == tonic::Code::Unimplemented => {
                    if !args_bytes.is_empty() {
                        return Err(CliError::Usage(
                            "this server does not support `kx app run --arg` (RunApp \
                             unavailable); upgrade the server, or run without --arg"
                                .into(),
                        ));
                    }
                    let resp = fetch_app(&mut client, &resolved, &handle).await?;
                    if !resp.found {
                        return Err(CliError::Usage(format!("app {handle:?} not found")));
                    }
                    let env = AppEnvelope::from_json_slice(&resp.envelope_json)
                        .map_err(|e| CliError::Usage(format!("stored envelope is invalid: {e}")))?;
                    // The legacy fallback DROPS references.connections +
                    // guards.secret_scope. If this App actually declares integrations,
                    // refuse LOUDLY rather than silently run a de-integrated workflow (the
                    // credentialed connector would never fire; the secret_scope narrowing
                    // would be lost). Only an integration-free App may take the legacy path.
                    if !env.references.connections.is_empty()
                        || !env.steering_config.guards.secret_scope.is_empty()
                    {
                        return Err(CliError::Usage(format!(
                            "app {handle:?} declares integrations (references.connections / \
                             guards.secret_scope) but this server lacks RunApp — refusing to \
                             run it de-integrated (the credentialed connector + secret_scope \
                             would be silently dropped). Upgrade the server (build with the \
                             mcp-gateway feature)."
                        )));
                    }
                    // Compile the blueprint through the ONE canonical path; the server
                    // re-resolves every warrant from the caller's grants (SN-8).
                    let dag: DagSpec = serde_json::from_value(env.blueprint).map_err(|e| {
                        CliError::Usage(format!("app blueprint is not a DagSpec: {e}"))
                    })?;
                    let req = to_request(dag)?;
                    client
                        .submit_workflow(resolved.request(req)?)
                        .await
                        .map_err(CliError::from_status)?
                        .into_inner()
                }
                Err(status) => return Err(CliError::from_status(status)),
            };
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
        AppSub::Scaffold {
            handle,
            goal,
            wait: do_wait,
            timeout_secs,
        } => {
            let resp = client
                .scaffold_app(resolved.request(proto::ScaffoldAppRequest {
                    handle: handle.clone(),
                    branch_handle: String::new(), // one-App-one-branch ⇒ server defaults to the App handle
                    instruction: goal.unwrap_or_default(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            let branch = resp.branch_handle.clone();
            println!("{}", format::render_scaffold_app(&resp, json));
            if do_wait {
                let status = poll_scaffold(
                    &mut client,
                    &resolved,
                    &branch,
                    Duration::from_secs(timeout_secs),
                )
                .await?;
                println!("{}", format::render_scaffold_status(&status, json));
                if status.phase == proto::get_scaffold_status_response::Phase::Failed as i32 {
                    return Err(CliError::Usage(format!(
                        "scaffold failed: {}",
                        status.detail
                    )));
                }
            }
            Ok(())
        }
        AppSub::Files { handle } => {
            let resp = client
                .get_branch(resolved.request(proto::GetBranchRequest { handle })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_get_branch(&resp, json));
            Ok(())
        }
        AppSub::Cat { handle, path, out } => {
            let resp = client
                .get_branch_content(resolved.request(proto::GetBranchContentRequest {
                    handle: handle.clone(),
                    path: path.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            if !resp.found {
                return Err(CliError::Usage(format!(
                    "app {handle:?} has no file {path:?} (or the App is not owned by you)"
                )));
            }
            if let Some(path) = out {
                std::fs::write(&path, &resp.payload)
                    .map_err(|e| CliError::Usage(format!("write {}: {e}", path.display())))?;
                println!("wrote {} ({} bytes)", path.display(), resp.payload.len());
            } else {
                use std::io::Write;
                std::io::stdout().write_all(&resp.payload).ok();
            }
            Ok(())
        }
        AppSub::Structure { handle } => {
            let resp = fetch_app(&mut client, &resolved, &handle).await?;
            if !resp.found {
                return Err(CliError::Usage(format!("app {handle:?} not found")));
            }
            let env = AppEnvelope::from_json_slice(&resp.envelope_json)
                .map_err(|e| CliError::Usage(format!("stored envelope is invalid: {e}")))?;
            // The blueprint IS the App's portable DagSpec structure. Validate it parses
            // (the lineage editor renders exactly this) before dumping it.
            let dag: DagSpec = serde_json::from_value(env.blueprint.clone())
                .map_err(|e| CliError::Usage(format!("app blueprint is not a DagSpec: {e}")))?;
            println!(
                "{}",
                render_app_structure(&handle, &dag, &env.blueprint, json)
            );
            Ok(())
        }
        AppSub::EditFile { handle, path, from } => {
            let payload = std::fs::read(&from)
                .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", from.display())))?;
            // PutContent the new body (server-derived ref, SN-8), then AdvanceBranch the
            // manifest path to it. A locked App refuses AdvanceBranch (LOCKED_BRANCH).
            let put = client
                .put_content(resolved.request(proto::PutContentRequest {
                    payload,
                    media_type: String::new(),
                    filename: path.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            let advanced = client
                .advance_branch(resolved.request(proto::AdvanceBranchRequest {
                    handle: handle.clone(),
                    path: path.clone(),
                    content_ref: put.content_ref,
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_advance_branch(&advanced, json));
            Ok(())
        }
        AppSub::Lock { handle } => {
            let resp = client
                .lock_app(resolved.request(proto::LockAppRequest {
                    branch_handle: handle.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_app_lock(&handle, resp.locked, json));
            Ok(())
        }
        AppSub::Unlock { handle } => {
            let resp = client
                .unlock_app(resolved.request(proto::UnlockAppRequest {
                    branch_handle: handle.clone(),
                })?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_app_lock(&handle, !resp.unlocked, json));
            Ok(())
        }
    }
}

/// Poll `GetScaffoldStatus` until the scaffold reaches a terminal phase (Done/Failed)
/// or the deadline elapses (then returns the last status — never an error on timeout,
/// so the caller can render the partial progress honestly).
async fn poll_scaffold(
    client: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    resolved: &crate::client::Resolved,
    branch_handle: &str,
    timeout: Duration,
) -> Result<proto::GetScaffoldStatusResponse, CliError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let status = client
            .get_scaffold_status(resolved.request(proto::GetScaffoldStatusRequest {
                branch_handle: branch_handle.to_string(),
            })?)
            .await
            .map_err(CliError::from_status)?
            .into_inner();
        let terminal = status.phase == proto::get_scaffold_status_response::Phase::Done as i32
            || status.phase == proto::get_scaffold_status_response::Phase::Failed as i32;
        if terminal || std::time::Instant::now() >= deadline {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
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

/// Render an App's blueprint structure (POC-5d `app structure`). `--json` emits the
/// raw blueprint DagSpec JSON; otherwise a human summary of steps + edges. The kind
/// inference mirrors the CLI `StepSpec::resolve_kind` / the SDK `inferKind` (an
/// explicit `kind`, else model fields ⇒ model, a tool contract ⇒ tool, else pure).
fn render_app_structure(
    handle: &str,
    dag: &DagSpec,
    raw: &serde_json::Value,
    json: bool,
) -> String {
    use std::fmt::Write as _;
    if json {
        return serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string());
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "app {handle}  ({} step{}, {} edge{})",
        dag.steps.len(),
        if dag.steps.len() == 1 { "" } else { "s" },
        dag.edges.len(),
        if dag.edges.len() == 1 { "" } else { "s" },
    );
    for (i, s) in dag.steps.iter().enumerate() {
        let kind: &str = match s.kind.as_deref() {
            Some(k) => k,
            None if !s.model_id.is_empty() || !s.prompt.is_empty() => "model",
            None if !s.tool_contract.is_empty() => "tool",
            None => "pure",
        };
        let mut line = format!("  [{i}] {kind}");
        if !s.model_id.is_empty() {
            let _ = write!(line, "  model={}", s.model_id);
        }
        if !s.tool_contract.is_empty() {
            let tools: Vec<String> = s
                .tool_contract
                .iter()
                .map(|(k, v)| format!("{k}@{v}"))
                .collect();
            let _ = write!(line, "  tools=[{}]", tools.join(", "));
        }
        if let Some(t) = s.max_turns {
            let _ = write!(line, "  max_turns={t}");
        }
        if let Some(t) = s.max_tool_calls {
            let _ = write!(line, "  max_tool_calls={t}");
        }
        let _ = writeln!(out, "{line}");
    }
    for e in &dag.edges {
        let label = if e.edge.is_empty() { "data" } else { &e.edge };
        let _ = writeln!(out, "  edge {} -> {} ({label})", e.parent, e.child);
    }
    out.trim_end().to_string()
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
    fn parse_scaffold_with_goal_and_wait() {
        match parse_ok(&[
            "scaffold",
            "apps/local/echo",
            "--goal",
            "summarize PDFs",
            "--wait",
        ])
        .sub
        {
            AppSub::Scaffold {
                handle, goal, wait, ..
            } => {
                assert_eq!(handle, "apps/local/echo");
                assert_eq!(goal.as_deref(), Some("summarize PDFs"));
                assert!(wait);
            }
            other => panic!("expected Scaffold, got {other:?}"),
        }
    }

    #[test]
    fn parse_scaffold_requires_handle() {
        let err = parse(["scaffold"].iter().map(ToString::to_string)).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn parse_files_and_cat() {
        assert!(matches!(
            parse_ok(&["files", "apps/local/echo"]).sub,
            AppSub::Files { .. }
        ));
        match parse_ok(&["cat", "apps/local/echo", "README.md"]).sub {
            AppSub::Cat { handle, path, .. } => {
                assert_eq!(handle, "apps/local/echo");
                assert_eq!(path, "README.md");
            }
            other => panic!("expected Cat, got {other:?}"),
        }
    }

    #[test]
    fn parse_cat_requires_path() {
        let err = parse(["cat", "apps/local/echo"].iter().map(ToString::to_string)).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn parse_lock_and_unlock() {
        assert!(matches!(
            parse_ok(&["lock", "apps/local/echo"]).sub,
            AppSub::Lock { .. }
        ));
        assert!(matches!(
            parse_ok(&["unlock", "apps/local/echo"]).sub,
            AppSub::Unlock { .. }
        ));
    }

    #[test]
    fn parse_structure() {
        match parse_ok(&["structure", "apps/local/echo"]).sub {
            AppSub::Structure { handle } => assert_eq!(handle, "apps/local/echo"),
            other => panic!("expected Structure, got {other:?}"),
        }
    }

    #[test]
    fn parse_edit_with_from() {
        match parse_ok(&[
            "edit",
            "apps/local/echo",
            "README.md",
            "--from",
            "/tmp/body.txt",
        ])
        .sub
        {
            AppSub::EditFile { handle, path, from } => {
                assert_eq!(handle, "apps/local/echo");
                assert_eq!(path, "README.md");
                assert_eq!(from, PathBuf::from("/tmp/body.txt"));
            }
            other => panic!("expected EditFile, got {other:?}"),
        }
    }

    #[test]
    fn parse_edit_requires_path_and_from() {
        // missing --from
        let err = parse(
            ["edit", "apps/local/echo", "README.md"]
                .iter()
                .map(ToString::to_string),
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        // missing <path>
        let err = parse(
            ["edit", "apps/local/echo", "--from", "/tmp/b.txt"]
                .iter()
                .map(ToString::to_string),
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn render_structure_human_lists_steps_and_edges() {
        let raw = serde_json::json!({
            "seed": 0,
            "steps": [
                { "kind": "model", "prompt": "go", "tool_contract": { "mcp-echo/echo": "1" }, "max_turns": 4 },
                { "kind": "pure" }
            ],
            "edges": [ { "parent": 0, "child": 1 } ]
        });
        let dag: DagSpec = serde_json::from_value(raw.clone()).unwrap();
        let human = render_app_structure("apps/local/echo", &dag, &raw, false);
        assert!(human.contains("2 steps, 1 edge"));
        assert!(human.contains("[0] model"));
        assert!(human.contains("tools=[mcp-echo/echo@1]"));
        assert!(human.contains("max_turns=4"));
        assert!(human.contains("[1] pure"));
        assert!(human.contains("edge 0 -> 1 (data)"));
        // --json emits the raw blueprint verbatim
        let j = render_app_structure("apps/local/echo", &dag, &raw, true);
        assert!(j.contains("\"mcp-echo/echo\""));
    }

    #[test]
    fn default_handle_sanitizes() {
        assert_eq!(default_handle("My Echo App!"), "apps/local/my-echo-app");
        assert_eq!(default_handle("..."), "apps/local/app");
    }
}
