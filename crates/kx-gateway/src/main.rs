#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

//! `kx-gateway` — the kortecx single-system gateway server binary (R1 / D130).
//!
//! ```text
//! kx-gateway serve --listen <addr:port> --journal <path> --content <dir> \
//!                  [--max-lease <N>] [--dev-allow-local]
//! kx-gateway --help | --version
//! ```
//!
//! `serve` brings up an embedded single-system runtime (coordinator + local
//! worker) and hosts the FROZEN `KxGateway` gRPC service over it, so a client
//! can `SubmitRun` a workflow and observe it reach `Committed` over the network.
//! The bound port defaults to deny-all auth; pass `--dev-allow-local` (loopback
//! only) for dev access. Thin: parse → call the library. `RUST_LOG` controls
//! tracing (default `info`).

use std::process::ExitCode;

use kx_gateway::{serve, Cli, USAGE};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init()
        .ok();

    match Cli::from_args(std::env::args().skip(1)) {
        Ok(Cli::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Cli::Version) => {
            println!("kx-gateway {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Cli::Serve(cfg)) => match serve(cfg).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("kx-gateway: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("kx-gateway: {e}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
