//! CLI parsing for the `kx-gateway` binary. Hand-rolled, no clap (mirrors
//! `kx-runtime`): the verb-then-`--flag value` loop keeps the dependency surface
//! minimal and matches the workspace's established CLI style. R3 matures the CLI.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use crate::error::GatewayError;

/// Default worker lease batch size — how many ready Motes the embedded worker
/// pulls per `run_once`. Modest by default; tune with `--max-lease`.
pub const DEFAULT_MAX_LEASE: u32 = 16;

/// Default embedded-worker POOL size — how many concurrent lease→run→propose
/// worker loops the serve spawns. `1` (the default) is byte-identical to
/// the historical single embedded worker. `>1` runs Pure/IO/tool Motes concurrently
/// (model Motes still funnel to the one `ModelCache` owner thread; Ollama swarms get
/// real concurrent inference over independent HTTP). Tune with `--workers` or the
/// `KX_WORKERS` / `KX_SERVE_WORKER_POOL` env. Distinct from `--max-lease` (batch size
/// PER worker, per `run_once`). Resolution + clamp live in the `env_caps` module.
pub const DEFAULT_WORKER_POOL: usize = 1;

/// Default address for the R5 WebSocket `StreamEvents` bridge (the browser live-
/// tail surface). Loopback by default (like the gRPC port); override with
/// `--ws-listen`. A `:0` port asks the OS for an ephemeral one (used by tests).
pub const DEFAULT_WS_LISTEN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 50152);

/// Default fail-closed `PutContent` payload cap (Batch A): 32 MiB. Checked in
/// the handler BEFORE the store is touched; override with `--content-max-bytes`.
/// The transport decode limit is sized from this (cap + headroom), so an
/// oversized upload is refused, never transport-mangled.
pub const DEFAULT_CONTENT_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Default address for the embedded web console (D139) — the third loopback
/// listener serving the compile-time-embedded SPA when the binary carries the
/// `console` feature. Override with `--console-listen`; opt out with
/// `--no-console`. Loopback ONLY (a non-loopback console bind is refused: the
/// CORS self-grant derives from this origin, and remote browsers have the
/// supported static-host + explicit `--cors-origin` path instead).
pub const DEFAULT_CONSOLE_LISTEN: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8888);

/// How the embedded web console should run (D139). Parsed feature-free so the
/// SAME flags are accepted by every build: on a console-less binary `Default`
/// and `Disabled` are no-ops while an explicit `--console-listen` is a loud
/// config error (the user asked for a surface this binary cannot serve).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    /// No flag: serve the console at [`DEFAULT_CONSOLE_LISTEN`] when the
    /// binary has the `console` feature; otherwise do nothing.
    Default,
    /// `--no-console`: never serve the console.
    Disabled,
    /// `--console-listen <addr:port>`: serve at this (loopback) address.
    Listen(SocketAddr),
}

impl ConsoleMode {
    /// The address the console should bind, or `None` when disabled.
    #[must_use]
    pub fn resolve(self) -> Option<SocketAddr> {
        match self {
            ConsoleMode::Default => Some(DEFAULT_CONSOLE_LISTEN),
            ConsoleMode::Disabled => None,
            ConsoleMode::Listen(addr) => Some(addr),
        }
    }
}

