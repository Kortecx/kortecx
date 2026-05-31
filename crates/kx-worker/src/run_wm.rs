//! Dispatching a leased WORLD-MUTATING / READ-ONLY-NONDET Mote (P3.6b, D58).
//!
//! The worker is **not** the journal writer (D40), so it cannot use the
//! single-node lifecycle ([`kx_executor::run_wm_mote`] / `StandardCommitProtocol`),
//! which appends `EffectStaged` + `Committed` to a journal it owns. Instead the
//! worker drives the **same stage→fire→commit ordering** with the journal appends
//! replaced by RPCs (D58 §4):
//!
//! 1. resolve the capability + build the [`EffectRequest`] (mirroring the
//!    single-node `engine::effect_request_for`);
//! 2. for `StageThenCommit`: `ReportEffectStaged` → **await the ack BEFORE firing**
//!    (the coordinator records the durable intent; firing first is the double-effect
//!    hazard, D58 §2). `IdempotentByConstruction` + `ValidateThenCommit` dispatch
//!    directly — mirroring the frozen `StandardCommitProtocol` per-pattern (VTC's
//!    safety is the sibling critic, not an `EffectStaged` hint);
//! 3. `broker.dispatch` fires the effect and stages its response bytes into the
//!    **shared** content store (D55 data plane), returning `staged_ref`;
//! 4. the caller PROPOSES `staged_ref` as the `result_ref` via `ReportCommit`
//!    (the coordinator's D55 phantom-ref guard verifies it is in the store).
//!
//! The worker does **not** schedule the VTC critic (D58 §6): distributed, the critic
//! is an ordinary DAG Mote that becomes ready once the producer commits, and the
//! coordinator's `ready_set` leases it — the worker has no scheduler authority. This
//! is glue over the broker trait, not an engine fork: `kx-executor` source is
//! untouched (the P2 thesis test holds).

use kx_capability::{
    idempotency_token_for, run_scoped_token, CapabilityBroker, EffectRequest, INSTANCE_ID_LEN,
};
use kx_content::ContentRef;
use kx_mote::{EffectPattern, Mote, ToolName};
use kx_warrant::{FsScope, NetScope, WarrantSpec};

use crate::client::WorkerClient;
use crate::error::WorkerError;

/// Drive stage→fire for a non-PURE Mote and return the staged `result_ref` to
/// PROPOSE via `ReportCommit`. Async because `ReportEffectStaged` is an RPC; the
/// broker's `dispatch` is the trait's synchronous method.
///
/// `instance_id` is the registered run (M1.2/D64): when `Some`, the cross-boundary
/// idempotency token is run-scoped (`run_scoped_token`), so the same Mote in a
/// different run fires a distinct effect; when `None` (unregistered run), it falls
/// back to the MoteId-only token.
pub(crate) async fn run_wm(
    client: &mut WorkerClient,
    broker: &dyn CapabilityBroker,
    mote: &Mote,
    warrant: &WarrantSpec,
    worker_id: u64,
    instance_id: Option<[u8; INSTANCE_ID_LEN]>,
) -> Result<ContentRef, WorkerError> {
    let capability = resolve_capability(mote)?;
    let request = effect_request_for(mote, instance_id);

    // Stage the intent durably BEFORE firing (StageThenCommit only). Await the ack:
    // `report_effect_staged` returns `Err(EffectStagedRejected)` if the coordinator
    // declines, and `?` aborts before any `broker.dispatch` — never fire unstaged.
    if effect_staged_required(mote.effect_pattern()) {
        let id = *mote.id.as_bytes();
        client.report_effect_staged(id, id, worker_id).await?;
    }

    let handle = broker.dispatch(mote, warrant, &capability, request)?;
    Ok(handle.staged_ref)
}

/// Resolve the single capability a non-PURE Mote dispatches under, from its
/// `tool_contract`. v0.1 expects exactly one entry and picks the first (a
/// `BTreeMap` iterates in a deterministic key order). The broker's own `precheck`
/// re-verifies the pick against `tool_contract` + `warrant.tool_grants`, so a wrong
/// pick fails loud at `dispatch` (mapped to [`WorkerError::Dispatch`]).
fn resolve_capability(mote: &Mote) -> Result<ToolName, WorkerError> {
    mote.def
        .tool_contract
        .keys()
        .next()
        .cloned()
        .ok_or(WorkerError::CapabilityResolution(mote.id))
}

/// Build the [`EffectRequest`] for a non-PURE Mote — mirrors the single-node
/// `engine::effect_request_for` (empty payload, pattern from the def, empty scopes).
///
/// The worker sets the 32-byte tool-boundary key (D38 §1) that makes a re-dispatch
/// after worker death a no-op at the world boundary (exactly-once, D58 §7), and
/// that token-class WM tools require (executor predicate R-10). M1.2/D64: when the
/// run is registered (`instance_id = Some`), the key is **run-scoped**
/// (`run_scoped_token`) so the same Mote in a *fresh* run fires a *distinct*
/// effect; an unregistered run falls back to the MoteId-only token. Harmless for
/// non-token capabilities.
fn effect_request_for(mote: &Mote, instance_id: Option<[u8; INSTANCE_ID_LEN]>) -> EffectRequest {
    let idempotency_key = match instance_id {
        Some(id) => run_scoped_token(&id, mote),
        None => idempotency_token_for(mote),
    };
    EffectRequest {
        payload: Vec::new(),
        pattern: mote.effect_pattern(),
        idempotency_key: Some(idempotency_key),
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

/// Whether this pattern requires the `ReportEffectStaged` RPC before firing — the
/// single decision point for the per-pattern split (D58 §4 vs the frozen
/// `StandardCommitProtocol`). `StageThenCommit` pre-records the dispatch intent;
/// `IdempotentByConstruction` (remote-API dedupe) and `ValidateThenCommit` (gated by
/// the sibling critic) dispatch directly, matching `commit_validate_then_commit` /
/// `commit_idempotent`. Flip this one line if D58 is amended to stage VTC too.
fn effect_staged_required(pattern: EffectPattern) -> bool {
    matches!(pattern, EffectPattern::StageThenCommit)
}
