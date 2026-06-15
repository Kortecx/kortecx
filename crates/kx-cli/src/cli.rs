//! Top-level verb dispatch + hand-rolled arg parsing (no clap — matures at
//! D127 step 1.2). `run` / `replay` / `digest` forward to the [`kx_runtime`]
//! engine; `serve` forwards to [`kx_gateway`]; the rest are gRPC client verbs.

use std::process::ExitCode;

use serde_json::json;

use crate::error::CliError;
use crate::verbs;

/// The default `serve` listen address (and the conventional client endpoint —
/// see [`crate::client::DEFAULT_ENDPOINT`]).
pub const DEFAULT_LISTEN: &str = "127.0.0.1:50151";

/// Env var naming the base data directory for a zero-config `kx serve`. When set
/// (and non-empty) it overrides the `~/.kortecx` default — useful for sandboxing
/// (tests) or pinning the runtime's durable state to a chosen disk.
pub const KX_DATA_DIR_ENV: &str = "KX_DATA_DIR";

/// Fallback base data-dir name (under `$HOME`, or the CWD when `$HOME` is unset)
/// used by zero-config `serve` when [`KX_DATA_DIR_ENV`] is not set. Stable across
/// restarts so a re-served runtime keeps its journal, telemetry, and content.
pub const KX_DATA_DIR_NAME: &str = ".kortecx";

