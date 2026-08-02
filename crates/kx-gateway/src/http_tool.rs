//! The bundled `http@1` capability — the workflow `http` step's deterministic,
//! credentialed dial (the runtime calls the world; no model anywhere).
//!
//! # What it is
//!
//! A [`ToolKind::Builtin`] sibling of `retrieve@1`: an authored `tool()` node
//! whose identity-bearing args (`config_subset[TOOL_ARGS_KEY]`) declare the
//! url/method/body and — by NAME only — the credential to inject. The
//! coordinator derives the per-call `net_scope` from the DECLARED url's host
//! and refuses an ungranted `secret_name` at lease (both checked against the
//! step's server-built warrant); the broker's `request ⊆ warrant` precheck
//! enforces the egress again at dispatch; and this capability re-vets the
//! actually-dialed address a third time through the SSRF resolver — defense in
//! depth over ONE authorization kernel, never a second one.
//!
//! # Boundaries (load-bearing)
//!
//! - **The secret VALUE never rests.** `secret_name` is a `SecretRef` NAME
//!   resolved transiently at dispatch through the SAME store→env chain the
//!   MCP dial uses (D81/D110.2); it is injected as a header and never
//!   journaled, staged, or logged.
//! - **Redirects refused (`redirects(0)`)** — a 3xx surfaces as a refused
//!   response, so a cross-host redirect can never smuggle egress past the
//!   allowlist.
//! - **Size-capped, refuse-not-truncate.** A response larger than
//!   [`MAX_RESPONSE_BYTES`] fails the effect — a truncated observation would
//!   be a plausible-looking lie.
//! - **5xx and transport faults FAIL the effect** (a retry-able failure for
//!   the per-step failure policy); 2xx–4xx COMMIT `{status, content_type,
//!   body}` — a 404 is an answer, an unreachable host is not.
//! - **`Idempotency-Key` header** carries the run-scoped token on every dial
//!   that has one, so a crash-redispatch of a POST is remote-dedupable (the
//!   `HttpTransport` precedent).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore};
use kx_mcp::{vet_resolved_addr, EgressPolicy, SecretStore};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::{IdempotencyClass, InputSchema, ParamSpec, ParamType, ToolDef, ToolKind};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, SecretRef, ToolRequirement};
use serde::{Deserialize, Serialize};

/// Both read (GET/HEAD → ReadOnlyNondet) and world-mutating (POST/… →
/// WorldMutating) http steps dispatch stage-then-commit.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// Response cap — over it the effect FAILS (refuse, never truncate).
const MAX_RESPONSE_BYTES: usize = 1 << 20; // 1 MiB

/// Default + ceiling for the per-call wall clock.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;

/// The bundled deterministic HTTP capability (`http@1`).
pub(crate) struct HttpCapability {
    name: ToolName,
    version: ToolVersion,
    secrets: Arc<dyn SecretStore>,
}

impl HttpCapability {
    pub(crate) fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            name: ToolName("http".into()),
            version: ToolVersion("1".into()),
            secrets,
        }
    }
}

/// The authored argument bag (validated fail-closed against the typed
/// `inputSchema` at authoring AND at lease; re-parsed here against smuggled
/// keys — the `retrieve@1` posture).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpArgs {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    body: Option<String>,
    /// The credential's `SecretRef` NAME (never a value).
    #[serde(default)]
    secret_name: Option<String>,
    /// The header the resolved secret is injected as (default `authorization`).
    #[serde(default)]
    secret_header: Option<String>,
    /// The value prefix (default `Bearer`; empty = inject the raw value).
    #[serde(default)]
    secret_scheme: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// The committed observation: an ANSWERED dial (2xx–4xx). Canonical JSON —
/// downstream steps (a conditional's json-path, a transform) read it verbatim.
#[derive(Serialize)]
struct HttpObservation {
    status: u16,
    content_type: String,
    body: String,
}

/// Extract the host from an `http(s)` URL (the same authority walk the
/// envelope's userinfo check uses; `url::Url` does the real parse).
fn host_of(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported scheme: {other}")),
    }
    parsed
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| "url has no host".to_string())
}

/// The `ureq` resolver that vets every resolved address through the SSRF/
/// egress kernel (the `HttpTransport` resolver, restated over the PUBLIC
/// `vet_resolved_addr` so this crate takes no private kx-mcp surface).
struct VettingResolver {
    policy: EgressPolicy,
}

