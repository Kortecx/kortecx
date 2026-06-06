//! A1 — in-binary TLS for the gRPC listener.
//!
//! Loads a PEM certificate chain + private key into a tonic [`ServerTlsConfig`]
//! (rustls under the hood — the same rustls 0.23 already in the lock via `ureq`,
//! so no new crypto provider). Only the **external** gateway listener is wrapped;
//! the embedded loopback coordinator + worker stay plaintext (internal traffic
//! that never leaves the process's loopback interface).
//!
//! Failure is loud and early: a missing/unreadable cert or key is surfaced by
//! [`server_tls_config`] when the server starts, before binding the port — never a
//! silent fall-back to plaintext (the `--tls-cert`/`--tls-key` both-or-neither
//! check in `config.rs` is the matching guard). Malformed PEM content is rejected
//! by rustls when the server begins serving.

use std::path::Path;

use tonic::transport::{Identity, ServerTlsConfig};

use crate::config::TlsPaths;
use crate::error::GatewayError;

/// Read the PEM cert chain + key from disk and build the gRPC server's TLS config.
pub(crate) fn server_tls_config(paths: &TlsPaths) -> Result<ServerTlsConfig, GatewayError> {
    let cert = read_pem(&paths.cert_path, "tls-cert")?;
    let key = read_pem(&paths.key_path, "tls-key")?;
    let identity = Identity::from_pem(cert, key);
    Ok(ServerTlsConfig::new().identity(identity))
}

/// Read a PEM file, mapping IO failures (missing / unreadable / empty) to a typed
/// TLS error so a bad path fails the server start loudly rather than degrading.
fn read_pem(path: &Path, what: &str) -> Result<Vec<u8>, GatewayError> {
    let bytes = std::fs::read(path)
        .map_err(|e| GatewayError::Tls(format!("--{what} {}: {e}", path.display())))?;
    if bytes.is_empty() {
        return Err(GatewayError::Tls(format!(
            "--{what} {} is empty",
            path.display()
        )));
    }
    Ok(bytes)
}