/// Resolved `serve` configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The address:port the gateway gRPC service binds (e.g. `127.0.0.1:50151`).
    /// A `0` port asks the OS for an ephemeral one (used by tests).
    pub listen: SocketAddr,
    /// The address:port the R5 WebSocket `StreamEvents` bridge binds (the browser
    /// live-tail surface; default [`DEFAULT_WS_LISTEN`]). A `0` port is ephemeral.
    /// Loopback-only under `--dev-allow-local` (same Rule-8c check as `listen`).
    pub ws_listen: SocketAddr,
    /// On-disk SQLite journal path. The embedded coordinator opens this
    /// read-write (sole writer); the gateway opens a SECOND read-only handle.
    pub journal_path: PathBuf,
    /// Directory backing the shared local-FS content store.
    pub content_root: PathBuf,
    /// Worker lease batch size (see [`DEFAULT_MAX_LEASE`]).
    pub max_lease: u32,
    /// Embedded-worker POOL size from `--workers`. `None` ⇒ not set on the
    /// CLI; the serve resolves it via `KX_WORKERS` / `KX_SERVE_WORKER_POOL` else the
    /// [`DEFAULT_WORKER_POOL`] (see `env_caps::resolve_worker_pool`). Kept as the raw
    /// flag (not pre-resolved) so `parse_serve` stays pure/hermetic — env is read once
    /// at spawn, matching the `KX_SERVE_*` point-of-use convention.
    pub worker_pool: Option<usize>,
    /// Install the dev `local-allow` auth resolver instead of deny-all. Refuses a
    /// non-loopback `listen` (loopback-only dev access). Mutually exclusive with
    /// `auth_tokens`.
    pub dev_allow_local: bool,
    /// Bearer tokens the gateway accepts, as `token → party handle`. Empty ⇒ no
    /// token resolver (deny-all unless `dev_allow_local`). Parsed from
    /// `--auth-token <token>=<party>` (repeatable) and `--auth-token-file <path>`.
    pub auth_tokens: HashMap<String, String>,
    /// Directory for the durable catalog SQLite files (the signature registry +,
    /// in R2b, the recipe ledgers). `None` ⇒ alongside the journal.
    pub catalog_dir: Option<PathBuf>,
    /// In-binary TLS for the gRPC listener (A1). `Some` ⇒ serve TLS (rustls) from
    /// the given PEM cert + key; `None` ⇒ plaintext (the default). `--tls-cert` and
    /// `--tls-key` are given together or not at all.
    pub tls: Option<TlsPaths>,
    /// Browser cross-origin allowlist for the gRPC-web shim (R9.5). Each entry is
    /// an explicit origin (`scheme://host[:port]`), parsed from `--cors-origin`
    /// (repeatable). **Empty ⇒ deny-by-default**: no CORS layer is installed, so a
    /// browser gets no cross-origin grant (native/`curl` clients are unaffected —
    /// CORS is a browser same-origin-policy mechanism). A wildcard (`*`) is refused
    /// at parse time — the allowlist is always explicit (SN-8 fail-closed posture).
    pub cors_origins: Vec<String>,
    /// The embedded web console (D139): default / `--no-console` /
    /// `--console-listen <addr:port>`. Loopback-only; served only by a
    /// `console`-feature binary (the gRPC-web allowlist then auto-extends with
    /// the console's OWN bound loopback origins, never anything wider).
    pub console_listen: ConsoleMode,
    /// The fail-closed `PutContent` payload cap in bytes (Batch A). Default
    /// [`DEFAULT_CONTENT_MAX_BYTES`]; `--content-max-bytes <BYTES>` overrides.
    pub content_max_bytes: u64,
    /// W1a (T-OBS2): the address:port the Prometheus `/metrics` endpoint binds, or
    /// `None` (the default) to NOT serve metrics. Opt-in via `--metrics-listen
    /// <addr:port>` (deny-by-default posture). The endpoint is unauthenticated (the
    /// scraper convention, like `grpc.health.v1`) — bind loopback or a trusted
    /// network; a non-loopback bind is allowed but warns at startup (Cloud adds the
    /// auth/party-scope). A `0` port is ephemeral (used by tests).
    pub metrics_listen: Option<SocketAddr>,
    /// D113 (trigger seam): the LOCAL webhook ingress listener address, or `None` (the
    /// default) to NOT serve webhooks. Opt-in via `--webhook-listen <addr:port>`
    /// (deny-by-default). The untrusted-INBOUND surface: every registered webhook
    /// trigger is per-trigger authenticated (HMAC/bearer; `NONE` only on a loopback
    /// bind), payload-capped, idempotency-deduped, and rate-limited. A non-loopback bind
    /// is allowed but warns at startup. A `0` port is ephemeral (used by tests).
    pub webhook_listen: Option<SocketAddr>,
    /// W1a (T-OBS1): a JSONL operator audit log path for the long-running serve, or
    /// `None` (the default) to NOT write one. Opt-in via `--audit-log <path>`; opened
    /// in APPEND mode so the trail accumulates across restarts. Off the truth path
    /// (best-effort, never gates a run); the operator owns retention/rotation.
    pub audit_log: Option<PathBuf>,
}

/// PEM paths for the gRPC listener's server TLS (A1). The embedded loopback
/// coordinator + worker stay plaintext (internal); only the external listener is
/// encrypted. The WebSocket bridge stays plaintext for now (wss is a fast-follow —
/// front it with the same TLS proxy, or upgrade in a focused PR).
#[derive(Debug, Clone)]
pub struct TlsPaths {
    /// PEM-encoded server certificate chain (leaf first).
    pub cert_path: PathBuf,
    /// PEM-encoded private key for the leaf certificate.
    pub key_path: PathBuf,
}

