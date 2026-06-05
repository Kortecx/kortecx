// `tonic::Status` is a large error type (~176 B), but it is the REQUIRED error
// of the tonic `Interceptor` contract (`Result<Request<()>, Status>`) and of
// `Status`-returning resolvers — it cannot be boxed without an extra conversion
// at every call site. Allow the perf lint here where the type is dictated by
// tonic, not chosen by us.
#![allow(clippy::result_large_err)]

//! The auth seam (R1 deny-all stub; R2 fills it with a real bearer-token
//! resolver).
//!
//! Rule 8c — *the moment a port is bound there is no OSS auth* — so the freshly
//! bound gateway defaults to [`DenyAll`]: every RPC is rejected with
//! `unauthenticated` unless the operator opts into [`DevAllowLocal`]
//! (`--dev-allow-local`, loopback-only) or configures real credentials
//! ([`TokenResolver`], `--auth-token`/`--auth-token-file`). R2 fills the
//! [`PrincipalResolver`] seam WITHOUT changing the trait: [`Principal`] grows
//! additively (a `party` field), and the interceptor wiring is unchanged.
//!
//! Identity is **server-derived** from transport metadata (SN-8) — the client
//! supplies a *credential* (a bearer token), never a claimed identity. mTLS /
//! OIDC are later impls of the SAME trait (OIDC stays cloud, D94/D101.1).
//! gateway-core stays auth-free (its dep-wall + `lib.rs` comment); auth lives
//! here, in the host binary.

use std::collections::HashMap;

use tonic::metadata::MetadataMap;
use tonic::Status;

#[cfg(feature = "embedded-worker")]
use std::sync::Arc;
#[cfg(feature = "embedded-worker")]
use tonic::Request;

/// A resolved caller identity. A struct (not an enum matched at call sites) so
/// new fields are additive and never a breaking change.
#[derive(Clone, Debug)]
pub struct Principal {
    /// The caller's subject. `"local-dev"` under [`DevAllowLocal`]; the party
    /// handle under [`TokenResolver`].
    pub subject: String,
    /// The party handle the caller acts as — the server-derived identity a recipe
    /// `Invoke` binds authority under (R2b). Equals `subject` for the token + dev
    /// resolvers; a future mTLS / OIDC resolver may diverge the two.
    pub party: String,
}

/// The auth seam: resolve a [`Principal`] from request metadata, or reject. The
/// resolver runs in a tonic interceptor BEFORE the request reaches the gateway
/// service, so a rejected request never touches the read-fold / propose-proxy.
pub trait PrincipalResolver: Send + Sync + 'static {
    /// Resolve the caller, or return the `Status` to fail the RPC with.
    fn resolve(&self, metadata: &MetadataMap) -> Result<Principal, Status>;
}

/// The SAFE default: reject every request. A bound port with no explicit
/// `--dev-allow-local` is a closed door (no silent open access).
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyAll;

impl PrincipalResolver for DenyAll {
    fn resolve(&self, _metadata: &MetadataMap) -> Result<Principal, Status> {
        Err(Status::unauthenticated(
            "kx-gateway: no auth configured; pass --dev-allow-local for loopback-only dev access (R2 adds real auth)",
        ))
    }
}

/// The explicit dev escape hatch (`--dev-allow-local`): allow every request,
/// attributing a fixed `local-dev` principal. Intended for a loopback `--listen`
/// only; the server refuses a non-loopback bind under this resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct DevAllowLocal;

impl PrincipalResolver for DevAllowLocal {
    fn resolve(&self, _metadata: &MetadataMap) -> Result<Principal, Status> {
        Ok(Principal {
            subject: "local-dev".to_string(),
            party: "local-dev".to_string(),
        })
    }
}

/// A bearer-token [`PrincipalResolver`]: a configured map of opaque token → party
/// handle. The token is read from the `authorization: Bearer <token>` gRPC
/// metadata header; the caller supplies a *credential*, never a claimed identity
/// (SN-8 — identity is server-derived). OIDC stays cloud (D94/D101.1); this is
/// the OSS single-system credential check.
///
/// Every failure mode — missing header, wrong scheme, malformed value, unknown
/// token — returns the SAME `unauthenticated` (no "valid-format-but-unknown"
/// oracle). Token comparison is a `HashMap` lookup (not constant-time); the
/// constant-time / OIDC path is cloud. Prefer `--auth-token-file` over
/// `--auth-token` so tokens stay off a world-readable `argv`.
#[derive(Clone, Debug, Default)]
pub struct TokenResolver {
    tokens: HashMap<String, String>,
}

impl TokenResolver {
    /// Build a resolver from a `token → party` map.
    #[must_use]
    pub fn new(tokens: HashMap<String, String>) -> Self {
        Self { tokens }
    }

