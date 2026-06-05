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