/// A parsed invocation: print help / version, or serve with a config.
// `Serve` legitimately carries the full `GatewayConfig`; `Help`/`Version` are rare
// one-shots. Boxing the common variant to shave a few bytes off a parse-once enum
// buys an allocation for no benefit — allow the size difference.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Cli {
    /// Print usage and exit 0.
    Help,
    /// Print the version and exit 0.
    Version,
    /// Run the server with the resolved config.
    Serve(GatewayConfig),
}

/// One-line usage string (printed on `--help` and on a parse error).
pub const USAGE: &str =
    "usage: kx-gateway serve --listen <addr:port> --journal <path> --content <dir> \
[--ws-listen <addr:port>] [--console-listen <addr:port> | --no-console] \
[--max-lease <N>] [--workers <N>] [--dev-allow-local | --allow-local-dev] \
[--auth-token <token>=<party>]... [--auth-token-file <path>] [--catalog-dir <dir>] \
[--tls-cert <path> --tls-key <path>] [--cors-origin <scheme://host[:port]>]... \
[--content-max-bytes <BYTES>] [--metrics-listen <addr:port>] \
[--webhook-listen <addr:port>] [--audit-log <path>]\n       \
kx-gateway --help | --version";

impl Cli {
    /// Parse `argv` (excluding the program name).
    ///
    /// Grammar: `serve --listen <addr:port> --journal <path> --content <dir>
    /// [--max-lease <N>] [--dev-allow-local]`, or `--help`/`-h`, or
    /// `--version`/`-V`. An empty argv is treated as `--help`.
    pub fn from_args<I, S>(args: I) -> Result<Self, GatewayError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = args.into_iter().map(Into::into);
        match args.next().as_deref() {
            None | Some("--help" | "-h") => Ok(Cli::Help),
            Some("--version" | "-V") => Ok(Cli::Version),
            Some("serve") => Ok(Cli::Serve(parse_serve(args)?)),
            Some(other) => Err(GatewayError::Config(format!(
                "unknown command {other:?} (expected: serve | --help | --version)"
            ))),
        }
    }
}

