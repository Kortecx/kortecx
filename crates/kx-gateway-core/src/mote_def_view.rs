//! The Mote-definition read seam behind `GetMoteDetail` (Batch B).
//!
//! Display ONLY (SN-8): nothing resolved here authorizes anything — the def is
//! the coordinator-admitted ground truth persisted content-addressed at
//! admission (its canonical encode's blake3 IS `mote_def_hash`), read back for
//! inspection. The seam speaks `kx_mote::MoteDef` — already in gateway-core's
//! vocabulary via [`crate::RunSubmitter`] (`kx_mote::Mote`), so no new type
//! crosses the dep wall. The host implements it over the SAME content store
//! the coordinator persists into (`kx-gateway`'s `HostMoteDefView`).

use crate::error::GatewayError;

/// The def-resolution read seam. `Ok(None)` is the HONEST miss — a def the
/// store does not hold (a journal predating Batch B, or a persist that
/// best-effort failed at admission); the handler answers `def_found = false`,
/// never an error.
pub trait MoteDefView: Send + Sync {
    /// Resolve `mote_def_hash` (the content address of the canonical def
    /// bytes) to the decoded definition.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]). An absent or
    /// undecodable blob is `Ok(None)`, not an error (fail-honest, not
    /// fail-loud — the blob is display substrate).
    fn get_def(&self, mote_def_hash: &[u8; 32]) -> Result<Option<kx_mote::MoteDef>, GatewayError>;
}
