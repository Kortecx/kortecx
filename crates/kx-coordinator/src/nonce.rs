//! [`RunNonceSource`] — the coordinator's source of fresh run instance ids (M1.1, D64).
//!
//! A run's identity is a **registered, journaled, immutable** 16-byte nonce (D64):
//! re-running the same recipe yields a NEW run with a NEW identity. The nonce is
//! generated ONCE at run registration (`RegisterRun`), journaled in the seq=1
//! `RunRegistered` entry, and thereafter **read on replay — never recomputed**.
//!
//! It is behind a trait so tests inject a deterministic nonce (keeping a
//! reproducible test surface) while production draws OS entropy: a run identity
//! must be unpredictable to serve as a cross-boundary idempotency-token root.
//!
//! SN-8 (reframed v2, D64): run identity is a registered nonce, NOT a content
//! hash. The nonce never enters a content-addressed digest; it is identity by
//! registration, not by derivation.

use kx_journal::INSTANCE_ID_LEN;

/// A source of fresh, unguessable run instance ids.
///
/// `Debug` is required so a service that owns a `dyn RunNonceSource` stays
/// `Debug`.
pub trait RunNonceSource: Send + Sync + std::fmt::Debug {
    /// Return a fresh 16-byte run instance id. Production implementations MUST
    /// return an unpredictable value (OS entropy); test doubles may return a
    /// fixed nonce for determinism.
    fn fresh_instance_id(&self) -> [u8; INSTANCE_ID_LEN];
}

/// The production nonce source — 16 bytes of OS entropy via `getrandom`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsRandomNonce;

impl RunNonceSource for OsRandomNonce {
    // The OS entropy source (getrandom) is infallible on every supported target
    // (Linux CI / Apple-Silicon local, SN-7); a failure means the host RNG is
    // unavailable, which is not a recoverable runtime condition. We surface it
    // loudly rather than fabricate a predictable nonce — a guessable run identity
    // would defeat D64's unguessable-identity / idempotency-token-root guarantee.
    #[allow(clippy::expect_used)]
    fn fresh_instance_id(&self) -> [u8; INSTANCE_ID_LEN] {
        let mut buf = [0u8; INSTANCE_ID_LEN];
        getrandom::fill(&mut buf).expect("OS entropy source (getrandom) must be available");
        buf
    }
}