// A flat `--flag value` parsing loop: one arm per flag keeps the grammar in one
// readable place; splitting it into sub-parsers would scatter the contract.
#[allow(clippy::too_many_lines)]
fn parse_serve(mut args: impl Iterator<Item = String>) -> Result<GatewayConfig, GatewayError> {
    let mut listen: Option<SocketAddr> = None;
    let mut ws_listen: SocketAddr = DEFAULT_WS_LISTEN;
    let mut journal_path: Option<PathBuf> = None;
    let mut content_root: Option<PathBuf> = None;
    let mut max_lease: u32 = DEFAULT_MAX_LEASE;
    let mut worker_pool: Option<usize> = None;
    let mut dev_allow_local = false;
    let mut auth_tokens: HashMap<String, String> = HashMap::new();
    let mut catalog_dir: Option<PathBuf> = None;
    let mut tls_cert: Option<PathBuf> = None;
    let mut tls_key: Option<PathBuf> = None;
    let mut cors_origins: Vec<String> = Vec::new();
    let mut console_listen = ConsoleMode::Default;
    let mut console_flag_seen = false;
    let mut content_max_bytes: u64 = DEFAULT_CONTENT_MAX_BYTES;
    let mut metrics_listen: Option<SocketAddr> = None;
    let mut webhook_listen: Option<SocketAddr> = None;
    let mut audit_log: Option<PathBuf> = None;

    while let Some(flag) = args.next() {
        let mut take_value = |name: &str| -> Result<String, GatewayError> {
            args.next()
                .ok_or_else(|| GatewayError::Config(format!("{name} requires a value")))
        };
        match flag.as_str() {
            "--listen" => listen = Some(parse_addr("--listen", &take_value("--listen")?)?),
            "--ws-listen" => ws_listen = parse_addr("--ws-listen", &take_value("--ws-listen")?)?,
            "--journal" => journal_path = Some(PathBuf::from(take_value("--journal")?)),
            "--content" => content_root = Some(PathBuf::from(take_value("--content")?)),
            "--max-lease" => {
                let v = take_value("--max-lease")?;
                max_lease = v.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| {
                    GatewayError::Config(format!(
                        "--max-lease expects a positive integer, got {v:?}"
                    ))
                })?;
            }
            // Bounded embedded-worker pool size (default 1 = single worker,
            // byte-identical). A positive integer; the env fallback + clamp live in
            // `env_caps::resolve_worker_pool` (read once at spawn).
            "--workers" => {
                let v = take_value("--workers")?;
                let n = v.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(|| {
                    GatewayError::Config(format!("--workers expects a positive integer, got {v:?}"))
                })?;
                worker_pool = Some(n);
            }
            // `--allow-local-dev` is an accepted alias (same loopback dev
            // posture); a single parse site keeps the two spellings in sync.
            "--dev-allow-local" | "--allow-local-dev" => dev_allow_local = true,
            "--auth-token" => {
                let v = take_value("--auth-token")?;
                let (token, party) = split_token_party(&v).ok_or_else(|| {
                    GatewayError::Config(format!("--auth-token expects <token>=<party>, got {v:?}"))
                })?;
                auth_tokens.insert(token, party);
            }
            "--auth-token-file" => {
                let path = take_value("--auth-token-file")?;
                let body = std::fs::read_to_string(&path)
                    .map_err(|e| GatewayError::Config(format!("--auth-token-file {path}: {e}")))?;
                parse_token_file(&body, &mut auth_tokens)?;
            }
            "--catalog-dir" => catalog_dir = Some(PathBuf::from(take_value("--catalog-dir")?)),
            "--tls-cert" => tls_cert = Some(PathBuf::from(take_value("--tls-cert")?)),
            "--tls-key" => tls_key = Some(PathBuf::from(take_value("--tls-key")?)),
            "--cors-origin" => {
                let v = take_value("--cors-origin")?;
                cors_origins.push(validate_cors_origin(&v)?);
            }
            "--console-listen" => {
                let v = take_value("--console-listen")?;
                console_listen = apply_console_flag(&mut console_flag_seen, Some(v.as_str()))?;
            }
            "--no-console" => console_listen = apply_console_flag(&mut console_flag_seen, None)?,
            "--content-max-bytes" => {
                let v = take_value("--content-max-bytes")?;
                content_max_bytes = v.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| {
                    GatewayError::Config(format!(
                        "--content-max-bytes expects a positive byte count, got {v:?}"
                    ))
                })?;
            }
            // W1a (T-OBS2): opt-in Prometheus metrics endpoint (default OFF).
            "--metrics-listen" => {
                metrics_listen = Some(parse_addr(
                    "--metrics-listen",
                    &take_value("--metrics-listen")?,
                )?);
            }
            // D113: opt-in local webhook ingress listener (default OFF).
            "--webhook-listen" => {
                webhook_listen = Some(parse_addr(
                    "--webhook-listen",
                    &take_value("--webhook-listen")?,
                )?);
            }
            // W1a (T-OBS1): opt-in serve-path JSONL audit log (default OFF).
            "--audit-log" => audit_log = Some(PathBuf::from(take_value("--audit-log")?)),
            other => return Err(GatewayError::Config(format!("unknown flag {other:?}"))),
        }
    }

    let listen = listen.ok_or_else(|| GatewayError::Config("--listen is required".into()))?;
    let journal_path =
        journal_path.ok_or_else(|| GatewayError::Config("--journal is required".into()))?;
    let content_root =
        content_root.ok_or_else(|| GatewayError::Config("--content is required".into()))?;

    // Exactly one auth posture: dev-allow-local and configured tokens are
    // mutually exclusive (no ambiguity about which resolver wins).
    if dev_allow_local && !auth_tokens.is_empty() {
        return Err(GatewayError::Config(
            "--dev-allow-local and --auth-token/--auth-token-file are mutually exclusive".into(),
        ));
    }

    let tls = pair_tls(tls_cert, tls_key)?;

    refuse_console_without_feature(console_listen)?;

    Ok(GatewayConfig {
        listen,
        ws_listen,
        journal_path,
        content_root,
        max_lease,
        worker_pool,
        dev_allow_local,
        auth_tokens,
        catalog_dir,
        tls,
        cors_origins,
        console_listen,
        content_max_bytes,
        metrics_listen,
        webhook_listen,
        audit_log,
    })
}

/// Parse an `addr:port` flag value (IP literal), naming the flag on failure.
fn parse_addr(name: &str, v: &str) -> Result<SocketAddr, GatewayError> {
    v.parse::<SocketAddr>().map_err(|_| {
        GatewayError::Config(format!(
            "{name} expects an addr:port (IP literal), got {v:?}"
        ))
    })
}

