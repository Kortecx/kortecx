//! CLI parsing for the `kx-gateway` binary. Hand-rolled, no clap (mirrors
//! `kx-runtime`): the verb-then-`--flag value` loop keeps the dependency surface
//! minimal and matches the workspace's established CLI style. R3 matures the CLI.

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::GatewayError;

/// Default worker lease batch size — how many ready Motes the embedded worker
/// pulls per `run_once`. Modest by default; tune with `--max-lease`.
pub const DEFAULT_MAX_LEASE: u32 = 16;

/// Resolved `serve` configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The address:port the gateway gRPC service binds (e.g. `127.0.0.1:50151`).
    /// A `0` port asks the OS for an ephemeral one (used by tests).
    pub listen: SocketAddr,
    /// On-disk SQLite journal path. The embedded coordinator opens this
    /// read-write (sole writer); the gateway opens a SECOND read-only handle.
    pub journal_path: PathBuf,
    /// Directory backing the shared local-FS content store.
    pub content_root: PathBuf,
    /// Worker lease batch size (see [`DEFAULT_MAX_LEASE`]).
    pub max_lease: u32,
    /// Install the dev `local-allow` auth resolver instead of deny-all. Refuses a
    /// non-loopback `listen` (loopback-only dev access).
    pub dev_allow_local: bool,
}

/// A parsed invocation: print help / version, or serve with a config.
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
[--max-lease <N>] [--dev-allow-local]\n       kx-gateway --help | --version";

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

fn parse_serve(mut args: impl Iterator<Item = String>) -> Result<GatewayConfig, GatewayError> {
    let mut listen: Option<SocketAddr> = None;
    let mut journal_path: Option<PathBuf> = None;
    let mut content_root: Option<PathBuf> = None;
    let mut max_lease: u32 = DEFAULT_MAX_LEASE;
    let mut dev_allow_local = false;

    while let Some(flag) = args.next() {
        let mut take_value = |name: &str| -> Result<String, GatewayError> {
            args.next()
                .ok_or_else(|| GatewayError::Config(format!("{name} requires a value")))
        };
        match flag.as_str() {
            "--listen" => {
                let v = take_value("--listen")?;
                listen = Some(v.parse::<SocketAddr>().map_err(|_| {
                    GatewayError::Config(format!(
                        "--listen expects an addr:port (IP literal), got {v:?}"
                    ))
                })?);
            }
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
            "--dev-allow-local" => dev_allow_local = true,
            other => return Err(GatewayError::Config(format!("unknown flag {other:?}"))),
        }
    }

    let listen = listen.ok_or_else(|| GatewayError::Config("--listen is required".into()))?;
    let journal_path =
        journal_path.ok_or_else(|| GatewayError::Config("--journal is required".into()))?;
    let content_root =
        content_root.ok_or_else(|| GatewayError::Config("--content is required".into()))?;

    Ok(GatewayConfig {
        listen,
        journal_path,
        content_root,
        max_lease,
        dev_allow_local,
    })
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
