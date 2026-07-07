//! `kx-profile` — capture an environment-labelled runtime profile (Golden Rule
//! 10).
//!
//! Hosts a fresh in-process gateway per iteration, measures warm-up
//! (`start` → health `SERVING`) and submit→Committed latency over the FFI-free
//! echo demo, and writes a schema-1 JSON [`Report`] to `target/profile/`
//! (gitignored). `just profile` runs this + re-runs the existing scale/ceiling
//! spikes; the captured JSON is then copied into the PRIVATE
//! `docs/benchmarks/` trend record (never committed to OSS — SN-2).
//!
//! Usage:
//! - In-process spikes (default): `kx-profile [--iterations N] [--out PATH]` (`N = 8`).
//! - Attach mode (GR24 dual-engine baseline): `kx-profile --serve <addr> <chat|embed>
//!   [--iterations N] [--prompt "..."] [--model <id>] [--token <t> | --token-file <p>]`
//!   — `chat` times a real chat; `embed` times a datasets server-embed ingest+query —
//!   against an EXTERNAL `kx serve` (whichever engine it runs).

use std::path::PathBuf;

use kx_profile::{
    capture_git_sha, chat_spikes, content_spikes, embed_spikes, mote_detail_spikes, percentile,
    react_spikes, spikes, vision_spikes, ChatOpts, EmbedOpts, Environment, Metric, ProfileError,
    Report, VisionOpts,
};

