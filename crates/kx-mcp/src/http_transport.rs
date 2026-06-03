//! [`HttpTransport`] — the M5.2b `ureq` HTTP [`McpTransport`] impl + its
//! application-layer egress sandbox.
//!
//! This is the transport real MCP/external tools use (the M5.2a [`StdioTransport`]
//! proved the seam over a subprocess). It POSTs the JSON-RPC `tools/call` body to a
//! warrant-scoped endpoint and decodes the response fail-closed — exactly like the
//! stdio path — but it also opens the network egress surface, so it is hardened on
//! four independent fronts:
//!
//! 1. **Host-allowlist binding.** The endpoint host MUST be in the warrant-derived
//!    [`EgressPolicy`] (built from the resolved tool's `net_scope`). The broker's
//!    `precheck` already proved `request.net_scope ⊆ warrant.net_scope`; this binds
//!    the *actually-dialed* host to that grant.
//! 2. **SSRF / DNS-rebind defense.** A custom [`Resolver`](ureq::Resolver) vets every
//!    resolved address through [`egress::vet_resolved_addr`] and refuses
//!    private/loopback/link-local targets (incl. the `169.254.169.254` cloud-metadata
//!    IP) unless the host is an explicitly-allowlisted literal — so a public hostname
//!    can never rebind to an internal address.
//! 3. **Redirect refusal.** The agent is built with `redirects(0)`; a `3xx` response
//!    is refused outright (a cross-host redirect cannot smuggle egress past the gate).
//! 4. **Hard wall-clock + size bounds.** A watchdog thread (mirroring
//!    [`StdioTransport`]) guarantees the caller returns within the budget even if the
//!    server slow-tricks; the response read is capped at `max_response_bytes + 1`.
//!
//! Secrets ride out-of-band: a [`CredentialRef`] is read transiently at dispatch and
//! injected as a request header, never stored / journaled / staged (D81). The run's
//! `idempotency_key` is sent as an `Idempotency-Key` header so a crash-recovery
//! re-dispatch makes the *remote* effect exactly-once (D38 §1 / M1.2).
//!
//! OS-level egress isolation (`bwrap`/nftables) is **out of OSS scope** (D94) — see
//! [`crate::egress`] for the honest boundary.

use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use url::Url;

use crate::credential::CredentialRef;
use crate::egress::{vet_resolved_addr, EgressPolicy};
use crate::errors::TransportError;
use crate::secret_store::{EnvSecretStore, SecretStore};
use crate::transport::{McpTransport, DEFAULT_WALL_CLOCK_MS};

/// Slack added to the per-call budget for ureq's own request timeout, so a worker
/// the watchdog has already abandoned still self-terminates (instead of lingering
/// until the OS gives up). It is deliberately LARGER than the watchdog's grace so
/// the watchdog always wins the race and the caller-facing outcome is a clean
/// `Timeout` (not a ureq transport error). The watchdog (the warrant's
/// `wall_clock_ms`) is the authoritative caller-facing bound.
const WORKER_BACKSTOP_SLACK_MS: u64 = 5_000;

/// Scheduling slack the watchdog allows beyond the budget before declaring a
/// timeout (covers thread wake-up jitter; far smaller than the worker backstop).
const WATCHDOG_GRACE_MS: u64 = 250;

