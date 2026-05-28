//! `ReportCommit` → validated [`CommitProposal`] (the D40 sole-writer path, step 1).
//!
//! This module is **pure**: it converts an untrusted `proto::ReportCommitRequest`
//! into the typed fields needed to build a `JournalEntry::Committed`, validating
//! every 32-byte hash length, rejecting `*_UNSPECIFIED` enums, and enforcing the
//! `idempotency_key == MoteId` identity invariant. It never touches the journal
//! or projection — assembling and appending the entry is the orchestration core's
//! job (`crate::state`), where the seq is assigned.

use kx_content::ContentRef;
use kx_journal::ParentEntry;
use kx_mote::{MoteDefHash, MoteId, NdClass, ParentRef};
use kx_proto::proto;
use kx_proto::ConvertError;
use smallvec::SmallVec;

use crate::error::CoordinatorError;

/// The typed, validated commit proposal extracted from a `ReportCommit` request.
///
/// Every field is already canonical: 32-byte refs are length-checked, the enum is
/// not the `UNSPECIFIED` sentinel, and `idempotency_key == mote_id`.
pub(crate) struct CommitProposal {
    pub(crate) mote_id: MoteId,
    pub(crate) idempotency_key: [u8; 32],
    pub(crate) result_ref: ContentRef,
    pub(crate) warrant_ref: ContentRef,
    pub(crate) mote_def_hash: MoteDefHash,
    pub(crate) nd_class: NdClass,
    pub(crate) parents: SmallVec<[ParentEntry; 4]>,
}

/// Validate that a wire `bytes` field is exactly a 32-byte hash.
fn hash32(bytes: &[u8], field: &'static str) -> Result<[u8; 32], CoordinatorError> {
    <[u8; 32]>::try_from(bytes).map_err(|_| CoordinatorError::BadHashLength {
        field,
        len: bytes.len(),
    })
}

/// Convert + validate a `ReportCommit` request into a [`CommitProposal`].
pub(crate) fn assemble(
    req: proto::ReportCommitRequest,
) -> Result<CommitProposal, CoordinatorError> {
    let mote_id = MoteId::from_bytes(hash32(&req.mote_id, "ReportCommit.mote_id")?);
    let idempotency_key = hash32(&req.idempotency_key, "ReportCommit.idempotency_key")?;
    if &idempotency_key != mote_id.as_bytes() {
        return Err(CoordinatorError::IdentityMismatch(mote_id));
    }
    let result_ref = ContentRef::from_bytes(hash32(&req.result_ref, "ReportCommit.result_ref")?);
    let warrant_ref = ContentRef::from_bytes(hash32(&req.warrant_ref, "ReportCommit.warrant_ref")?);
    let mote_def_hash =
        MoteDefHash::from_bytes(hash32(&req.mote_def_hash, "ReportCommit.mote_def_hash")?);

    let proto_nd =
        proto::NdClass::try_from(req.nd_class).map_err(|_| ConvertError::UnknownEnum {
            enum_name: "NdClass",
            value: req.nd_class,
        })?;
    let nd_class = NdClass::try_from(proto_nd)?;

    let parents: SmallVec<[ParentEntry; 4]> = req
        .parents
        .into_iter()
        .map(|p| ParentRef::try_from(p).map(|pr| ParentEntry::from_parent_ref(&pr)))
        .collect::<Result<_, ConvertError>>()?;

    Ok(CommitProposal {
        mote_id,
        idempotency_key,
        result_ref,
        warrant_ref,
        mote_def_hash,
        nd_class,
        parents,
    })
}