/// Pair the TLS cert + key: both given (TLS) or neither (plaintext). A
/// half-configured TLS would silently fall back to plaintext — fail closed instead.
fn pair_tls(
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
) -> Result<Option<TlsPaths>, GatewayError> {
    match (tls_cert, tls_key) {
        (Some(cert_path), Some(key_path)) => Ok(Some(TlsPaths {
            cert_path,
            key_path,
        })),
        (None, None) => Ok(None),
        _ => Err(GatewayError::Config(
            "--tls-cert and --tls-key must be given together".into(),
        )),
    }
}

/// Parse ONE console flag occurrence (D139): `Some(addr)` for
/// `--console-listen <addr:port>`, `None` for `--no-console`. The two flags
/// are mutually exclusive, and a `--console-listen` address must be loopback —
/// its origin is what the gRPC-web CORS allowlist auto-grants, and a public
/// bind cannot soundly self-derive a grantable public origin (remote browsers
/// use the static-host + explicit `--cors-origin` path instead).
fn apply_console_flag(seen: &mut bool, value: Option<&str>) -> Result<ConsoleMode, GatewayError> {
    if *seen {
        return Err(GatewayError::Config(
            "--console-listen and --no-console are mutually exclusive".into(),
        ));
    }
    *seen = true;
    let Some(v) = value else {
        return Ok(ConsoleMode::Disabled);
    };
    let addr = v.parse::<SocketAddr>().map_err(|_| {
        GatewayError::Config(format!(
            "--console-listen expects an addr:port (IP literal), got {v:?}"
        ))
    })?;
    if !addr.ip().is_loopback() {
        return Err(GatewayError::Config(format!(
            "--console-listen must bind a loopback address (got {addr}); for \
             remote browsers, static-host the SPA and grant its origin via \
             --cors-origin"
        )));
    }
    Ok(ConsoleMode::Listen(addr))
}