/// One-line-per-section usage, printed on `--help` and on a parse error.
pub const USAGE: &str = "\
usage: kx <command> [args]

  engine (local, no server):
    kx run|replay|digest --journal <path> --content <dir> [--crash-at <pt>] [--checkpoint-every N]
                         [--audit-log <path>] [--json]

  server:
    kx serve --dev-allow-local [--journal <path>] [--content <dir>] [--catalog-dir <dir>]
             [--listen <addr:port>] [--ws-listen <addr:port>]
             [--auth-token <tok>=<party>]... [--auth-token-file <path>]
             [--max-lease N] [--tls-cert <p> --tls-key <p>] [--cors-origin <scheme://host[:port]>]...
             (zero-config: omit --journal/--content/--catalog-dir and they auto-resolve under
              $KX_DATA_DIR (default ~/.kortecx), created on first run + REUSED across restarts;
              the resolved paths + endpoints print as a startup banner. An auth posture is REQUIRED:
              --dev-allow-local (alias --allow-local-dev, loopback only) or --auth-token(-file).
              --listen defaults to 127.0.0.1:50151; --ws-listen — the live-event WebSocket — to :50152;
              --cors-origin enables the gRPC-web browser shim for the listed origins, deny-by-default)

  client verbs (gRPC over the gateway; common flags: --endpoint <url> --token <t> | --token-file <p> --tls-ca <path> --json):
    kx invoke <handle> --args <json> [--args-file <path>] [--wait] [--stream] [--timeout-secs N] [--out <file>]
    kx chain run \"<dsl>\" --tasks <tasks.json> [--seed N] [--wait] [--timeout-secs N] [--out <file>]
                                                 (string-DSL DAG: a > [b & c]; see `kx help chain`)
    kx projection --instance <hex16> [--at-seq N]
    kx runs list [--limit N] [--before-seq N]    (durable run history, newest-first)
    kx runs rerun <instance-hex16> [--set k=v]   (re-run a prior run with edited args)
    kx mote show <instance-hex16> <mote-hex32>   (display-only definition inspection)
    kx content get --ref <hex32> [--instance <hex16>] [--out <file>]   (no --instance = the uploads scope)
    kx content put <file> [--media-type <mime>] [--filename <name>]
    kx events --instance <hex16> [--since N] [--follow]
    kx events --all [--since N] [--follow]       (the global cross-run event tail)
    kx telemetry list [--instance <hex16>] [--mote <hex32>] [--limit N] [--before-seq N]
    kx feedback submit --rating up|down --message-id <id> [--instance <hex16>] [--comment <s>]
    kx feedback list [--instance <hex16>] [--limit N] [--before-rowid N]
    kx replan list [--limit N]                   (re-plan rounds, newest-first)
    kx react list [--instance <hex16>] [--limit N]     (ReAct turns, newest-first)
    kx capture list [--instance <hex16>] [--limit N]   (captured actions, newest-first)
    kx signatures list | get --id <hex32> | register --manifest-file <path>
    kx tools list | score --intent <text> --tool <id>@<ver>... [--language-tag <t>]... [--tolerance-threshold-bp N]
    kx recipe list | search <intent> [--keyword <k>]... [--limit N]   (advisory recipe discovery)
    kx models list                              (display-only model discovery)
    kx health                                   (grpc.health.v1 liveness; exit 0 iff SERVING)

    --endpoint defaults to http://127.0.0.1:50151

  kx --help | --version | help <command>";

/// A parsed invocation.
#[derive(Debug)]
pub enum Cli {
    /// Print usage (global, or for a specific command) and exit 0.
    Help(Option<String>),
    /// Print the version and exit 0.
    Version,
    /// Forward to the engine: the full `<mode> ...` argv + whether `--json` was set.
    Runtime {
        /// The engine argv with the mode (`run`/`replay`/`digest`) re-prepended.
        argv: Vec<String>,
        /// Whether `--json` was requested (stripped before forwarding).
        json: bool,
    },
    /// Forward to the gateway server: the `serve` args (verb stripped).
    Serve(Vec<String>),
    /// `invoke` a published blueprint by handle (wire-legacy: recipe).
    Invoke(verbs::invoke::InvokeArgs),
    /// `blueprint run` — author a Tier-1 DAG and run it (SubmitWorkflow).
    Blueprint(verbs::blueprint::BlueprintArgs),
    /// `chain run` — author a Tier-1 DAG from the string-DSL and run it (SubmitWorkflow).
    Chain(verbs::chain::ChainArgs),
    /// Render a run as a DAG of Mote states.
    Projection(verbs::projection::ProjectionArgs),
    /// Durable run history (Batch B `ListRuns`; read-only).
    Runs(verbs::runs::RunsArgs),
    /// Recipe catalog + advisory discovery (PR-4 Batch D `ListRecipes`/`SearchRecipes`).
    Recipe(verbs::recipe::RecipeArgs),
    /// Per-mote definition inspection (Batch B `GetMoteDetail`; display-only).
    Mote(verbs::mote::MoteArgs),
    /// Fetch a committed result.
    Content(verbs::content::ContentArgs),
    /// Stream/poll a run's event deltas (or the global cross-run tail).
    Events(verbs::events::EventsArgs),
    /// Mote execution telemetry (Batch C `ListMoteTelemetry`; display-only).
    Telemetry(verbs::telemetry::TelemetryArgs),
    /// User 👍/👎 feedback on an answer (PR-4.1 `SubmitFeedback`/`ListFeedback`).
    Feedback(verbs::feedback::FeedbackArgs),
    /// Re-plan-round observability (PR-2c-2 `ListReplanRounds`; read-only).
    Replan(verbs::replan::ReplanArgs),
    /// ReAct-turn observability (PR-2d-1 `ListReactTurns`; read-only).
    React(verbs::react::ReactArgs),
    /// Captured-action records (`ListCaptureRecords`; read-only join keys).
    Capture(verbs::capture::CaptureArgs),
    /// Catalog signature RPCs.
    Signatures(verbs::signatures::SignaturesArgs),
    /// Advisory toolscout RPCs (tool discovery + TaskBundle preview).
    Tools(verbs::tools::ToolsArgs),
    /// Model discovery (Batch A `ListModels`; display-only).
    Models(verbs::models::ModelsArgs),
    /// Liveness/readiness probe (grpc.health.v1).
    Health(verbs::health::HealthArgs),
}

impl Cli {
    /// Parse `argv` (excluding the program name). An empty argv is `--help`.
    pub fn from_args<I, S>(args: I) -> Result<Cli, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = args.into_iter().map(Into::into);
        match args.next().as_deref() {
            None | Some("--help" | "-h") => Ok(Cli::Help(None)),
            Some("help") => Ok(Cli::Help(args.next())),
            Some("--version" | "-V") => Ok(Cli::Version),
            Some(verb @ ("run" | "replay" | "digest")) => {
                // The engine parser doesn't know `--json`; strip it here and
                // forward the rest with the mode re-prepended.
                let mut engine_argv = vec![verb.to_string()];
                let mut json = false;
                for a in args {
                    if a == "--json" {
                        json = true;
                    } else {
                        engine_argv.push(a);
                    }
                }
                Ok(Cli::Runtime {
                    argv: engine_argv,
                    json,
                })
            }
            Some("serve") => Ok(Cli::Serve(args.collect())),
            Some("invoke") => Ok(Cli::Invoke(verbs::invoke::parse(args)?)),
            Some("blueprint") => Ok(Cli::Blueprint(verbs::blueprint::parse(args)?)),
            Some("chain") => Ok(Cli::Chain(verbs::chain::parse(args)?)),
            Some("projection") => Ok(Cli::Projection(verbs::projection::parse(args)?)),
            Some("runs") => Ok(Cli::Runs(verbs::runs::parse(args)?)),
            Some("recipe") => Ok(Cli::Recipe(verbs::recipe::parse(args)?)),
            Some("mote") => Ok(Cli::Mote(verbs::mote::parse(args)?)),
            Some("content") => Ok(Cli::Content(verbs::content::parse(args)?)),
            Some("events") => Ok(Cli::Events(verbs::events::parse(args)?)),
            Some("telemetry") => Ok(Cli::Telemetry(verbs::telemetry::parse(args)?)),
            Some("feedback") => Ok(Cli::Feedback(verbs::feedback::parse(args)?)),
            Some("replan") => Ok(Cli::Replan(verbs::replan::parse(args)?)),
            Some("react") => Ok(Cli::React(verbs::react::parse(args)?)),
            Some("capture") => Ok(Cli::Capture(verbs::capture::parse(args)?)),
            Some("signatures") => Ok(Cli::Signatures(verbs::signatures::parse(args)?)),
            Some("tools") => Ok(Cli::Tools(verbs::tools::parse(args)?)),
            Some("models") => Ok(Cli::Models(verbs::models::parse(args)?)),
            Some("health") => Ok(Cli::Health(verbs::health::parse(args)?)),
            Some(other) => Err(CliError::Usage(format!(
                "unknown command {other:?} (try `kx --help`)"
            ))),
        }
    }
}