    /// `true` when no tokens are configured (the host then keeps deny-all).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// The required `authorization` scheme prefix.
const BEARER_PREFIX: &str = "Bearer ";

impl PrincipalResolver for TokenResolver {
    fn resolve(&self, metadata: &MetadataMap) -> Result<Principal, Status> {
        // One uniform error for every failure mode — no existence oracle.
        let unauth = || Status::unauthenticated("invalid or missing bearer token");
        let raw = metadata.get("authorization").ok_or_else(unauth)?;
        let value = raw.to_str().map_err(|_| unauth())?;
        let token = value.strip_prefix(BEARER_PREFIX).ok_or_else(unauth)?;
        let party = self.tokens.get(token).ok_or_else(unauth)?;
        Ok(Principal {
            subject: party.clone(),
            party: party.clone(),
        })
    }
}

/// Build the tonic request interceptor for `resolver`. On success it stashes the
/// resolved [`Principal`] in the request extensions (server-derived identity for
/// R2 to read) and forwards the request; on failure it short-circuits with the
/// resolver's `Status`. The closure captures only an `Arc`, so it is `Clone`
/// (required for the intercepted service to be `Clone` for `tonic::Server`).
#[cfg(feature = "embedded-worker")]
pub(crate) fn interceptor(
    resolver: Arc<dyn PrincipalResolver>,
) -> impl FnMut(Request<()>) -> Result<Request<()>, Status> + Clone {
    move |mut req: Request<()>| {
        let principal = resolver.resolve(req.metadata())?;
        // Server-derived identity for the gateway handlers (the Invoke path, R2b)
        // — gateway-core reads `CallerParty`, not the host-owned `Principal`.
        req.extensions_mut()
            .insert(kx_gateway_core::CallerParty(principal.party.clone()));
        req.extensions_mut().insert(principal);
        Ok(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_all_rejects_with_unauthenticated() {
        let md = MetadataMap::new();
        let err = DenyAll.resolve(&md).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn dev_allow_local_attributes_a_local_principal() {
        let md = MetadataMap::new();
        let principal = DevAllowLocal.resolve(&md).unwrap();
        assert_eq!(principal.subject, "local-dev");
        assert_eq!(principal.party, "local-dev");
    }

    fn md_with_auth(value: &str) -> MetadataMap {
        let mut md = MetadataMap::new();
        md.insert("authorization", value.parse().unwrap());
        md
    }

    fn token_resolver() -> TokenResolver {
        let mut tokens = HashMap::new();
        tokens.insert("s3cr3t".to_string(), "alice@acme".to_string());
        TokenResolver::new(tokens)
    }

    #[test]
    fn token_resolver_maps_valid_bearer_to_party() {
        let p = token_resolver()
            .resolve(&md_with_auth("Bearer s3cr3t"))
            .unwrap();
        assert_eq!(p.party, "alice@acme");
        assert_eq!(p.subject, "alice@acme");
    }

    #[test]
    fn token_resolver_rejects_missing_authorization() {
        let err = token_resolver().resolve(&MetadataMap::new()).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn token_resolver_rejects_non_bearer_scheme() {
        let err = token_resolver()
            .resolve(&md_with_auth("Basic s3cr3t"))
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn token_resolver_unknown_token_is_indistinguishable_from_missing() {
        // No "valid-format-but-unknown" oracle: the unknown-token error must be
        // byte-identical (code + message) to the missing-header one.
        let unknown = token_resolver()
            .resolve(&md_with_auth("Bearer nope"))
            .unwrap_err();
        let missing = token_resolver().resolve(&MetadataMap::new()).unwrap_err();
        assert_eq!(unknown.code(), tonic::Code::Unauthenticated);
        assert_eq!(unknown.code(), missing.code());
        assert_eq!(unknown.message(), missing.message());
    }

    #[test]
    fn empty_token_resolver_is_empty() {
        assert!(TokenResolver::default().is_empty());
        assert!(!token_resolver().is_empty());
    }

    #[cfg(feature = "embedded-worker")]
    #[test]
    fn interceptor_denies_then_allows_by_resolver() {
        use kx_gateway_core::CallerParty;

        // Deny resolver short-circuits.
        let mut deny = interceptor(Arc::new(DenyAll));
        assert!(deny(Request::new(())).is_err());

        // Allow resolver forwards the request with BOTH the host Principal and the
        // gateway-core CallerParty stashed (server-derived identity for handlers).
        let mut allow = interceptor(Arc::new(DevAllowLocal));
        let out = allow(Request::new(())).unwrap();
        assert_eq!(
            out.extensions()
                .get::<Principal>()
                .map(|p| p.subject.as_str()),
            Some("local-dev"),
        );
        assert_eq!(
            out.extensions().get::<CallerParty>().map(|p| p.0.as_str()),
            Some("local-dev"),
        );
    }

    #[cfg(feature = "embedded-worker")]
    #[test]
    fn interceptor_stashes_token_resolved_party() {
        use kx_gateway_core::CallerParty;

        let mut intercept = interceptor(Arc::new(token_resolver()));
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer s3cr3t".parse().unwrap());
        let out = intercept(req).unwrap();
        assert_eq!(
            out.extensions().get::<CallerParty>().map(|p| p.0.as_str()),
            Some("alice@acme"),
        );
    }
}
