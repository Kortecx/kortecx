//! The uploads audit/scope seam behind `PutContent` + the EMPTY-`instance_id`
//! ("uploads scope") reads on `GetContent`/`GetContentBatch`.
//!
//! Spoken in gateway-core's own wire vocabulary (`[u8; 32]` / `String` / `u64`)
//! — no host type crosses the seam (the [`crate::CaptureView`] precedent). The
//! host (`kx-gateway`) implements it over an `uploads.db` SQLite sidecar.
//!
//! # Boundaries (load-bearing)
//!
//! - **Off the truth path.** The sidecar is REBUILDABLE-TO-EMPTY audit state:
//!   truth (the blobs) lives in the content store; losing the sidecar loses
//!   only the uploads-scope authorization index + advisory audit rows, and
//!   re-uploading the same bytes restores authorization at the same ref
//!   (content-addressed). Never journaled, never a `MoteId` input, never a
//!   digest input.
//! - **Advisory metadata only.** `media_type`/`filename` are display/audit
//!   fields; identity is the server-derived blake3 ref alone (SN-8).
//! - **`None` seam ⇒ degrade.** A host without the sidecar leaves `PutContent`
//!   `unimplemented` and the uploads scope uniformly not-authorized.

use crate::error::GatewayError;

/// One recorded client upload — the advisory audit row + the ref that joins
/// the uploads authorized set.
#[derive(Clone, Debug)]
pub struct UploadRecord {
    /// The server-derived blake3 ref of the stored payload (the join key into
    /// the content store; the ONLY identity-bearing field).
    pub content_ref: [u8; 32],
    /// Advisory mime as supplied by the caller (display/audit only).
    pub media_type: String,
    /// Advisory display name as supplied by the caller (display/audit only).
    pub filename: String,
    /// The SERVER-RESOLVED caller party (from the auth interceptor — never the
    /// wire request).
    pub principal: String,
    /// Wall-clock upload time in unix ms (audit only — never identity).
    pub uploaded_ms: u64,
}

/// The uploads ledger seam: record a client upload, and answer "is this ref in
/// the uploads scope?" for the empty-`instance_id` read path. A `None` seam on
/// the service ⇒ `PutContent` returns `unimplemented` and uploads-scope reads
/// uniformly deny.
pub trait UploadsLedger: Send + Sync {
    /// Durably record an upload (idempotent on `content_ref` — a re-upload of
    /// identical bytes refreshes the advisory metadata).
    ///
    /// # Errors
    /// A host write failure ([`GatewayError::Internal`]).
    fn record(&self, rec: UploadRecord) -> Result<(), GatewayError>;

    /// `true` iff `content_ref` was recorded as an upload (the uploads-scope
    /// authorized-set membership test).
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn contains(&self, content_ref: &[u8; 32]) -> Result<bool, GatewayError>;
}
