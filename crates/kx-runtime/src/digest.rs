//! Deterministic projection digest — the cross-process / cross-machine
//! comparison surface for the kill-and-replay exit-gate assertions.
//!
//! The digest is BLAKE3 over the committed-result set: for every
//! Committed-and-not-Repudiated Mote, `mote_id ‖ result_ref ‖ nd_class`,
//! emitted in `MoteId` order. Two projections with byte-identical committed
//! results produce the same digest regardless of the order entries were
//! folded or the process / machine that folded them. This is exactly the
//! `01-build-sequence.md` §1.13 assertion-(a)/(c) surface ("compared via
//! content hashes across the projection").

use kx_journal::Journal;
use kx_projection::{MoteState, Projection};

use crate::error::RuntimeError;

/// A 32-byte digest of a projection's committed-result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionDigest(pub [u8; 32]);

impl ProjectionDigest {
    /// Lowercase 64-char hex.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

/// Compute the digest of a projection's committed-result set.
#[must_use]
pub fn digest_projection(projection: &Projection) -> ProjectionDigest {
    // Collect committed facts, then sort by MoteId so fold order cannot
    // affect the digest.
    let mut facts: Vec<([u8; 32], [u8; 32], u8)> = Vec::new();
    for (mote_id, state) in projection.iter_motes() {
        if state != MoteState::Committed {
            continue;
        }
        let (Some(result_ref), Some(nd)) = (
            projection.result_ref_of(&mote_id),
            projection.nondeterminism_of(&mote_id),
        ) else {
            continue;
        };
        facts.push((*mote_id.as_bytes(), *result_ref.as_bytes(), nd.as_u8()));
    }
    facts.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    for (mote_id, result_ref, nd) in facts {
        hasher.update(&mote_id);
        hasher.update(&result_ref);
        hasher.update(&[nd]);
    }
    ProjectionDigest(*hasher.finalize().as_bytes())
}

/// Fold a journal into a fresh projection and digest it. This is the
/// "different machine replays to a bit-identical projection" path — a fresh
/// process that has only the journal file reconstructs the same digest.
pub fn digest_journal<J: Journal>(journal: &J) -> Result<ProjectionDigest, RuntimeError> {
    let projection = Projection::from_journal(journal)?;
    Ok(digest_projection(&projection))
}