/// D139: an EXPLICIT console request on a binary that cannot serve one is a
/// loud error (never a silent no-op); the default/`--no-console` modes parse
/// everywhere so shared fixtures can pass `--no-console` unconditionally.
fn refuse_console_without_feature(mode: ConsoleMode) -> Result<(), GatewayError> {
    if !cfg!(feature = "console") {
        if let ConsoleMode::Listen(_) = mode {
            return Err(GatewayError::Config(
                "this kx was built without the web console (`console` feature); \
                 use the prebuilt release binary, or build from a repo checkout with \
                 `--features console` after `just console-dist`"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// Validate a `--cors-origin` value fail-closed: it must be a concrete origin
/// (`scheme://host[:port]`), never a wildcard. The allowlist is always explicit so
/// a browser can never be granted blanket cross-origin access (the gRPC-web shim's
/// security posture mirrors the deny-all auth default). Returns the trimmed origin.
///
/// We reject `*` and `null` (the two blanket/opaque grants) and require a
/// `scheme://` prefix; the exact host is matched verbatim at request time by the
/// CORS layer, so we keep parsing minimal (no scheme/host allowlist beyond the
/// shape) — a typo yields a benign no-match (the browser is simply denied), never
/// an over-broad grant.
fn validate_cors_origin(value: &str) -> Result<String, GatewayError> {
    let v = value.trim();
    if v.is_empty() {
        return Err(GatewayError::Config(
            "--cors-origin requires a non-empty origin".into(),
        ));
    }
    if v == "*" || v.eq_ignore_ascii_case("null") {
        return Err(GatewayError::Config(format!(
            "--cors-origin must be an explicit origin, not a wildcard, got {v:?} \
             (the allowlist is deny-by-default — list each browser origin explicitly)"
        )));
    }
    // Require a scheme://host shape so an accidental bare host is caught early
    // rather than silently never matching.
    let Some((scheme, rest)) = v.split_once("://") else {
        return Err(GatewayError::Config(format!(
            "--cors-origin expects <scheme://host[:port]>, got {v:?}"
        )));
    };
    if scheme.is_empty() || rest.is_empty() {
        return Err(GatewayError::Config(format!(
            "--cors-origin expects <scheme://host[:port]>, got {v:?}"
        )));
    }
    Ok(v.to_string())
}

/// Split a `token=party` spec on the LAST `=` (so a base64 token with `=`
/// padding survives — the party handle never contains `=`). Both sides must be
/// non-empty.
fn split_token_party(spec: &str) -> Option<(String, String)> {
    let (token, party) = spec.rsplit_once('=')?;
    if token.is_empty() || party.is_empty() {
        return None;
    }
    Some((token.to_string(), party.to_string()))
}

/// Parse a token file: one `token=party` per line, skipping blank lines and
/// `#` comments. A non-conforming line is a hard error (fail-closed config).
fn parse_token_file(body: &str, tokens: &mut HashMap<String, String>) -> Result<(), GatewayError> {
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (token, party) = split_token_party(line).ok_or_else(|| {
            GatewayError::Config(format!(
                "--auth-token-file line is not <token>=<party>: {line:?}"
            ))
        })?;
        tokens.insert(token, party);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serve(cli: Cli) -> GatewayConfig {
        match cli {
            Cli::Serve(c) => c,
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    #[test]
    fn parses_serve_with_all_flags() {
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:50151",
                "--journal",
                "/tmp/kx.db",
                "--content",
                "/tmp/blobs",
                "--max-lease",
                "8",
                "--dev-allow-local",
            ])
            .unwrap(),
        );
        assert_eq!(c.listen, "127.0.0.1:50151".parse::<SocketAddr>().unwrap());
        assert_eq!(c.journal_path, PathBuf::from("/tmp/kx.db"));
        assert_eq!(c.content_root, PathBuf::from("/tmp/blobs"));
        assert_eq!(c.max_lease, 8);
        assert!(c.dev_allow_local);
    }

    #[test]
    fn workers_flag_parses_and_defaults_to_none() {
        // `--workers N` records the raw flag (env/default resolution happens at
        // spawn in `env_caps::resolve_worker_pool`, so `parse_serve` stays pure).
        let with = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--workers",
                "4",
            ])
            .unwrap(),
        );
        assert_eq!(with.worker_pool, Some(4), "the --workers flag is captured");
        // Omitted ⇒ None (resolves to KX_WORKERS / KX_SERVE_WORKER_POOL / default 1 at spawn).
        let without = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ])
            .unwrap(),
        );
        assert_eq!(
            without.worker_pool, None,
            "omitted --workers stays None (byte-identical)"
        );
        // A non-positive value is a loud config error (never a silent 0-worker serve).
        assert!(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--workers",
                "0",
            ])
            .is_err(),
            "--workers 0 is refused"
        );
    }

    #[test]
    fn allow_local_dev_is_an_accepted_alias() {
        // `--allow-local-dev` resolves to the SAME loopback dev posture as the
        // canonical `--dev-allow-local` (the zero-config `kx serve` ergonomics
        // accept either spelling).
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--allow-local-dev",
            ])
            .unwrap(),
        );
        assert!(c.dev_allow_local, "the alias enables the dev posture");
    }

    #[test]
    fn ws_listen_parses_and_defaults() {
        // Default when omitted.
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ])
            .unwrap(),
        );
        assert_eq!(c.ws_listen, DEFAULT_WS_LISTEN);

        // Explicit override.
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--ws-listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ])
            .unwrap(),
        );
        assert_eq!(c.ws_listen, "127.0.0.1:0".parse::<SocketAddr>().unwrap());

        // Bad value is a config error.
        assert!(Cli::from_args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--ws-listen",
            "not-an-addr",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
        ])
        .is_err());
    }

    #[test]
    fn max_lease_defaults_and_dev_allow_defaults_off() {
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ])
            .unwrap(),
        );
        assert_eq!(c.max_lease, DEFAULT_MAX_LEASE);
        assert!(!c.dev_allow_local, "deny-all is the default posture");
    }

    #[test]
    fn help_and_version_are_recognized() {
        assert!(matches!(Cli::from_args(["--help"]).unwrap(), Cli::Help));
        assert!(matches!(Cli::from_args(["-h"]).unwrap(), Cli::Help));
        assert!(matches!(
            Cli::from_args(Vec::<String>::new()).unwrap(),
            Cli::Help
        ));
        assert!(matches!(
            Cli::from_args(["--version"]).unwrap(),
            Cli::Version
        ));
        assert!(matches!(Cli::from_args(["-V"]).unwrap(), Cli::Version));
    }

    #[test]
    fn parses_auth_tokens_and_catalog_dir() {
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--auth-token",
                "tok-a=alice@acme",
                "--auth-token",
                "tok-b=bob@acme",
                "--catalog-dir",
                "/tmp/cat",
            ])
            .unwrap(),
        );
        assert_eq!(
            c.auth_tokens.get("tok-a").map(String::as_str),
            Some("alice@acme")
        );
        assert_eq!(
            c.auth_tokens.get("tok-b").map(String::as_str),
            Some("bob@acme")
        );
        assert_eq!(c.catalog_dir, Some(PathBuf::from("/tmp/cat")));
        assert!(!c.dev_allow_local);
    }

    #[test]
    fn content_max_bytes_defaults_overrides_and_rejects_garbage() {
        let base = |extra: &[&str]| {
            let mut a = vec![
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ];
            a.extend_from_slice(extra);
            Cli::from_args(a)
        };
        // Default.
        let c = serve(base(&[]).unwrap());
        assert_eq!(c.content_max_bytes, DEFAULT_CONTENT_MAX_BYTES);
        // Override.
        let c = serve(base(&["--content-max-bytes", "1048576"]).unwrap());
        assert_eq!(c.content_max_bytes, 1_048_576);
        // Zero and garbage are refused (a 0-byte cap would refuse every upload
        // silently — fail loudly at parse instead).
        assert!(base(&["--content-max-bytes", "0"]).is_err());
        assert!(base(&["--content-max-bytes", "lots"]).is_err());
        assert!(base(&["--content-max-bytes"]).is_err());
    }

    #[test]
    fn metrics_listen_and_audit_log_default_off_and_parse() {
        let base = |extra: &[&str]| {
            let mut a = vec![
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ];
            a.extend_from_slice(extra);
            Cli::from_args(a)
        };
        // Default: both OFF (deny-by-default observability posture).
        let c = serve(base(&[]).unwrap());
        assert_eq!(c.metrics_listen, None);
        assert_eq!(c.audit_log, None);
        // Opt-in: both parse.
        let c = serve(
            base(&[
                "--metrics-listen",
                "127.0.0.1:9090",
                "--audit-log",
                "/tmp/audit.jsonl",
            ])
            .unwrap(),
        );
        assert_eq!(
            c.metrics_listen,
            Some("127.0.0.1:9090".parse::<SocketAddr>().unwrap())
        );
        assert_eq!(c.audit_log, Some(PathBuf::from("/tmp/audit.jsonl")));
        // A malformed metrics addr is a config error; a missing value too.
        assert!(base(&["--metrics-listen", "not-an-addr"]).is_err());
        assert!(base(&["--metrics-listen"]).is_err());
        assert!(base(&["--audit-log"]).is_err());
    }

    #[test]
    fn tls_cert_and_key_must_be_given_together() {
        let base = |extra: &[&str]| {
            let mut a = vec![
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ];
            a.extend_from_slice(extra);
            Cli::from_args(a)
        };
        // Both → Some(TlsPaths).
        let c = serve(base(&["--tls-cert", "/tmp/cert.pem", "--tls-key", "/tmp/key.pem"]).unwrap());
        let tls = c.tls.expect("tls configured");
        assert_eq!(tls.cert_path, PathBuf::from("/tmp/cert.pem"));
        assert_eq!(tls.key_path, PathBuf::from("/tmp/key.pem"));
        // Cert-without-key and key-without-cert are both errors (fail closed).
        assert!(base(&["--tls-cert", "/tmp/cert.pem"]).is_err());
        assert!(base(&["--tls-key", "/tmp/key.pem"]).is_err());
        // Neither → None (plaintext default).
        assert!(serve(base(&[]).unwrap()).tls.is_none());
    }

    #[test]
    fn cors_origin_parses_repeatable_and_defaults_empty() {
        // Default: no --cors-origin ⇒ empty ⇒ deny-by-default (no CORS layer).
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ])
            .unwrap(),
        );
        assert!(
            c.cors_origins.is_empty(),
            "deny-by-default: no browser origin is granted unless listed"
        );

        // Repeatable: each --cors-origin appends, in order.
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--cors-origin",
                "http://localhost:5173",
                "--cors-origin",
                "https://app.example.com",
            ])
            .unwrap(),
        );
        assert_eq!(
            c.cors_origins,
            vec![
                "http://localhost:5173".to_string(),
                "https://app.example.com".to_string()
            ]
        );
    }

    #[test]
    fn cors_origin_rejects_wildcard_and_malformed() {
        let base = |origin: &str| {
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--cors-origin",
                origin,
            ])
        };
        // A wildcard / opaque grant is refused fail-closed (no blanket access).
        assert!(base("*").is_err(), "wildcard origin must be refused");
        assert!(
            base("null").is_err(),
            "opaque 'null' origin must be refused"
        );
        assert!(base("NULL").is_err(), "case-insensitive 'null' refused");
        // A bare host (no scheme) is caught early rather than silently never matching.
        assert!(base("app.example.com").is_err());
        assert!(base("https://").is_err());
        assert!(base("").is_err());
        // A concrete origin is accepted.
        assert!(base("https://app.example.com").is_ok());
    }

    #[test]
    fn console_defaults_and_no_console_parse_everywhere() {
        // No flag → Default (the console serves iff the binary has the feature).
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ])
            .unwrap(),
        );
        assert_eq!(c.console_listen, ConsoleMode::Default);
        assert_eq!(
            ConsoleMode::Default.resolve(),
            Some(DEFAULT_CONSOLE_LISTEN),
            "default mode binds the well-known console port"
        );

        // --no-console → Disabled, on EVERY build (fixtures pass it blindly).
        let c = serve(
            Cli::from_args([
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
                "--no-console",
            ])
            .unwrap(),
        );
        assert_eq!(c.console_listen, ConsoleMode::Disabled);
        assert_eq!(ConsoleMode::Disabled.resolve(), None);
    }

    #[cfg(feature = "console")]
    #[test]
    fn console_listen_parses_loopback_and_refuses_public_and_conflicts() {
        let base = |extra: &[&str]| {
            let mut a = vec![
                "serve",
                "--listen",
                "127.0.0.1:0",
                "--journal",
                "/tmp/j",
                "--content",
                "/tmp/c",
            ];
            a.extend_from_slice(extra);
            Cli::from_args(a)
        };
        let c = serve(base(&["--console-listen", "127.0.0.1:0"]).unwrap());
        assert_eq!(
            c.console_listen,
            ConsoleMode::Listen("127.0.0.1:0".parse().unwrap())
        );
        // D139.3: non-loopback console binds are refused outright.
        assert!(base(&["--console-listen", "0.0.0.0:8888"]).is_err());
        assert!(base(&["--console-listen", "192.168.1.10:8888"]).is_err());
        // The two console flags are mutually exclusive, in either order.
        assert!(base(&["--no-console", "--console-listen", "127.0.0.1:0"]).is_err());
        assert!(base(&["--console-listen", "127.0.0.1:0", "--no-console"]).is_err());
        // Malformed addr.
        assert!(base(&["--console-listen", "not-an-addr"]).is_err());
    }

    #[cfg(not(feature = "console"))]
    #[test]
    fn explicit_console_listen_is_a_loud_error_without_the_feature() {
        let err = Cli::from_args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--console-listen",
            "127.0.0.1:8888",
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("without the web console"),
            "the error names the remedy: {err}"
        );
    }

    #[test]
    fn auth_token_and_dev_allow_local_are_mutually_exclusive() {
        assert!(Cli::from_args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--dev-allow-local",
            "--auth-token",
            "tok=alice",
        ])
        .is_err());
    }

    #[test]
    fn split_token_party_keeps_base64_padding_in_token() {
        // The separator is the LAST '=', so a token with '=' padding survives.
        assert_eq!(
            split_token_party("YWJj==alice@acme"),
            Some(("YWJj=".to_string(), "alice@acme".to_string()))
        );
        assert_eq!(split_token_party("noequals"), None);
        assert_eq!(split_token_party("=party"), None);
        assert_eq!(split_token_party("token="), None);
    }

    #[test]
    fn token_file_parses_lines_and_skips_comments() {
        let mut tokens = HashMap::new();
        let body = "# a comment\n\n  tok-a=alice@acme  \ntok-b=bob@acme\n# trailing\n";
        parse_token_file(body, &mut tokens).unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens.get("tok-a").map(String::as_str), Some("alice@acme"));
        assert_eq!(tokens.get("tok-b").map(String::as_str), Some("bob@acme"));
        // A non-conforming line is a hard error.
        let mut bad = HashMap::new();
        assert!(parse_token_file("not-a-pair\n", &mut bad).is_err());
    }

    #[test]
    fn rejects_missing_required_and_unknown() {
        // Missing --listen / --journal / --content.
        assert!(Cli::from_args(["serve", "--journal", "/tmp/j", "--content", "/tmp/c"]).is_err());
        assert!(Cli::from_args(["serve", "--listen", "127.0.0.1:0"]).is_err());
        // Unknown verb + unknown flag + bad listen + bad max-lease.
        assert!(Cli::from_args(["frobnicate"]).is_err());
        assert!(Cli::from_args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--nope"
        ])
        .is_err());
        assert!(Cli::from_args([
            "serve",
            "--listen",
            "not-an-addr",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c"
        ])
        .is_err());
        assert!(Cli::from_args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--journal",
            "/tmp/j",
            "--content",
            "/tmp/c",
            "--max-lease",
            "0"
        ])
        .is_err());
    }
}