/// Parse `argv` and run, returning the process exit code. The single entry point
/// the binary calls.
pub async fn run<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let cli = match Cli::from_args(args) {
        Ok(c) => c,
        Err(e) => return render_error(&e),
    };
    match dispatch(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => render_error(&e),
    }
}

/// Render an error to stderr (with the usage block for usage/config errors) and
/// return its exit code.
fn render_error(e: &CliError) -> ExitCode {
    if e.is_usage() {
        eprintln!("{USAGE}");
    }
    eprintln!("kx: {e}");
    e.exit_code()
}

async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli {
        Cli::Help(None) => {
            println!("{USAGE}");
            Ok(())
        }
        Cli::Help(Some(cmd)) => {
            println!("{}", help_for(&cmd));
            Ok(())
        }
        Cli::Version => {
            println!("kx {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Cli::Runtime { argv, json } => run_engine(argv, json).await,
        Cli::Serve(rest) => serve(rest).await,
        Cli::Invoke(a) => verbs::invoke::execute(a).await,
        Cli::Blueprint(a) => verbs::blueprint::execute(a).await,
        Cli::Chain(a) => verbs::chain::execute(a).await,
        Cli::Projection(a) => verbs::projection::execute(a).await,
        Cli::Runs(a) => verbs::runs::execute(a).await,
        Cli::Recipe(a) => verbs::recipe::execute(a).await,
        Cli::Mote(a) => verbs::mote::execute(a).await,
        Cli::Content(a) => verbs::content::execute(a).await,
        Cli::Events(a) => verbs::events::execute(a).await,
        Cli::Telemetry(a) => verbs::telemetry::execute(a).await,
        Cli::Feedback(a) => verbs::feedback::execute(a).await,
        Cli::Replan(a) => verbs::replan::execute(a).await,
        Cli::React(a) => verbs::react::execute(a).await,
        Cli::Capture(a) => verbs::capture::execute(a).await,
        Cli::Signatures(a) => verbs::signatures::execute(a).await,
        Cli::Tools(a) => verbs::tools::execute(a).await,
        Cli::Models(a) => verbs::models::execute(a).await,
        Cli::Health(a) => verbs::health::execute(a).await,
    }
}

/// The engine result, distinguished so `digest` and `run`/`replay` render
/// differently (parity with the `kx-runtime` binary's output).
enum EngineOut {
    Digest(kx_runtime::ProjectionDigest),
    Run(kx_runtime::RunOutcome),
}

/// Forward to the engine on a blocking thread (the orchestrator is CPU-bound and
/// reused VERBATIM — the projection-digest invariant is preserved).
async fn run_engine(argv: Vec<String>, json: bool) -> Result<(), CliError> {
    let config =
        kx_runtime::RuntimeConfig::from_args(argv).map_err(|e| CliError::Config(e.to_string()))?;
    let out =
        tokio::task::spawn_blocking(move || -> Result<EngineOut, kx_runtime::RuntimeError> {
            match config.mode {
                kx_runtime::Mode::Digest => kx_runtime::digest_only(&config).map(EngineOut::Digest),
                kx_runtime::Mode::Run | kx_runtime::Mode::Replay => {
                    kx_runtime::run(&config).map(EngineOut::Run)
                }
            }
        })
        .await
        .map_err(|e| CliError::Runtime(format!("engine task panicked: {e}")))?
        .map_err(|e| CliError::Runtime(e.to_string()))?;

    let rendered = match out {
        EngineOut::Digest(d) => {
            let hex = d.to_hex();
            if json {
                json!({ "digest": hex }).to_string()
            } else {
                hex
            }
        }
        EngineOut::Run(o) => {
            let hex = o.digest.to_hex();
            if json {
                json!({ "digest": hex, "committed": o.committed, "total": o.total }).to_string()
            } else {
                format!("{hex} ({}/{} committed)", o.committed, o.total)
            }
        }
    };
    println!("{rendered}");
    Ok(())
}

/// Forward to the embedded gateway server, defaulting `--listen` when absent.
async fn serve(rest: Vec<String>) -> Result<(), CliError> {
    require_auth_posture(&rest)?;
    let rest = inject_data_dir_defaults(rest)?;
    let argv = inject_listen_default(rest);
    let cli = kx_gateway::Cli::from_args(std::iter::once("serve".to_string()).chain(argv))
        .map_err(|e| CliError::Config(e.to_string()))?;
    match cli {
        kx_gateway::Cli::Serve(cfg) => kx_gateway::serve(cfg).await.map_err(map_gateway_err),
        // Unreachable (we always pass "serve"), but keep the match total + safe.
        kx_gateway::Cli::Help => {
            println!("{}", kx_gateway::USAGE);
            Ok(())
        }
        kx_gateway::Cli::Version => {
            println!("kx-gateway");
            Ok(())
        }
    }
}

/// A gateway config error keeps exit-2 semantics; a bind/runtime failure is exit 1.
fn map_gateway_err(e: kx_gateway::GatewayError) -> CliError {
    match e {
        kx_gateway::GatewayError::Config(m) => CliError::Config(m),
        other => CliError::Runtime(other.to_string()),
    }
}

/// Inject the default `--listen` if the operator didn't pass one.
fn inject_listen_default(mut rest: Vec<String>) -> Vec<String> {
    if !rest.iter().any(|a| a == "--listen") {
        rest.push("--listen".to_string());
        rest.push(DEFAULT_LISTEN.to_string());
    }
    rest
}

/// Resolve the base data directory for a zero-config `kx serve`:
/// `$KX_DATA_DIR` → else `$HOME/.kortecx` → else `./.kortecx`. STABLE across
/// restarts (no random suffix) so a re-served runtime keeps its journal,
/// telemetry, capture, and content. No `dirs` crate (Linux CI + Apple-Silicon
/// targets only — SN-7).
fn resolve_base_data_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(v) = std::env::var_os(KX_DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(v);
    }
    match std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        Some(home) => PathBuf::from(home).join(KX_DATA_DIR_NAME),
        None => PathBuf::from(".").join(KX_DATA_DIR_NAME),
    }
}