impl ureq::Resolver for VettingResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        use std::net::ToSocketAddrs;
        let host = netloc.rsplit_once(':').map_or(netloc, |(h, _)| h);
        let addrs: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        let mut vetted = Vec::with_capacity(addrs.len());
        for addr in addrs {
            match vet_resolved_addr(host, &addr, &self.policy) {
                Ok(()) => vetted.push(addr),
                Err(denied) => {
                    return Err(std::io::Error::other(format!("egress refused: {denied:?}")));
                }
            }
        }
        Ok(vetted)
    }
}

impl Capability for HttpCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn version(&self) -> &ToolVersion {
        &self.version
    }

    fn supported_patterns(&self) -> &[EffectPattern] {
        PATTERNS
    }

    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        let args: HttpArgs = serde_json::from_slice(&request.payload)
            .map_err(|e| CapabilityFailureReason::Other(format!("http: bad args: {e}")))?;
        let host =
            host_of(&args.url).map_err(|e| CapabilityFailureReason::Other(format!("http: {e}")))?;
        // Defense in depth: the request scope the coordinator derived from the
        // SAME declared url (and the broker already checked ⊆ warrant) must
        // permit this host, and the resolver below re-vets every resolved
        // address (rebind/SSRF).
        let policy = EgressPolicy::from_net_scope(&request.net_scope);
        if !policy.permits_host(&host) {
            return Err(CapabilityFailureReason::Other(format!(
                "http: host {host} is not in the granted egress allowlist"
            )));
        }
        let method = args.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
        let timeout = args
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(100, MAX_TIMEOUT_MS);
        let agent = ureq::AgentBuilder::new()
            .resolver(VettingResolver { policy })
            .redirects(0)
            .build();
        let mut req = agent
            .request(&method, &args.url)
            .timeout(Duration::from_millis(timeout));
        // Transient credential injection (D81): resolved by NAME at dispatch,
        // never stored, never logged.
        if let Some(name) = args.secret_name.as_deref() {
            let Some(value) = self.secrets.resolve(&SecretRef(name.to_string())) else {
                return Err(CapabilityFailureReason::Other(format!(
                    "http: secret {name:?} did not resolve (local store or environment)"
                )));
            };
            let header = args.secret_header.as_deref().unwrap_or("authorization");
            let scheme = args.secret_scheme.as_deref().unwrap_or("Bearer");
            let header_value = if scheme.is_empty() {
                value
            } else {
                format!("{scheme} {value}")
            };
            req = req.set(header, &header_value);
        }
        if let Some(key) = request.idempotency_key.as_ref() {
            req = req.set("idempotency-key", &hex(key));
        }
        let result = match args.body.as_deref() {
            Some(body) if method != "GET" && method != "HEAD" => req.send_string(body),
            _ => req.call(),
        };
        let response = match result {
            Ok(r) => r,
            // A NON-5xx status ureq surfaces as Status is an ANSWER (committed
            // below); 5xx and transport faults FAIL the effect (retry-able).
            Err(ureq::Error::Status(code, r)) if code < 500 => r,
            Err(ureq::Error::Status(code, _)) => {
                return Err(CapabilityFailureReason::Other(format!(
                    "http: {method} {host} answered {code}"
                )));
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(CapabilityFailureReason::Other(format!(
                    "http: {method} {host} transport: {t}"
                )));
            }
        };
        let status = response.status();
        // Redirects are refused by construction (redirects(0) surfaces 3xx
        // here) — a cross-host Location can never be followed; commit it as an
        // honest answer like any other status.
        let content_type = response.content_type().to_string();
        let mut body = Vec::new();
        {
            use std::io::Read;
            response
                .into_reader()
                .take((MAX_RESPONSE_BYTES + 1) as u64)
                .read_to_end(&mut body)
                .map_err(|e| CapabilityFailureReason::Other(format!("http: read: {e}")))?;
        }
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(CapabilityFailureReason::Other(format!(
                "http: response exceeds the {MAX_RESPONSE_BYTES}-byte cap (refused, not truncated)"
            )));
        }
        let body = String::from_utf8_lossy(&body).into_owned();
        serde_json::to_vec(&HttpObservation {
            status,
            content_type,
            body,
        })
        .map_err(|e| CapabilityFailureReason::Other(format!("http: encode: {e}")))
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(64);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The bundled http tool's identity — `http@1` (a FLAT builtin id, the
/// `retrieve@1` precedent).
#[must_use]
pub(crate) fn http_tool() -> (ToolName, ToolVersion) {
    (ToolName("http".into()), ToolVersion("1".into()))
}

