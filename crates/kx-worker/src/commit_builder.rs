//! Building the `ReportCommit` proposal for a PURE Mote the worker just ran.

use kx_content::ContentRef;
use kx_mote::Mote;
use kx_proto::proto;
use kx_warrant::{warrant_ref_of, WarrantSpec};

/// Assemble the `ReportCommit` proposal from the held `Mote` + `warrant` and the
/// executor's output `result_ref`.
///
/// Every metadata field is **re-derived from the Mote/warrant** — the same
/// canonical construction the coordinator uses to build the durable `Committed`
/// entry — so the proposal cannot diverge from what the coordinator expects:
/// `nd_class` and `parents` come from the Mote itself (NOT from the throwaway
/// journal, which records `parents = empty` / `nd_class = Pure`). Only
/// `result_ref` is new information from the run.
///
/// `idempotency_key == mote_id` is the identity invariant the coordinator's
/// `commit::assemble` enforces.
pub(crate) fn report_commit_request(
    mote: &Mote,
    warrant: &WarrantSpec,
    result_ref: ContentRef,
    worker_id: u64,
) -> proto::ReportCommitRequest {
    let id = mote.id.as_bytes().to_vec();
    proto::ReportCommitRequest {
        mote_id: id.clone(),
        idempotency_key: id,
        result_ref: result_ref.as_bytes().to_vec(),
        warrant_ref: warrant_ref_of(warrant).as_bytes().to_vec(),
        mote_def_hash: mote.def.hash().as_bytes().to_vec(),
        nd_class: proto::NdClass::from(mote.nd_class()) as i32,
        parents: mote.parents.iter().copied().map(Into::into).collect(),
        worker_id,
    }
}
