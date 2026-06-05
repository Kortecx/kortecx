#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

//! `kx` — the kortecx unified CLI binary (R3 / D130). Thin: init tracing, parse
//! `argv`, hand off to [`kx_cli::run`], propagate its [`std::process::ExitCode`].
//!
//! ```text
//! kx run|replay|digest --journal <path> --content <dir> [...]      # forward to the engine
//! kx serve --journal <path> --content <dir> [--listen addr:port] [...]   # forward to the gateway
//! kx invoke <handle> --args <json> [--wait] [--endpoint url] [--token t]  # gRPC client verbs
//! kx submit --demo [--wait]
//! kx projection --instance <hex16> [--at-seq N]
//! kx content --ref <hex32> --instance <hex16> [--out <file>]
//! kx events --instance <hex16> [--since N] [--follow]
//! kx signatures list | get --id <hex32> | register --manifest-file <path>
//! kx --help | --version | help <command>
//! ```
//!
//! `RUST_LOG` controls tracing (default `info`); logs go to stderr so stdout
//! stays a clean, parseable result channel for agents and pipelines.

use std::process::ExitCode;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init()
        .ok();

    kx_cli::run(std::env::args().skip(1)).await
}
