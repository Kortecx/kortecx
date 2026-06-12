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

/// One-line-per-section usage, printed on `--help` and on a parse error.
pub const USAGE: &str = "\
usage: kx <command> [args]

  engine (local, no server):
    kx run|replay|digest --journal <path> --content <dir> [--crash-at <pt>] [--checkpoint-every N]
                         [--audit-log <path>] [--json]

  server:
    kx serve --journal <path> --content <dir> [--listen <addr:port>] [--ws-listen <addr:port>]
             [--dev-allow-local] [--auth-token <tok>=<party>]... [--auth-token-file <path>]
             [--max-lease N] [--catalog-dir <dir>] [--tls-cert <p> --tls-key <p>]
             [--cors-origin <scheme://host[:port]>]...
             (--listen defaults to 127.0.0.1:50151; --ws-listen — the live-event WebSocket — to :50152;
              --cors-origin enables the gRPC-web browser shim for the listed origins, deny-by-default)

  client verbs (gRPC over the gateway; common flags: --endpoint <url> --token <t> | --token-file <p> --tls-ca <path> --json):
    kx invoke <handle> --args <json> [--args-file <path>] [--wait] [--timeout-secs N] [--out <file>]
    kx submit --demo [--wait] [--timeout-secs N] [--out <file>]
    kx projection --instance <hex16> [--at-seq N]
    kx runs list [--limit N] [--before-seq N]    (durable run history, newest-first)
    kx mote show <instance-hex16> <mote-hex32>   (display-only definition inspection)
    kx content get --ref <hex32> [--instance <hex16>] [--out <file>]   (no --instance = the uploads scope)
    kx content put <file> [--media-type <mime>] [--filename <name>]
    kx events --instance <hex16> [--since N] [--follow]
    kx signatures list | get --id <hex32> | register --manifest-file <path>
    kx tools list | score --intent <text> --tool <id>@<ver>... [--language-tag <t>]... [--tolerance-threshold-bp N]
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
    /// `submit` a built-in demo run.
    Submit(verbs::submit::SubmitArgs),
    /// `blueprint run` — author a Tier-1 DAG and run it (SubmitWorkflow).
    Blueprint(verbs::blueprint::BlueprintArgs),
    /// Render a run as a DAG of Mote states.
    Projection(verbs::projection::ProjectionArgs),
    /// Durable run history (Batch B `ListRuns`; read-only).
    Runs(verbs::runs::RunsArgs),
    /// Per-mote definition inspection (Batch B `GetMoteDetail`; display-only).
    Mote(verbs::mote::MoteArgs),
    /// Fetch a committed result.
    Content(verbs::content::ContentArgs),
    /// Stream/poll a run's event deltas.
    Events(verbs::events::EventsArgs),
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
            Some("submit") => Ok(Cli::Submit(verbs::submit::parse(args)?)),
            Some("blueprint") => Ok(Cli::Blueprint(verbs::blueprint::parse(args)?)),
            Some("projection") => Ok(Cli::Projection(verbs::projection::parse(args)?)),
            Some("runs") => Ok(Cli::Runs(verbs::runs::parse(args)?)),
            Some("mote") => Ok(Cli::Mote(verbs::mote::parse(args)?)),
            Some("content") => Ok(Cli::Content(verbs::content::parse(args)?)),
            Some("events") => Ok(Cli::Events(verbs::events::parse(args)?)),
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
        Cli::Submit(a) => verbs::submit::execute(a).await,
        Cli::Blueprint(a) => verbs::blueprint::execute(a).await,
        Cli::Projection(a) => verbs::projection::execute(a).await,
        Cli::Runs(a) => verbs::runs::execute(a).await,
        Cli::Mote(a) => verbs::mote::execute(a).await,
        Cli::Content(a) => verbs::content::execute(a).await,
        Cli::Events(a) => verbs::events::execute(a).await,
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
kx serve --journal <path> --content <dir> [--listen <addr:port>] [--ws-listen <addr:port>] [--dev-allow-local] ...
  Hosts the embedded single-system gateway. --listen (gRPC) defaults to 127.0.0.1:50151;
  --ws-listen (the R5 live-event WebSocket bridge) defaults to 127.0.0.1:50152.
  Web console: a console-build kx (the prebuilt release) also serves the embedded
  browser console at http://127.0.0.1:50180 — override with --console-listen
  <addr:port> (loopback only) or disable with --no-console.
  Deny-all by default: pass --dev-allow-local (loopback only) or --auth-token(-file).
  Browser SPAs: --cors-origin <scheme://host[:port]> (repeatable, deny-by-default) enables the
  gRPC-web shim for the listed origins (pair with --tls-cert/--tls-key for https)."
            .into(),
        "invoke" => "\
kx invoke <handle> --args <json> [--args-file <path>] [--wait] [--timeout-secs N] [--out <file>] [client flags]
  Bind a PUBLISHED blueprint (wire-legacy: recipe) by handle (e.g. kx/recipes/echo) to JSON args and run it.
  With --wait, poll to completion and print the committed result (run the runtime like
  a function). Without --wait, print the async handle (instance_id/terminal_mote_id)."
            .into(),
        "submit" => "\
kx submit --demo [--wait] [--timeout-secs N] [--out <file>] [client flags]
  Submit a built-in PURE demo run via the low-level SubmitRun path."
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
        "projection" => "\
kx projection --instance <hex16> [--at-seq N] [client flags]
  Render a run as a DAG: each Mote's state, nd-class, result ref, and committed seq."
            .into(),
        "runs" => "\
kx runs list [--limit N] [--before-seq N] [client flags]
  Durable run history (Batch B): every registered run, newest-first, from one
  server-side journal fold. --limit caps the page (server max 500); --before-seq
  pages older runs (pass the last page's lowest registered_seq). Read-only."
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
  Print the run's event deltas. StreamEvents is snapshot-to-head today: this catches
  up to the current journal boundary and stops. --follow re-polls from the last cursor
  (~250ms) until Ctrl-C; true live-tail arrives in a later release."
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
}