/// Inject a zero-config data layout (`--journal` / `--content` / `--catalog-dir`
/// under a STABLE, durable base dir, [`resolve_base_data_dir`]) for the
/// no-flags `kx serve --dev-allow-local` path.
///
/// ALL-OR-NOTHING: this fires ONLY when the operator gave NO data-path flag at
/// all. If ANY of `--journal`/`--content`/`--catalog-dir` is present, the
/// operator owns the layout and we return `rest` untouched — the gateway then
/// applies its own defaults (notably `catalog_dir` → the journal's parent). A
/// partial inject would be a footgun: a `--journal X --content Y` invocation
/// (without `--catalog-dir`) would otherwise get its catalog REDIRECTED to the
/// shared base dir, colliding the membership/telemetry sidecars across every
/// gateway that shares the base (the cause of the test-suite breakage).
///
/// The base is a durable dir (never a `tempfile`, which would delete the data
/// the operator wants to inspect); we create only the dirs we inject so the
/// gateway's stores open cleanly (SQLite needs the journal's parent to exist).
fn inject_data_dir_defaults(mut rest: Vec<String>) -> Result<Vec<String>, CliError> {
    let has = |name: &str| rest.iter().any(|a| a == name);
    // Any explicit data path ⇒ respect the operator's layout entirely.
    if has("--journal") || has("--content") || has("--catalog-dir") {
        return Ok(rest);
    }
    let base = resolve_base_data_dir();
    let mkdir = |p: &std::path::Path| -> Result<(), CliError> {
        std::fs::create_dir_all(p)
            .map_err(|e| CliError::Config(format!("create data dir {}: {e}", p.display())))
    };
    let content = base.join("content");
    let catalog = base.join("catalog");
    mkdir(&base)?;
    mkdir(&content)?;
    mkdir(&catalog)?;
    rest.push("--journal".to_string());
    rest.push(base.join("kx.db").to_string_lossy().into_owned());
    rest.push("--content".to_string());
    rest.push(content.to_string_lossy().into_owned());
    rest.push("--catalog-dir".to_string());
    rest.push(catalog.to_string_lossy().into_owned());
    Ok(rest)
}