/// The `http@1` [`ToolDef`]. The DECLARED net requirement is `None` because the
/// real requirement is PER-CALL (derived by the coordinator from the authored
/// url's host and checked against the step warrant at lease — a static
/// declaration could only be wrong in one direction or the other).
/// `IdempotencyClass::Token` so a world-mutating dispatch carries the
/// run-scoped `Idempotency-Key` (R-10).
#[must_use]
pub(crate) fn http_tool_def() -> ToolDef {
    let (tool_id, tool_version) = http_tool();
    ToolDef {
        tool_id,
        tool_version,
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
            net_scope_required: NetScope::None,
            fs_scope_required: FsScope::empty(),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: "Deterministic HTTP dial for workflow http steps. Args: {\"url\": <http(s) url>, \"method\": <GET|POST|..., default GET>, \"body\": <string, optional>, \"secret_name\": <SecretRef NAME injected as a header, optional>, \"secret_header\": <default authorization>, \"secret_scheme\": <default Bearer>, \"timeout_ms\": <100..120000>}. Commits {status, content_type, body}; 5xx/transport faults fail the effect. Egress is bound to the step warrant's allowlist; redirects are refused.".into(),
        idempotency_class: IdempotencyClass::Token,
        input_schema: Some(InputSchema {
            params: vec![
                ParamSpec {
                    name: "url".into(),
                    ty: ParamType::Str { max_len: 2048 },
                    required: true,
                },
                ParamSpec {
                    name: "method".into(),
                    ty: ParamType::Str { max_len: 8 },
                    required: false,
                },
                ParamSpec {
                    name: "body".into(),
                    ty: ParamType::Str { max_len: 65_536 },
                    required: false,
                },
                ParamSpec {
                    name: "secret_name".into(),
                    ty: ParamType::Str { max_len: 128 },
                    required: false,
                },
                ParamSpec {
                    name: "secret_header".into(),
                    ty: ParamType::Str { max_len: 64 },
                    required: false,
                },
                ParamSpec {
                    name: "secret_scheme".into(),
                    ty: ParamType::Str { max_len: 16 },
                    required: false,
                },
                ParamSpec {
                    name: "timeout_ms".into(),
                    ty: ParamType::Int {
                        min: Some(100),
                        max: Some(120_000),
                    },
                    required: false,
                },
            ],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled [`HttpCapability`] (`http@1`) on the serve broker over
/// the SAME store→env secret chain the MCP dial uses.
pub(crate) fn register_http_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
    secrets: Arc<dyn SecretStore>,
) {
    broker.register_capability(Box::new(HttpCapability::new(secrets)));
    tracing::info!("http@1 capability registered (deterministic workflow http steps)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_extraction_is_scheme_bound() {
        assert_eq!(host_of("http://127.0.0.1:8080/x").unwrap(), "127.0.0.1");
        assert_eq!(
            host_of("https://api.example.com/v1").unwrap(),
            "api.example.com"
        );
        assert!(host_of("ftp://x.example").is_err());
        assert!(host_of("not a url").is_err());
        assert!(host_of("file:///etc/passwd").is_err());
    }

    #[test]
    fn an_unallowed_host_is_refused_before_any_dial() {
        // NetScope::None ⇒ empty policy ⇒ every host refused — the capability
        // fails BEFORE constructing an agent (no socket is ever opened).
        let cap = HttpCapability::new(Arc::new(kx_mcp::EnvSecretStore));
        let req = EffectRequest {
            payload: br#"{"url":"http://127.0.0.1:1/x"}"#.to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
            secret_scope: kx_warrant::SecretScope::None,
        };
        let err = cap.invoke(&req).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("not in the granted egress allowlist"), "{msg}");
    }

    #[test]
    fn a_missing_secret_is_refused_by_name_never_fabricated() {
        use std::collections::BTreeSet;
        let mut hosts = BTreeSet::new();
        hosts.insert(kx_warrant::Host("127.0.0.1".into()));
        let cap = HttpCapability::new(Arc::new(kx_mcp::EnvSecretStore));
        let req = EffectRequest {
            payload:
                br#"{"url":"http://127.0.0.1:1/x","secret_name":"KX_TEST_NO_SUCH_SECRET_XYZ"}"#
                    .to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::EgressAllowlist(hosts),
            fs_scope: FsScope::empty(),
            secret_scope: kx_warrant::SecretScope::None,
        };
        let err = cap.invoke(&req).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("did not resolve"), "{msg}");
        // The refusal names the SECRET NAME, never a value.
        assert!(msg.contains("KX_TEST_NO_SUCH_SECRET_XYZ"), "{msg}");
    }
}