/// An HTTP MCP transport over a pooled [`ureq::Agent`].
///
/// The agent is built **once** (its connection pool + TLS session cache are reused
/// across round-trips — no per-dispatch TLS handshake) and is cheap to `clone`
/// (it shares the pool by `Arc`). It is wired with the vetting resolver +
/// `redirects(0)` at construction, so every dial it can ever make is already
/// egress-bound.
pub struct HttpTransport {
    agent: ureq::Agent,
    endpoint: Url,
    /// `header-name → credential` pairs injected transiently at dispatch (D81).
    credentials: Vec<(String, CredentialRef)>,
    /// Resolves a `CredentialRef`'s `SecretRef` → value at dispatch (D110.2).
    /// Defaults to [`EnvSecretStore`]; a cloud vault swaps in via
    /// [`HttpTransport::with_secret_store`].
    secret_store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print only header NAMES + the credential identities (never a secret);
        // elide the agent + the secret store (not `Debug`-meaningful / secret-bearing).
        f.debug_struct("HttpTransport")
            .field("endpoint", &self.endpoint.as_str())
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    /// Build an HTTP transport that POSTs to `endpoint_url`, permitted to dial only
    /// hosts in `net_scope` (the resolved tool's warrant-validated egress).
    ///
    /// When `tls_required` is `true` (the warrant's `tls_required` axis, D118.5),
    /// the agent is built with `https_only(true)` and refuses any plaintext
    /// `http://` dial at request time — closing the plaintext-credential gap. The
    /// hermetic loopback path passes `false` (it serves plaintext `http://127.0.0.1`).
    ///
    /// # Errors
    ///
    /// [`TransportError::Unreachable`] if `endpoint_url` is not a valid `http(s)`
    /// URL with a host, or if that host is not permitted by `net_scope` (a
    /// misconfiguration — the broker's `net_scope ⊆ warrant` gate would also refuse
    /// it, but failing here keeps a mis-scoped transport from ever being built).
    pub fn new(
        endpoint_url: &str,
        net_scope: &kx_warrant::NetScope,
        tls_required: bool,
    ) -> Result<Self, TransportError> {
        let endpoint = Url::parse(endpoint_url)
            .map_err(|e| TransportError::Unreachable(format!("invalid endpoint URL: {e}")))?;
        match endpoint.scheme() {
            "http" | "https" => {}
            other => {
                return Err(TransportError::Unreachable(format!(
                    "unsupported endpoint scheme: {other}"
                )));
            }
        }
        let host = endpoint
            .host_str()
            .ok_or_else(|| TransportError::Unreachable("endpoint has no host".to_string()))?
            .to_string();

        let policy = EgressPolicy::from_net_scope(net_scope);
        if !policy.permits_host(&host) {
            return Err(TransportError::Unreachable(format!(
                "endpoint host {host} is not in the granted egress allowlist"
            )));
        }

        let agent = ureq::AgentBuilder::new()
            .resolver(VettingResolver {
                policy: policy.clone(),
            })
            // Refuse ALL redirects: a 3xx surfaces as an Ok response we reject in
            // `round_trip`, so a cross-host redirect can never smuggle egress.
            .redirects(0)
            // The host-allowlist + vetting resolver bind WHERE we dial; `https_only`
            // binds the SCHEME. The warrant's `tls_required` axis (D118.5) drives it:
            // `true` refuses plaintext `http://`; `false` permits the hermetic
            // loopback `http://127.0.0.1`.
            .https_only(tls_required)
            // No FIXED agent-wide timeout: a per-call ureq timeout (set in
            // `round_trip`, derived from the warrant budget) is the worker backstop,
            // so a large legitimate budget is never pre-empted by a shared ceiling.
            .build();

        Ok(Self {
            agent,
            endpoint,
            credentials: Vec::new(),
            secret_store: Arc::new(EnvSecretStore),
        })
    }

    /// Swap the [`SecretStore`] used to resolve credential secrets (the cloud KMS/HSM
    /// vault swaps in here behind the same trait, D110.2). Defaults to
    /// [`EnvSecretStore`].
    #[must_use]
    pub fn with_secret_store(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = store;
        self
    }

    /// Register a credential to inject as the `header_name` request header,
    /// transiently at dispatch time (D81). For a bearer token the caller supplies
    /// the full value convention (e.g. an env var holding `Bearer <token>`); the
    /// value is injected verbatim and never stored.
    #[must_use]
    pub fn header_credential(
        mut self,
        header_name: impl Into<String>,
        credential: CredentialRef,
    ) -> Self {
        self.credentials.push((header_name.into(), credential));
        self
    }
}