#[tokio::main]
async fn main() {
    // Surface the gateway-under-measurement's tracing (best-effort; ignore an
    // already-installed global subscriber).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    if let Err(e) = run().await {
        eprintln!("kx-profile: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ProfileError> {
    let args = Args::parse(std::env::args().skip(1));

    // Capture the load-bearing labels FIRST — abort before any work if the
    // environment can't be fully described (Golden Rule 10).
    let git_sha = capture_git_sha()?;
    let env = Environment::capture()?;
    eprintln!(
        "kx-profile: profiling {sha} on {host} ({cores} cores, {os}/{arch}) — {n} iteration(s)",
        sha = short_sha(&git_sha),
        host = env.host,
        cores = env.cores,
        os = env.os,
        arch = env.arch,
        n = args.iterations,
    );

    // Attach mode (`--serve <addr>`): profile a real chat against an EXTERNAL
    // `kx serve` (whichever engine it runs — Ollama or llama.cpp), the GR10/GR24
    // dual-engine baseline. Otherwise run the in-process FFI-free spikes.
    let metrics = if let Some(endpoint) = args.serve.clone() {
        match args.mode {
            AttachMode::Chat => attach_chat_metrics(&endpoint, &args).await?,
            AttachMode::Embed => attach_embed_metrics(&endpoint, &args).await?,
            AttachMode::Vision => attach_vision_metrics(&endpoint, &args).await?,
        }
    } else {
        inproc_metrics(args.iterations).await?
    };

    let report = Report::new(git_sha.clone(), env, metrics);
    let json = report
        .to_json()
        .map_err(|e| ProfileError::Report(e.to_string()))?;

    let out = args.out.unwrap_or_else(|| default_out(&git_sha));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ProfileError::Report(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&out, format!("{json}\n"))
        .map_err(|e| ProfileError::Report(format!("write {}: {e}", out.display())))?;
    eprintln!("kx-profile: wrote {}", out.display());

    // The JSON also goes to stdout so `just profile` can tee it into the
    // private trend record.
    println!("{json}");
    Ok(())
}

/// The in-process FFI-free spikes (warm-up + submit→Committed, react settle,
/// content path, inspector path). The default profile when `--serve` is absent.
async fn inproc_metrics(iterations: usize) -> Result<Vec<Metric>, ProfileError> {
    let mut metrics = Vec::new();
    // Warm-up + submit→Committed (the latter doubles as the PR-2 admission-
    // persist overhead measurement against the pre-PR-2 baseline).
    let samples = spikes::measure(iterations).await?;
    push_spikes(
        &mut metrics,
        &[
            ("warmup_to_serving_p50", percentile(&samples.warmup_ms, 50)),
            (
                "submit_to_committed_p50",
                percentile(&samples.submit_ms, 50),
            ),
            (
                "submit_to_committed_p99",
                percentile(&samples.submit_ms, 99),
            ),
        ],
    );

    // The embedded worker pool: submit→Committed at pool ∈ {1, 2, 4}. The
    // regression guard is `pool1 == the no-pool baseline` (byte-identical default) with
    // bounded pool>1 per-run overhead. (Throughput-under-real-work is the live swarm
    // witness, not this free-execution stub — see `measure_pool` docs.)
    let pool = spikes::measure_pool(iterations, &[1, 2, 4]).await?;
    push_spikes(
        &mut metrics,
        &[
            ("pool1_submit_to_committed_p50", percentile(&pool[0].1, 50)),
            ("pool2_submit_to_committed_p50", percentile(&pool[1].1, 50)),
            ("pool4_submit_to_committed_p50", percentile(&pool[2].1, 50)),
        ],
    );

    // PR-2d-2 — M7a/M7b: the live react chain's settle machinery, model-free at
    // the coordinator layer (M7b fires the REAL bundled stdio tool; skipped —
    // empty samples — when the bin is absent).
    let react = react_spikes::measure(iterations).await?;
    push_spikes(
        &mut metrics,
        &[
            ("react_answer_settle_p50", percentile(&react.answer_ms, 50)),
            ("react_answer_settle_p99", percentile(&react.answer_ms, 99)),
        ],
    );
    if !react.tool_round_ms.is_empty() {
        push_spikes(
            &mut metrics,
            &[
                ("react_tool_round_p50", percentile(&react.tool_round_ms, 50)),
                ("react_tool_round_p99", percentile(&react.tool_round_ms, 99)),
            ],
        );
    }

    // Batch A — the content path: a 1 MiB client upload (the first client
    // write path) + the full 64-ref × 4 KiB batch read (the N+1 collapse).
    let content = content_spikes::measure(iterations).await?;
    push_spikes(
        &mut metrics,
        &[
            ("put_content_1mib_p50", percentile(&content.put_1mib_ms, 50)),
            ("put_content_1mib_p95", percentile(&content.put_1mib_ms, 95)),
            (
                "content_batch_64x4k_p50",
                percentile(&content.batch_64x4k_ms, 50),
            ),
            (
                "content_batch_64x4k_p95",
                percentile(&content.batch_64x4k_ms, 95),
            ),
        ],
    );

    // Batch B — the inspector path: GetMoteDetail cold (fold + store get +
    // decode) and warm (the host's def cache).
    let detail = mote_detail_spikes::measure(iterations).await?;
    push_spikes(
        &mut metrics,
        &[
            ("mote_detail_cold", percentile(&detail.detail_cold_ms, 50)),
            (
                "mote_detail_warm_p50",
                percentile(&detail.detail_warm_ms, 50),
            ),
            (
                "mote_detail_warm_p95",
                percentile(&detail.detail_warm_ms, 95),
            ),
        ],
    );
    Ok(metrics)
}

/// Profile a real chat against an EXTERNAL `kx serve` at `endpoint` (GR10 + GR24
/// dual-engine baseline). Each metric id is prefixed with the engine that answered
/// (`chat__kx-ollama__…` / `chat__kx-llamacpp__…`) so an Ollama capture and a
/// llama.cpp capture never collide in the private trend record.
async fn attach_chat_metrics(endpoint: &str, args: &Args) -> Result<Vec<Metric>, ProfileError> {
    let channel = chat_spikes::connect(endpoint).await?;
    let token = match (&args.token, &args.token_file) {
        (Some(t), _) => Some(t.clone()),
        (None, Some(path)) => Some(read_token_file(path)?),
        (None, None) => None,
    };
    let opts = ChatOpts {
        iterations: args.iterations,
        prompt: args
            .prompt
            .clone()
            .unwrap_or_else(|| chat_spikes::DEFAULT_PROMPT.to_string()),
        model: args.model.clone(),
        token,
    };
    let chat = chat_spikes::measure(&channel, &opts).await?;
    eprintln!(
        "kx-profile: chat baseline | engine={engine} | model={model} | ctx={ctx} | \
         {n} timed iter(s){ttft}",
        engine = chat.engine,
        model = chat.model_id,
        ctx = chat.context_len,
        n = chat.total_ms.len(),
        ttft = if chat.ttft_ms.is_empty() {
            " | ttft: unavailable (no token stream)"
        } else {
            ""
        },
    );
    let p = |m: &str| format!("chat__{}__{m}", chat.engine);
    let mut metrics = Vec::new();
    push_spikes(
        &mut metrics,
        &[
            (p("warmup_first_ms").as_str(), chat.warmup_first_ms),
            (p("total_p50").as_str(), percentile(&chat.total_ms, 50)),
            (p("total_p95").as_str(), percentile(&chat.total_ms, 95)),
            (p("total_p99").as_str(), percentile(&chat.total_ms, 99)),
        ],
    );
    if !chat.ttft_ms.is_empty() {
        push_spikes(
            &mut metrics,
            &[
                (p("ttft_p50").as_str(), percentile(&chat.ttft_ms, 50)),
                (p("ttft_p99").as_str(), percentile(&chat.ttft_ms, 99)),
            ],
        );
    }
    Ok(metrics)
}

/// Profile a real vision (image→text) turn against an EXTERNAL `kx serve` at `endpoint`
/// (GR10 + GR24 dual-engine baseline). Each metric id is prefixed with the engine that
/// answered (`vision__kx-ollama__…` / `vision__kx-llamacpp__…`) so an Ollama capture and
/// a llama.cpp capture never collide in the private trend record.
async fn attach_vision_metrics(endpoint: &str, args: &Args) -> Result<Vec<Metric>, ProfileError> {
    let channel = chat_spikes::connect(endpoint).await?;
    let token = match (&args.token, &args.token_file) {
        (Some(t), _) => Some(t.clone()),
        (None, Some(path)) => Some(read_token_file(path)?),
        (None, None) => None,
    };
    let opts = VisionOpts {
        iterations: args.iterations,
        prompt: args
            .prompt
            .clone()
            .unwrap_or_else(|| vision_spikes::DEFAULT_PROMPT.to_string()),
        model: args.model.clone(),
        token,
    };
    let vision = vision_spikes::measure(&channel, &opts).await?;
    eprintln!(
        "kx-profile: vision baseline | engine={engine} | model={model} | {n} timed iter(s)",
        engine = vision.engine,
        model = vision.model_id,
        n = vision.total_ms.len(),
    );
    let p = |m: &str| format!("vision__{}__{m}", vision.engine);
    let mut metrics = Vec::new();
    push_spikes(
        &mut metrics,
        &[
            (p("warmup_first_ms").as_str(), vision.warmup_first_ms),
            (p("total_p50").as_str(), percentile(&vision.total_ms, 50)),
            (p("total_p95").as_str(), percentile(&vision.total_ms, 95)),
            (p("total_p99").as_str(), percentile(&vision.total_ms, 99)),
        ],
    );
    Ok(metrics)
}

/// Profile real datasets server-embed (ingest + query) against an EXTERNAL `kx serve`
/// at `endpoint` (GR10 + GR24 dual-engine baseline). Each metric id is prefixed with
/// the engine that embeds (`embed__kx-ollama__…` / `embed__kx-llamacpp__…`) so an
/// Ollama capture and a llama.cpp capture never collide in the trend record.
async fn attach_embed_metrics(endpoint: &str, args: &Args) -> Result<Vec<Metric>, ProfileError> {
    let channel = chat_spikes::connect(endpoint).await?;
    let token = match (&args.token, &args.token_file) {
        (Some(t), _) => Some(t.clone()),
        (None, Some(path)) => Some(read_token_file(path)?),
        (None, None) => None,
    };
    let opts = EmbedOpts {
        iterations: args.iterations,
        text: args
            .prompt
            .clone()
            .unwrap_or_else(|| embed_spikes::DEFAULT_TEXT.to_string()),
        model: args.model.clone(),
        token,
    };
    let embed = embed_spikes::measure(&channel, &opts).await?;
    eprintln!(
        "kx-profile: embed baseline | engine={engine} | model={model} | {n} timed iter(s)",
        engine = embed.engine,
        model = embed.model_id,
        n = embed.ingest_ms.len(),
    );
    let p = |m: &str| format!("embed__{}__{m}", embed.engine);
    let mut metrics = Vec::new();
    push_spikes(
        &mut metrics,
        &[
            (p("ingest_p50").as_str(), percentile(&embed.ingest_ms, 50)),
            (p("ingest_p95").as_str(), percentile(&embed.ingest_ms, 95)),
            (p("query_p50").as_str(), percentile(&embed.query_ms, 50)),
            (p("query_p95").as_str(), percentile(&embed.query_ms, 95)),
        ],
    );
    Ok(metrics)
}

/// Read a bearer token from a file (trimmed of surrounding whitespace).
fn read_token_file(path: &str) -> Result<String, ProfileError> {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| ProfileError::Client(format!("read token file {path}: {e}")))
}

