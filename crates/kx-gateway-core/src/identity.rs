//! The server-derived caller identity the host's auth interceptor stashes in a
//! request's extensions for the gateway handlers to read.
//!
//! gateway-core owns this type because it depends on neither `kx-gateway`'s
//! `Principal` nor `kx-catalog`'s `PartyId` (the dependency wall). The host
//! authenticates the caller (R2: a bearer token), derives the party, and inserts
//! a [`CallerParty`] into the [`tonic::Request`] extensions; handlers that act on
//! behalf of a party (the `Invoke` path, R2b) read it back. The client NEVER
//! supplies it — identity is server-derived (SN-8 / D70).

/// A resolved caller party, as an opaque handle string.
///
/// Held as a plain `String` so gateway-core stays off `kx-catalog`; the host
/// re-wraps it into a `kx_catalog::PartyId` when resolving authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerParty(pub String);

/// Present ONLY when the caller authenticated with a hosted-app scoped token — a served page
/// talking back to the runtime, never the operator.
///
/// Its presence is the whole signal: an operator (dev-local or bearer) request never carries
/// one, so a handler that requires it refuses every non-page caller, and a handler that reads
/// it knows the request is a page acting within its declared reach. Held here, beside
/// [`CallerParty`], so the RPC handlers can enforce the boundary without depending on the
/// host's token store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerAppScope {
    /// The hosted app's own handle (audit).
    pub app_handle: String,
    /// The App handles this page may run — its envelope's `references.apps`, resolved at mint.
    pub runnable: Vec<String>,
}

impl CallerAppScope {
    /// Whether this page may run `handle`. Exact match — no prefix, no pattern.
    #[must_use]
    pub fn may_run(&self, handle: &str) -> bool {
        self.runnable.iter().any(|h| h == handle)
    }
}