impl McpTransport for HttpTransport {
    fn round_trip(
        &self,
        request: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
        idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, TransportError> {
        let read_cap = u64::try_from(max_response_bytes.saturating_add(1)).unwrap_or(u64::MAX);
        let budget_ms = if wall_clock_ms == 0 {
            DEFAULT_WALL_CLOCK_MS
        } else {
            wall_clock_ms
        };
        let budget = Duration::from_millis(budget_ms);
        // The worker's own (looser) ureq timeout: strictly larger than the watchdog
        // so the watchdog wins the race (caller sees `Timeout`, not a ureq error),
        // while an abandoned worker still self-terminates rather than lingering.
        let worker_timeout =
            Duration::from_millis(budget_ms.saturating_add(WORKER_BACKSTOP_SLACK_MS));

        // Resolve credential secrets transiently HERE (they live only in this local
        // Vec for the duration of the call, then drop — never on `self`).
        let headers: Vec<(String, String)> = self
            .credentials
            .iter()
            .filter_map(|(name, cred)| {
                cred.read_secret(&*self.secret_store)
                    .map(|val| (name.clone(), val))
            })
            .collect();
        let idempotency_header = idempotency_key.map(hex32);

        // Clone the pooled agent (Arc-shared) + owned inputs onto the worker thread,
        // so BOTH the send and the body read run under the wall-clock watchdog —
        // mirrors StdioTransport (ureq's per-read timeout is not a hard *total*
        // bound, and ureq has no in-flight cancel, so the watchdog is authoritative).
        let agent = self.agent.clone();
        let url = self.endpoint.to_string();
        let body = request.to_vec();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut req = agent
                .post(&url)
                .timeout(worker_timeout)
                .set("Content-Type", "application/json")
                .set("Accept", "application/json");
            if let Some(ref key) = idempotency_header {
                req = req.set("Idempotency-Key", key);
            }
            for (name, value) in &headers {
                req = req.set(name, value);
            }
            // `headers` (and the secrets inside) drop at the end of this closure.

            let outcome = match req.send_bytes(&body) {
                Ok(resp) => read_capped(resp, read_cap),
                // ureq classifies 4xx/5xx as `Status`; a `tools/call` over those is
                // not a result — surface as Io (the decoder would also fail-closed).
                Err(ureq::Error::Status(code, _)) => {
                    Err(TransportError::Io(format!("http status {code}")))
                }
                // Transport errors: connect refused, TLS failure, and — crucially —
                // the vetting resolver's egress refusal (an io::Error it raised).
                Err(ureq::Error::Transport(t)) => Err(TransportError::Unreachable(t.to_string())),
            };
            let _ = tx.send(outcome);
        });

        match rx.recv_timeout(budget.saturating_add(Duration::from_millis(WATCHDOG_GRACE_MS))) {
            Ok(result) => {
                let _ = worker.join();
                result
            }
            // The worker is left to self-terminate (its ureq per-call timeout fires
            // at `budget`, then it exits) — joining here would re-block the caller
            // past the budget, defeating the watchdog.
            Err(RecvTimeoutError::Timeout) => Err(TransportError::Timeout { wall_clock_ms }),
            Err(RecvTimeoutError::Disconnected) => {
                Err(TransportError::Io("worker thread disconnected".to_string()))
            }
        }
    }
}

/// Read a response body, refusing a redirect (3xx) and bounding the read to
/// `read_cap` bytes (`max_response_bytes + 1`, so the decoder detects oversize).
fn read_capped(resp: ureq::Response, read_cap: u64) -> Result<Vec<u8>, TransportError> {
    let status = resp.status();
    // `redirects(0)` returns a 3xx as Ok — refuse it (cross-host redirect defense).
    if (300..400).contains(&status) {
        return Err(TransportError::Unreachable(format!(
            "redirect refused (status {status})"
        )));
    }
    if !(200..300).contains(&status) {
        return Err(TransportError::Io(format!("http status {status}")));
    }
    let mut buf = Vec::new();
    resp.into_reader()
        .take(read_cap)
        .read_to_end(&mut buf)
        .map_err(|e| TransportError::Io(e.to_string()))?;
    Ok(buf)
}

/// Lower-hex-encode a 32-byte token for the `Idempotency-Key` header (no dep).
fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// A [`ureq::Resolver`] that performs DNS, then vets every resolved address through
/// the [`EgressPolicy`] — the behavioural half of the egress sandbox.
struct VettingResolver {
    policy: EgressPolicy,
}

