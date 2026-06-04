// `tonic::Status` is a large error type (~176 B), but it is the REQUIRED error
// of the tonic `Interceptor` contract (`Result<Request<()>, Status>`) and of
// `Status`-returning resolvers — it cannot be boxed without an extra conversion
// at every call site. Allow the perf lint here where the type is dictated by
// tonic, not chosen by us.
#![allow(clippy::result_large_err)]

//! The deny-all auth seam (R1 stub; R2 fills it).
//!
//! Rule 8c — *the moment a port is bound there is no OSS auth* — so the freshly
//! bound gateway defaults to [`DenyAll`]: every RPC is rejected with
//! `unauthenticated` until the operator opts into [`DevAllowLocal`] via
//! `--dev-allow-local`. The [`PrincipalResolver`] trait is the fill point: R2
//! swaps in a real token / mTLS resolver WITHOUT changing this trait, the
//! [`Principal`] type (which grows additively), or the interceptor wiring.
//!
//! Identity is **server-derived** from transport metadata (SN-8) — the client
//! never asserts who it is into the snapshot path. gateway-core stays auth-free
//! (its dep-wall + `lib.rs` comment); auth lives here, in the host binary.

use tonic::metadata::MetadataMap;
use tonic::Status;

#[cfg(feature = "embedded-worker")]
use std::sync::Arc;
#[cfg(feature = "embedded-worker")]
use tonic::Request;

/// A resolved caller identity. Opaque in R1 (just a subject label); R2 fills it
/// with real claims. A struct (not an enum matched at call sites) so new fields
/// are additive and never a breaking change.
#[derive(Clone, Debug)]
pub struct Principal {
    /// The caller's subject. `"local-dev"` under [`DevAllowLocal`].
    pub subject: String,
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
    }

    #[cfg(feature = "embedded-worker")]
    #[test]
    fn interceptor_denies_then_allows_by_resolver() {
        // Deny resolver short-circuits.
        let mut deny = interceptor(Arc::new(DenyAll));
        assert!(deny(Request::new(())).is_err());

        // Allow resolver forwards the request with the principal stashed.
        let mut allow = interceptor(Arc::new(DevAllowLocal));
        let out = allow(Request::new(())).unwrap();
        assert_eq!(
            out.extensions()
                .get::<Principal>()
                .map(|p| p.subject.as_str()),
            Some("local-dev"),
        );
    }
}