/// Fail closed: `kx serve` must pick an explicit auth posture — we NEVER inject
/// one. Silently defaulting to a no-auth server is a security regression (GR8),
/// so a bare `kx serve` with neither the dev loopback flag nor a token source
/// errors (exit 2) with the exact remediation. The gateway is likewise
/// deny-by-default; this surfaces the requirement earlier as a clean config
/// error instead of a server that starts and rejects every request.
fn require_auth_posture(rest: &[String]) -> Result<(), CliError> {
    let has_auth = rest.iter().any(|a| {
        matches!(
            a.as_str(),
            "--dev-allow-local" | "--allow-local-dev" | "--auth-token" | "--auth-token-file"
        )
    });
    if has_auth {
        return Ok(());
    }
    Err(CliError::Config(
        "kx serve refuses unauthenticated access. Re-run with --dev-allow-local \
         (loopback-only dev access), or --auth-token <token>=<party> / \
         --auth-token-file <path> for token auth."
            .to_string(),
    ))
}

/// Longer help for a single command (`kx help <command>`). A flat text table —
/// one arm per verb; splitting it would scatter the help corpus for no clarity
/// gain (the `start_impl` precedent). Allow the length.
#[allow(clippy::too_many_lines)]
fn help_for(cmd: &str) -> String {
    match cmd {
        "run" | "replay" | "digest" => "\
kx run|replay|digest --journal <path> --content <dir> [--crash-at <pt>] [--checkpoint-every N]
                     [--audit-log <path>] [--json]
  Forwards to the kx-runtime engine. `run` drives the canonical demo from scratch;
  `replay` recovers + finishes an existing journal; `digest` prints the projection
  digest. Output is parity-identical to the `kx-runtime` binary.
  --audit-log <path> writes a best-effort JSONL audit trail of the run lifecycle
  (off the truth path; never changes the digest). Honored by run/replay."
            .into(),
        "serve" => "\
kx serve --dev-allow-local [--journal <path>] [--content <dir>] [--catalog-dir <dir>] [--listen <addr:port>] ...
  Hosts the embedded single-system gateway. ZERO-CONFIG: omit --journal/--content/--catalog-dir
  and they auto-resolve under $KX_DATA_DIR (default ~/.kortecx), created on first run and REUSED
  across restarts so the journal, telemetry, capture, and content persist. The resolved data dir +
  every store path + the gRPC/WebSocket/console endpoints print as a startup banner for reference.
  --listen (gRPC) defaults to 127.0.0.1:50151; --ws-listen (the R5 live-event WebSocket bridge) to
  127.0.0.1:50152. Web console: a console-build kx (the prebuilt release) also serves the embedded
  browser console at http://127.0.0.1:50180 — override with --console-listen <addr:port> (loopback
  only) or disable with --no-console.
  Deny-all by default: an auth posture is REQUIRED — pass --dev-allow-local (alias --allow-local-dev,
  loopback only) or --auth-token(-file); a bare `kx serve` with neither errors with a hint.
  Browser SPAs: --cors-origin <scheme://host[:port]> (repeatable, deny-by-default) enables the
  gRPC-web shim for the listed origins (pair with --tls-cert/--tls-key for https)."
            .into(),
        "invoke" => "\
kx invoke <handle> --args <json> [--args-file <path>] [--wait] [--stream] [--timeout-secs N] [--out <file>] [client flags]
  Bind a PUBLISHED blueprint (wire-legacy: recipe) by handle (e.g. kx/recipes/echo) to JSON args and run it.
  With --wait, poll to completion and print the committed result (run the runtime like
  a function). Without --wait, print the async handle (instance_id/terminal_mote_id).
  With --stream, print the terminal model mote's tokens live as they generate (advisory;
  the committed result stays the authority), then resolve — handy for chat/vision recipes."
            .into(),
        "blueprint" => "\
kx blueprint run --file <dag.json> [--wait] [--timeout-secs N] [--out <file>] [client flags]
  Author a Tier-1 DAG (a vetted palette of PURE / MODEL steps + DATA/CONTROL edges)
  and run it via SubmitWorkflow. The server COMPILES the DAG, derives all identity,
  and builds every warrant from the party's grants (SN-8) — the client sends only the
  topology + params. The authored run is then viewable in the console (Runs, Monitoring).
  JSON: { \"seed\": N, \"steps\": [{\"kind\":\"pure\"|\"model\", \"prompt\":..., \"params\":{..}}],
          \"edges\": [{\"parent\":i, \"child\":j, \"edge\":\"data\"|\"control\"}], \"execution_mode\":\"frozen\" }"
            .into(),
        "chain" => "\
kx chain run \"<dsl>\" --tasks <tasks.json> [--seed N] [--wait] [--timeout-secs N] [--out <file>] [client flags]
  Author a Tier-1 DAG from the kortecx Chains STRING-DSL and run it via SubmitWorkflow
  (the same compile + warrant path `blueprint run` uses — a chain only changes how the
  topology is AUTHORED). The positional <dsl> composes task handles with operators:
    >   sequential (a DATA edge parent -> child), tightest binary
    &   parallel merge (no edge), tighter than |
    |   parallel merge (no edge), loosest
    [ ] grouping (precedence override)
  Precedence (matches Python >> / & / |), tightest -> loosest: [ ] > `>` > & > |.
  A handle that appears more than once is the SAME node (reuse builds DAGs). Examples:
    \"a > [b & c]\" fans out (a->b, a->c); \"[a & b] > c\" fans in (a->c, b->c).
  --tasks is a JSON object map { \"a\": {\"kind\":\"pure\"|\"model\", \"prompt\":..., \"params\":{..}}, ... };
  each value is a step definition (P1 palette: pure | model). Tasks defined but unused are
  ignored. Errors fail closed: empty/empty-group -> parse; an unknown handle; a cycle."
            .into(),
        "projection" => "\
kx projection --instance <hex16> [--at-seq N] [client flags]
  Render a run as a DAG: each Mote's state, nd-class, result ref, and committed seq."
            .into(),
        "runs" => "\
kx runs list [--limit N] [--before-seq N] [client flags]
  Durable run history (Batch B): every registered run, newest-first, from one
  server-side journal fold. --limit caps the page (server max 500); --before-seq
  pages older runs (pass the last page's lowest registered_seq). Read-only.

kx runs rerun <instance-hex16> [--set k=v]... [--wait] [--timeout-secs N] [--out PATH] [client flags]
  Re-run with changes (PR-D): fetch the args a run was submitted with
  (GetRunInputs), overlay each --set key=value edit, and re-invoke. A value that
  parses as JSON keeps its type (--set count=3 → 3); otherwise it is a string.
  Only the changed sub-DAG recomputes; an unchanged re-run returns the existing
  result (idempotent). Same admission as `kx invoke` (never SubmitRun). An old
  gateway / a run with no captured args degrades honestly."
            .into(),
        "mote" => "\
kx mote show <instance-hex16> <mote-hex32> [client flags]
  Display-only definition inspection (Batch B): resolve a committed Mote's
  def hash to its admitted definition — step kind, model, prompt, params
  (capped), tool contract, nd-class, effect pattern. An uncommitted mote (or
  one admitted by a pre-Batch-B binary) answers def_found: false honestly.
  SN-8: nothing shown here authorizes anything."
            .into(),
        "content" => "\
kx content get --ref <hex32> [--instance <hex16>] [--out <file>] [client flags]
kx content put <file> [--media-type <mime>] [--filename <name>] [client flags]
  get: fetch a blob. With --instance the run scope (the run's committed result
  refs); WITHOUT it the UPLOADS scope (refs you uploaded). Writes RAW bytes to
  stdout (binary-safe, no newline); --out <file> saves; --json hex-encodes.
  The original flag-form `kx content --ref … --instance …` still works.
  put: upload a file to the gateway's content store (a content-store write,
  never a journal write). Prints the SERVER-derived blake3 ref + whether the
  blob already existed; --media-type/--filename are advisory audit fields.
  The server caps the payload fail-closed (kx serve --content-max-bytes)."
            .into(),
        "events" => "\
kx events --instance <hex16> [--since N] [--follow] [client flags]
kx events --all [--since N] [--follow] [client flags]
  Print event deltas. --instance streams ONE run's deltas (the frozen per-run
  cursor); --all streams the operator-global cross-run tail (Batch C) — every
  delta stamped with its run's instance_id (watermark attribution; empty before
  any registration) plus the run_registered \"run started\" marker the per-run
  cursor never carries. The two forms are mutually exclusive. Without --follow
  this catches up to the current journal boundary and stops; --follow keeps the
  live tail open until Ctrl-C, transparently resuming from the last next_seq if
  the server drops a slow consumer."
            .into(),
        "telemetry" => "\
kx telemetry list [--instance <hex16>] [--mote <hex32>] [--limit N] [--before-seq N] [client flags]
  Mote execution telemetry (Batch C): host-recorded exhaust as motes actually
  ran — wall-clock, model usage, the fired tool. Newest-first; --limit caps the
  page (server clamps 1..=500, default 200); --before-seq pages older rows
  (pass the last page's lowest seq). Lives in a rebuildable-to-empty sidecar:
  AUDIT/DISPLAY ONLY — never truth, identity, or a digest input. input_tokens
  is never set in OSS (the frozen backend seam reports no input count). A
  gateway without the sidecar answers Unimplemented (upgrade the serve)."
            .into(),
        "feedback" => "\
kx feedback submit --rating up|down --message-id <id> [--instance <hex16>] [--mote <hex32>] \
[--content-ref <hex32>] [--comment <s>] [--handle <s>] [--model <s>] [client flags]
kx feedback list [--instance <hex16>] [--limit N] [--before-rowid N] [client flags]
  Record + read back 👍/👎 feedback on an answer (PR-4.1). The caller principal +
  the feedback_id are server-derived (SN-8); re-rating the SAME answer overwrites.
  --message-id is the stable per-answer key (required on submit). list is
  newest-first; --limit caps the page (server clamps 1..=500, default 200);
  --before-rowid pages older rows. Lives in a rebuildable-to-empty feedback.db
  sidecar: ADVISORY/DISPLAY ONLY — never truth, identity, or a digest input. A
  gateway without the sidecar answers Unimplemented (upgrade the serve)."
            .into(),
        "replan" => "\
kx replan list [--limit N] [client flags]
  Re-plan-round observability (read-only): the durable ReplanRound facts the
  live re-plan-on-failure loop commits — round index (0 = the initial-plan
  anchor), the shaper Mote, the resolved model, the failed steps that triggered
  the round, and whether the model escalated to a human (the run quiesces).
  Newest-first; operator-global on single-node OSS."
            .into(),
        "react" => "\
kx react list [--instance <hex16>] [--limit N] [client flags]
  ReAct-turn observability (read-only): the durable ReactRound facts the live
  ReAct chain commits — each turn's run-salted Mote id, its settled branch
  (pending | answer | tool | dead_lettered), the fired tool for a tool branch,
  and the run's durable budget caps. Newest-first; --instance scopes to one
  run's chain."
            .into(),
        "capture" => "\
kx capture list [--instance <hex16>] [--limit N] [client flags]
  The Morphic Data Engine capture read surface (read-only): durably-captured
  ACTION records — a committed Mote's join keys (mote / instance / result_ref /
  nd-class / seq), plus the ReAct turn/branch when the Mote is a ReAct turn.
  JOIN-KEY-ONLY by construction (no payload/reasoning fields). Newest-first;
  --instance scopes to one run."
            .into(),
        "signatures" => "\
kx signatures list | get --id <hex32> | register --manifest-file <path> [client flags]
  Browse / fetch / register catalog task signatures over the gateway."
            .into(),
        "tools" => "\
kx tools list [client flags]
kx tools score --intent <text> --tool <id>@<ver> [--tool <id>@<ver>]... [--language-tag <t>]...
               [--tolerance-threshold-bp N] [client flags]
  Advisory MCP-tool discovery + TaskBundle preview (W1.A5). `list` shows the
  gateway's registered tool manifests; `score` ranks every manifest against an
  intent (integer basis points: 10000 = exact keyword hit; lower = similar) and
  dry-runs the real lowering gate (verdict: would-lower / unavailable / refused).
  ADVISORY ONLY (SN-8): scores and the verdict NEVER authorize a tool — the
  exact (name, version) grant gate stays the broker's. No warrant is sent."
            .into(),
        "recipe" => "\
kx recipe list [client flags]
kx recipe search <intent> [--keyword <k>]... [--limit N] [client flags]
  Recipe catalog + advisory discovery (PR-4 Batch D). `list` shows the gateway's
  provisioned, invocable recipe handles with their advisory metadata (description,
  tags, version); `search` ranks them against an intent (integer basis points:
  10000 = exact handle; lower = name/tag/description match). ADVISORY ONLY (SN-8):
  scores NEVER authorize a recipe — `kx invoke` stays the gate. No warrant is sent."
            .into(),
        "models" => "\
kx models list [client flags]
  Display-only model discovery (Batch A): the models the connected gateway
  serves (id, modalities, context window, serving flag). An FFI-free serve
  lists nothing. SN-8: listing a model never routes one — model selection
  stays a recipe ENUM free-param validated server-side at binding."
            .into(),
        "health" => "\
kx health [client flags]
  Probe the gateway's grpc.health.v1 liveness/readiness. Prints SERVING / NOT_SERVING
  (or --json) and exits 0 iff SERVING — a purpose-built healthcheck (the compose
  stack uses it). Unauthenticated; honors --endpoint / --tls-ca for a TLS gateway."
            .into(),
        other => format!("no help for {other:?}; try `kx --help`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_help_and_version() {
        assert!(matches!(
            Cli::from_args(Vec::<String>::new()).unwrap(),
            Cli::Help(None)
        ));
        assert!(matches!(
            Cli::from_args(["--help"]).unwrap(),
            Cli::Help(None)
        ));
        assert!(matches!(Cli::from_args(["-h"]).unwrap(), Cli::Help(None)));
        assert!(matches!(
            Cli::from_args(["help", "invoke"]).unwrap(),
            Cli::Help(Some(v)) if v == "invoke"
        ));
        assert!(matches!(
            Cli::from_args(["--version"]).unwrap(),
            Cli::Version
        ));
        assert!(matches!(Cli::from_args(["-V"]).unwrap(), Cli::Version));
    }

    /// The released binary self-reports the WORKSPACE version (the v0.1.0
    /// release-prep bump): `kx --version` prints `kx <CARGO_PKG_VERSION>` —
    /// version-agnostic (refactor-proof), but pins that the manifest version is
    /// the single source the banner reads, never a hand-mirrored literal.
    #[test]
    fn version_banner_reads_the_manifest_version() {
        let banner = format!("kx {}", env!("CARGO_PKG_VERSION"));
        assert!(
            banner.starts_with("kx 0."),
            "the banner derives from CARGO_PKG_VERSION: {banner}"
        );
        assert!(
            !env!("CARGO_PKG_VERSION").is_empty(),
            "the manifest version is the banner's single source"
        );
    }

    #[test]
    fn runtime_forwarding_preserves_verb_and_strips_json() {
        let Cli::Runtime { argv, json } = Cli::from_args([
            "digest",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--json",
        ])
        .unwrap() else {
            panic!("expected Runtime");
        };
        assert!(json, "--json is extracted");
        assert_eq!(
            argv[0], "digest",
            "the mode is re-prepended for the engine parser"
        );
        assert!(
            !argv.iter().any(|a| a == "--json"),
            "--json is not forwarded"
        );
        assert!(argv.iter().any(|a| a == "/tmp/j"));
    }

    #[test]
    fn serve_collects_rest() {
        let Cli::Serve(rest) =
            Cli::from_args(["serve", "--journal", "/tmp/j", "--content", "/tmp/c"]).unwrap()
        else {
            panic!("expected Serve");
        };
        assert_eq!(rest, vec!["--journal", "/tmp/j", "--content", "/tmp/c"]);
    }

    #[test]
    fn listen_default_injection() {
        // Absent → injected.
        let injected = inject_listen_default(vec!["--journal".into(), "/tmp/j".into()]);
        assert!(injected
            .windows(2)
            .any(|w| w[0] == "--listen" && w[1] == DEFAULT_LISTEN));
        // Present → unchanged.
        let kept = inject_listen_default(vec!["--listen".into(), "0.0.0.0:9".into()]);
        assert_eq!(kept.iter().filter(|a| *a == "--listen").count(), 1);
        assert!(kept.iter().any(|a| a == "0.0.0.0:9"));
        // And the gateway parser accepts the injected form.
        let argv = inject_listen_default(vec![
            "--journal".into(),
            "/tmp/j".into(),
            "--content".into(),
            "/tmp/c".into(),
            "--dev-allow-local".into(),
        ]);
        let cli =
            kx_gateway::Cli::from_args(std::iter::once("serve".to_string()).chain(argv)).unwrap();
        let kx_gateway::Cli::Serve(cfg) = cli else {
            panic!("expected Serve")
        };
        assert_eq!(cfg.listen.to_string(), DEFAULT_LISTEN);
    }

    #[test]
    fn unknown_command_is_usage_error() {
        assert!(Cli::from_args(["frobnicate"]).is_err());
    }

    #[test]
    fn data_dir_injection_is_noop_when_any_path_is_explicit() {
        // ALL-OR-NOTHING: if the operator gave ANY data-path flag, injection is a
        // no-op (NO env read, NO dir creation, argv byte-identical) — the operator
        // owns the layout and the gateway defaults the rest. Critically, this
        // includes the `--journal`+`--content` WITHOUT `--catalog-dir` case: a
        // partial inject there would redirect the catalog to the shared base dir
        // and collide every gateway's sidecars (the test-suite breakage).
        let cases: &[&[&str]] = &[
            &[
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--catalog-dir",
                "/tmp/cat",
                "--dev-allow-local",
            ],
            &[
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--dev-allow-local",
            ], // the bug case
            &["--journal", "/tmp/j", "--dev-allow-local"],
            &["--catalog-dir", "/tmp/cat", "--dev-allow-local"],
        ];
        for case in cases {
            let argv: Vec<String> = case.iter().map(|s| (*s).to_string()).collect();
            let out = inject_data_dir_defaults(argv.clone()).unwrap();
            assert_eq!(
                out, argv,
                "any explicit data path ⇒ argv passes through unchanged: {case:?}"
            );
        }
    }

    #[test]
    fn auth_posture_required() {
        // No posture ⇒ a clean config error that names the remediation flag.
        let err =
            require_auth_posture(&["--journal".to_string(), "/tmp/j".to_string()]).unwrap_err();
        assert!(
            matches!(&err, CliError::Config(m) if m.contains("--dev-allow-local")),
            "missing auth posture errors with a hint: {err:?}"
        );
        // Each accepted posture (incl. the alias) passes.
        for flag in [
            "--dev-allow-local",
            "--allow-local-dev",
            "--auth-token",
            "--auth-token-file",
        ] {
            assert!(
                require_auth_posture(&[flag.to_string()]).is_ok(),
                "{flag} is an accepted auth posture"
            );
        }
    }
}
