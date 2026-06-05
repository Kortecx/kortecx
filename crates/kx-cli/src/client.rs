//! Gateway-client plumbing shared by the client verbs: the common flags
//! (`--endpoint` / `--token` / `--token-file` / `--json`), credential
//! resolution, dialing a [`KxGatewayClient`], and attaching a bearer token to a
//! request.
//!
//! Identity is server-derived (SN-8): the CLI sends a *credential* (a bearer
//! token), never a claimed identity. A `--dev-allow-local` server needs no token.

use std::path::PathBuf;

use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use crate::error::CliError;

/// The default gateway endpoint (matches the conventional `kx serve` listen
/// address; see [`crate::cli::DEFAULT_LISTEN`]).
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:50151";

/// Flags every client verb accepts. Populated by the per-verb arg loops via
/// [`ClientCommon::try_consume`].
#[derive(Debug, Clone)]
pub struct ClientCommon {
    /// The gateway endpoint URL (`--endpoint`, default [`DEFAULT_ENDPOINT`]).
    pub endpoint: String,
    /// An inline bearer token (`--token`). Visible in `ps`; prefer `--token-file`.
    pub token: Option<String>,
    /// A file holding a bearer token (`--token-file`); the file is read + trimmed.
    pub token_file: Option<PathBuf>,
    /// Emit machine-readable JSON instead of the human rendering (`--json`).
    pub json: bool,
}

impl Default for ClientCommon {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            token: None,
            token_file: None,
            json: false,
        }
    }
}

/// Pull the next value for a flag, erroring with a usage message if absent.
pub fn next_value<I: Iterator<Item = String>>(
    args: &mut I,
    name: &str,
) -> Result<String, CliError> {
    args.next()
        .ok_or_else(|| CliError::Usage(format!("{name} requires a value")))
}

/// Pull the next value and hex-decode it to a fixed-size byte array (a
/// wrong-length / non-hex value becomes a usage error). Used by the verbs that
/// take a `--instance` (16B) / `--ref` / `--id` (32B).
pub fn take_fixed<I: Iterator<Item = String>, const N: usize>(
    args: &mut I,
    name: &str,
) -> Result<[u8; N], CliError> {
    Ok(crate::hex::decode_fixed::<N>(&next_value(args, name)?)?)
}

impl ClientCommon {
    /// If `flag` is a common client flag, consume it (pulling its value from
    /// `args` when needed) and return `Ok(true)`; otherwise `Ok(false)` so the
    /// caller can handle a verb-specific flag.
    pub fn try_consume<I: Iterator<Item = String>>(
        &mut self,
        flag: &str,
        args: &mut I,
    ) -> Result<bool, CliError> {
        match flag {
            "--endpoint" => self.endpoint = next_value(args, "--endpoint")?,
            "--token" => self.token = Some(next_value(args, "--token")?),
            "--token-file" => {
                self.token_file = Some(PathBuf::from(next_value(args, "--token-file")?));
            }
            "--json" => self.json = true,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Resolve the effective credential, applying the precedence + safety nudges:
    /// `--token-file` (preferred) over `--token`; the two are mutually exclusive.
    /// Warns (stderr, non-fatal) when an inline `--token` is used (argv-visible)
    /// and when a token would be sent to a non-loopback plaintext endpoint
    /// (bearer-over-plaintext is dev/loopback only; TLS/mTLS is a later step).
    pub fn resolve(&self) -> Result<Resolved, CliError> {
        if self.token.is_some() && self.token_file.is_some() {
            return Err(CliError::Usage(
                "--token and --token-file are mutually exclusive".into(),
            ));
        }
        let token = match &self.token_file {
            Some(path) => {
                let body = std::fs::read_to_string(path)
                    .map_err(|e| CliError::Io(format!("--token-file {}: {e}", path.display())))?;
                let trimmed = body.trim().to_string();
                if trimmed.is_empty() {
                    return Err(CliError::Usage(format!(
                        "--token-file {} is empty",
                        path.display()
                    )));
                }
                Some(trimmed)
            }
            None => self.token.clone(),
        };
        if self.token.is_some() {
            eprintln!("kx: warning: --token is visible in the process list; prefer --token-file");
        }
        if token.is_some() && is_nonloopback_plaintext(&self.endpoint) {
            eprintln!(
                "kx: warning: sending a bearer token to a non-loopback plaintext endpoint ({}); \
                 it travels in cleartext — TLS/mTLS is a later step",
                self.endpoint
            );
        }
        Ok(Resolved {
            endpoint: self.endpoint.clone(),
            token,
        })
    }
}

/// A resolved endpoint + optional bearer token.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The gateway endpoint to dial.
    pub endpoint: String,
    /// The bearer token to attach (if any).
    pub token: Option<String>,
}

impl Resolved {
    /// Dial the gateway, mapping a transport failure to [`CliError::Connect`].
    pub async fn connect(&self) -> Result<KxGatewayClient<Channel>, CliError> {
        KxGatewayClient::connect(self.endpoint.clone())
            .await
            .map_err(|e| CliError::Connect {
                endpoint: self.endpoint.clone(),
                detail: e.to_string(),
            })
    }