/// Push a batch of millisecond spike metrics (one `Metric::spike` per pair).
fn push_spikes(metrics: &mut Vec<Metric>, spikes: &[(&str, f64)]) {
    for (id, value) in spikes {
        metrics.push(Metric::spike(*id, *value, "ms"));
    }
}

/// Which attach-mode baseline to capture (the subverb after `--serve`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttachMode {
    Chat,
    Embed,
    Vision,
}

/// Parsed CLI arguments (hand-rolled, no clap — the FFI-free `kx` convention).
struct Args {
    iterations: usize,
    out: Option<PathBuf>,
    /// `--serve <addr>`: profile against an EXTERNAL `kx serve` (attach mode). Absent ⇒
    /// run the in-process FFI-free spikes.
    serve: Option<String>,
    /// The attach SUBVERB: `chat` (default) or `embed`.
    mode: AttachMode,
    /// `--prompt <text>`: the chat prompt in attach mode (default in `chat_spikes`).
    prompt: Option<String>,
    /// `--model <id>`: chat a specific served model (default = the primary).
    model: Option<String>,
    /// `--token <bearer>`: auth metadata for a non-dev serve.
    token: Option<String>,
    /// `--token-file <path>`: read the bearer token from a file.
    token_file: Option<String>,
}

impl Args {
    fn parse(mut argv: impl Iterator<Item = String>) -> Self {
        let mut iterations = 8usize;
        let mut out = None;
        let mut serve = None;
        let mut prompt = None;
        let mut model = None;
        let mut token = None;
        let mut token_file = None;
        let mut mode = AttachMode::Chat;
        while let Some(flag) = argv.next() {
            match flag.as_str() {
                "--iterations" | "-n" => {
                    if let Some(v) = argv.next() {
                        if let Ok(n) = v.parse::<usize>() {
                            iterations = n.max(1);
                        } else {
                            eprintln!("kx-profile: invalid --iterations {v:?}; using {iterations}");
                        }
                    }
                }
                "--out" | "-o" => {
                    out = argv.next().map(PathBuf::from);
                }
                "--serve" => {
                    serve = argv.next();
                }
                "--prompt" => {
                    prompt = argv.next();
                }
                "--model" => {
                    model = argv.next();
                }
                "--token" => {
                    token = argv.next();
                }
                "--token-file" => {
                    token_file = argv.next();
                }
                // The attach subverb: selects the baseline (`chat` default | `embed` | `vision`).
                "chat" => mode = AttachMode::Chat,
                "embed" => mode = AttachMode::Embed,
                "vision" => mode = AttachMode::Vision,
                other => {
                    eprintln!("kx-profile: ignoring unrecognized argument {other:?}");
                }
            }
        }
        Self {
            iterations,
            out,
            serve,
            mode,
            prompt,
            model,
            token,
            token_file,
        }
    }
}

/// The first 12 hex of a commit sha (for human log lines / file names).
fn short_sha(sha: &str) -> &str {
    let end = sha.len().min(12);
    &sha[..end]
}

/// Default output path: `target/profile/profile-<sha12>.json` (`target/` is
/// already gitignored).
fn default_out(git_sha: &str) -> PathBuf {
    PathBuf::from("target")
        .join("profile")
        .join(format!("profile-{}.json", short_sha(git_sha)))
}