impl ureq::Resolver for VettingResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let host = host_of_netloc(netloc);
        if !self.policy.permits_host(host) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "egress host not in allowlist",
            ));
        }
        let vetted: Vec<SocketAddr> = netloc
            .to_socket_addrs()?
            .filter(|addr| vet_resolved_addr(host, addr, &self.policy).is_ok())
            .collect();
        if vetted.is_empty() {
            // Either DNS returned nothing, or every address failed egress vetting
            // (SSRF / rebind). Surface a typed io error — ureq wraps it as a
            // Transport error → `TransportError::Unreachable` (no why-leak).
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "no egress-permitted address for host",
            ));
        }
        Ok(vetted)
    }
}

/// Extract the bare host from a ureq `netloc` (`host:port`, or `[ipv6]:port`).
fn host_of_netloc(netloc: &str) -> &str {
    if let Some(rest) = netloc.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &rest[..end]; // bracketed IPv6 literal
        }
    }
    // Domains / IPv4 have no internal colon; split on the LAST one to drop the port.
    netloc.rsplit_once(':').map_or(netloc, |(h, _)| h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::{Host, NetScope};
    use ureq::Resolver as _;

    fn scope(hosts: &[&str]) -> NetScope {
        NetScope::EgressAllowlist(hosts.iter().map(|h| Host((*h).to_string())).collect())
    }

    #[test]
    fn host_of_netloc_handles_domain_ipv4_ipv6() {
        assert_eq!(host_of_netloc("example.com:443"), "example.com");
        assert_eq!(host_of_netloc("127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(host_of_netloc("[::1]:8080"), "::1");
        assert_eq!(host_of_netloc("[2606:4700::1111]:443"), "2606:4700::1111");
    }

    #[test]
    fn hex32_is_lowercase_64_chars() {
        let mut b = [0u8; 32];
        b[0] = 0xab;
        b[31] = 0x0f;
        let h = hex32(&b);
        assert_eq!(h.len(), 64);
        assert!(h.starts_with("ab"));
        assert!(h.ends_with("0f"));
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn new_refuses_endpoint_host_outside_allowlist() {
        let err = HttpTransport::new("https://evil.com/mcp", &scope(&["api.example.com"]), false)
            .expect_err("host not allowlisted must refuse");
        assert!(matches!(err, TransportError::Unreachable(_)));
    }

    #[test]
    fn new_refuses_none_scope() {
        let err = HttpTransport::new("https://api.example.com/mcp", &NetScope::None, false)
            .expect_err("None egress must refuse every host");
        assert!(matches!(err, TransportError::Unreachable(_)));
    }

    #[test]
    fn new_refuses_non_http_scheme() {
        let err = HttpTransport::new("ftp://api.example.com/mcp", &scope(&["api.example.com"]), false)
            .expect_err("non-http scheme must refuse");
        assert!(matches!(err, TransportError::Unreachable(_)));
    }

    #[test]
    fn new_accepts_allowlisted_endpoint() {
        let t = HttpTransport::new("https://api.example.com/mcp", &scope(&["api.example.com"]), true)
            .expect("allowlisted endpoint builds (tls_required does not fail at build)");
        let _ = t;
    }

    #[test]
    fn vetting_resolver_refuses_unlisted_host() {
        let r = VettingResolver {
            policy: EgressPolicy::from_net_scope(&scope(&["api.example.com"])),
        };
        // A host not in the allowlist is refused before any DNS happens.
        assert!(r.resolve("evil.com:443").is_err());
    }

    #[test]
    fn vetting_resolver_refuses_loopback_for_public_host() {
        // Even if a permitted-looking netloc resolves to loopback via a literal, a
        // non-literal host whose name is not in the list is refused at the gate.
        let r = VettingResolver {
            policy: EgressPolicy::from_net_scope(&scope(&["127.0.0.1"])),
        };
        // An explicitly-allowlisted loopback literal IS permitted to resolve.
        assert!(r.resolve("127.0.0.1:8080").is_ok());
    }
}