    /// Wrap `payload` in a request, attaching `authorization: Bearer <token>`
    /// when a token is set. A token with non-ASCII / control characters is a
    /// usage error (it can't form a valid metadata value) rather than a panic.
    pub fn request<T>(&self, payload: T) -> Result<tonic::Request<T>, CliError> {
        let mut req = tonic::Request::new(payload);
        if let Some(token) = &self.token {
            let value = format!("Bearer {token}").parse().map_err(|_| {
                CliError::Usage("--token contains characters not valid in an HTTP header".into())
            })?;
            req.metadata_mut().insert("authorization", value);
        }
        Ok(req)
    }
}

/// `true` iff `endpoint` is plaintext `http://` to a non-loopback host (the case
/// where a bearer token would travel in cleartext over a network).
#[must_use]
pub fn is_nonloopback_plaintext(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return false; // https:// (or anything else) is not plaintext-http
    };
    // A bracketed IPv6 host (`[::1]:port`) keeps its colons inside the brackets;
    // a plain host runs up to the first ':' (port) or '/' (path).
    let host = if let Some(after) = rest.strip_prefix('[') {
        after.split(']').next().unwrap_or(after)
    } else {
        rest.split(['/', ':']).next().unwrap_or(rest)
    };
    !matches!(host, "127.0.0.1" | "::1" | "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_endpoint_and_no_token() {
        let c = ClientCommon::default();
        assert_eq!(c.endpoint, DEFAULT_ENDPOINT);
        assert!(c.token.is_none() && c.token_file.is_none() && !c.json);
    }

    #[test]
    fn try_consume_handles_common_flags_only() {
        let mut c = ClientCommon::default();
        let mut it = vec!["http://h:1".to_string()].into_iter();
        assert!(c.try_consume("--endpoint", &mut it).unwrap());
        assert_eq!(c.endpoint, "http://h:1");
        assert!(c.try_consume("--json", &mut std::iter::empty()).unwrap());
        assert!(c.json);
        // A non-common flag is left for the caller.
        assert!(!c
            .try_consume("--instance", &mut std::iter::empty())
            .unwrap());
        // A common flag missing its value is a usage error.
        assert!(c.try_consume("--token", &mut std::iter::empty()).is_err());
    }

    #[test]
    fn token_and_token_file_are_mutually_exclusive() {
        let c = ClientCommon {
            token: Some("t".into()),
            token_file: Some(PathBuf::from("/x")),
            ..ClientCommon::default()
        };
        assert!(c.resolve().is_err());
    }

    #[test]
    fn token_file_is_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tok");
        std::fs::write(&path, "  s3cr3t\n").unwrap();
        let c = ClientCommon {
            token_file: Some(path),
            ..ClientCommon::default()
        };
        assert_eq!(c.resolve().unwrap().token.as_deref(), Some("s3cr3t"));
    }

    #[test]
    fn empty_token_file_is_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tok");
        std::fs::write(&path, "   \n").unwrap();
        let c = ClientCommon {
            token_file: Some(path),
            ..ClientCommon::default()
        };
        assert!(c.resolve().is_err());
    }

    #[test]
    fn plaintext_detection() {
        assert!(is_nonloopback_plaintext("http://example.com:50151"));
        assert!(is_nonloopback_plaintext("http://10.0.0.5:50151"));
        assert!(!is_nonloopback_plaintext("http://127.0.0.1:50151"));
        assert!(!is_nonloopback_plaintext("http://localhost:50151"));
        assert!(!is_nonloopback_plaintext("http://[::1]:50151"));
        assert!(!is_nonloopback_plaintext("https://example.com")); // TLS, not plaintext
    }

    #[test]
    fn request_attaches_bearer_and_rejects_bad_token() {
        let ok = Resolved {
            endpoint: DEFAULT_ENDPOINT.into(),
            token: Some("s3cr3t".into()),
        };
        let req = ok.request(()).unwrap();
        assert_eq!(
            req.metadata()
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer s3cr3t"
        );
        let bad = Resolved {
            endpoint: DEFAULT_ENDPOINT.into(),
            token: Some("bad\ntoken".into()),
        };
        assert!(bad.request(()).is_err());
    }
}
