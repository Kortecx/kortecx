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
use kx_warrant::{FsScope, NetScope, ToolGrant, WarrantSpec};

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
///
/// `tool_args` (PR-2d-2, react-tools-live) is the coordinator-VALIDATED
/// `(args_bytes, net_scope)` pair from `WorkItem.tool_args` — the model-proposed
/// args for a ReAct observation, decoded + schema-checked ONCE on the sole
/// writer and re-derived at every lease (never decoded here — one decode site).
/// `None` for every legacy WM Mote ⇒ the empty-payload request is byte-identical
/// to pre-PR-2d-2. **Fail-closed**: a Mote whose contract tool IS granted by its
/// warrant and whose pattern is `StageThenCommit` (the observation shape) but
/// that carries NO args is REFUSED (`MissingToolArgs`, terminal) — the worker
/// never fires a granted tool with an empty payload.
pub(crate) async fn run_wm(
    client: &mut WorkerClient,
    broker: &dyn CapabilityBroker,
    mote: &Mote,
    warrant: &WarrantSpec,
    worker_id: u64,
    instance_id: Option<[u8; INSTANCE_ID_LEN]>,
    tool_args: Option<(Vec<u8>, NetScope)>,
) -> Result<ContentRef, WorkerError> {
    let capability = resolve_capability(mote)?;
    if tool_args.is_none() && requires_tool_args(mote, warrant, &capability) {
        return Err(WorkerError::MissingToolArgs(mote.id));
    }
    let request = effect_request_for(mote, instance_id, tool_args);

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

/// PR-2d-2: whether this Mote is the SHAPE that must carry coordinator-validated
/// args — its contract tool is GRANTED by its warrant (the ReAct observation is
/// serve's only granted-tool Mote) and its pattern is `StageThenCommit` (the MCP
/// default, D66). Every legacy WM path has empty `tool_grants`, so this is
/// `false` there and nothing changes.
fn requires_tool_args(mote: &Mote, warrant: &WarrantSpec, capability: &ToolName) -> bool {
    if mote.effect_pattern() != EffectPattern::StageThenCommit {
        return false;
    }
    mote.def
        .tool_contract
        .get(capability)
        .is_some_and(|version| {
            warrant.tool_grants.contains(&ToolGrant {
                tool_id: capability.clone(),
                tool_version: version.clone(),
            })
        })
}

/// Build the [`EffectRequest`] for a non-PURE Mote — mirrors the single-node
/// `engine::effect_request_for` (pattern from the def, empty scopes), PLUS the
/// PR-2d-2 args path: a ReAct observation's coordinator-validated
/// `(args_bytes, net_scope)` becomes the request's payload + egress (the harness
/// `dispatch_decoded_call` recipe — the broker's `precheck` still enforces
/// `net_scope ⊆ warrant` at dispatch). `tool_args = None` (every legacy Mote)
/// keeps the empty payload / `NetScope::None` request byte-identical.
///
/// The worker sets the 32-byte tool-boundary key (D38 §1) that makes a re-dispatch
/// after worker death a no-op at the world boundary (exactly-once, D58 §7), and
/// that token-class WM tools require (executor predicate R-10). M1.2/D64: when the
/// run is registered (`instance_id = Some`), the key is **run-scoped**
/// (`run_scoped_token`) so the same Mote in a *fresh* run fires a *distinct*
/// effect; an unregistered run falls back to the MoteId-only token. Harmless for
/// non-token capabilities. The token is DELIBERATELY args-free (mote + instance
/// only, D58 §7): a crash-recovery re-dispatch of the SAME observation re-derives
/// the SAME args (a pure function of committed facts), so the dedup is sound.
fn effect_request_for(
    mote: &Mote,
    instance_id: Option<[u8; INSTANCE_ID_LEN]>,
    tool_args: Option<(Vec<u8>, NetScope)>,
) -> EffectRequest {
    let idempotency_key = match instance_id {
        Some(id) => run_scoped_token(&id, mote),
        None => idempotency_token_for(mote),
    };
    let (payload, net_scope) = tool_args.unwrap_or((Vec::new(), NetScope::None));
    EffectRequest {
        payload,
        pattern: mote.effect_pattern(),
        idempotency_key: Some(idempotency_key),
        net_scope,
        fs_scope: FsScope::empty(),
        secret_scope: kx_warrant::SecretScope::None,
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kx_mote::{
        GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, MoteDef, NdClass,
        PromptTemplateHash, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_warrant::Host;
    use smallvec::SmallVec;

    use super::*;

    fn tool() -> (ToolName, ToolVersion) {
        (ToolName("mcp-echo".into()), ToolVersion("1".into()))
    }

    /// A WM `StageThenCommit` Mote declaring `mcp-echo@1` (the observation shape).
    fn obs_mote() -> Mote {
        let (name, version) = tool();
        let mut tool_contract = BTreeMap::new();
        tool_contract.insert(name, version);
        let def = MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([9; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([9; 32]),
            tool_contract,
            nd_class: NdClass::WorldMutating,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([9; 32]),
            GraphPosition(vec![9]),
            SmallVec::new(),
        )
    }

    fn warrant(granted: bool) -> WarrantSpec {
        let mut tool_grants = BTreeSet::new();
        if granted {
            let (tool_id, tool_version) = tool();
            tool_grants.insert(ToolGrant {
                tool_id,
                tool_version,
            });
        }
        WarrantSpec {
            tool_grants,
            ..WarrantSpec::default()
        }
    }

    /// The PR-2d-2 fail-closed predicate: a GRANTED StageThenCommit tool Mote
    /// (the ReAct observation shape) REQUIRES coordinator-validated args; every
    /// legacy WM Mote (empty grants) does not — its request is byte-unchanged.
    #[test]
    fn granted_stc_tool_requires_args_legacy_does_not() {
        let mote = obs_mote();
        let (name, _) = tool();
        assert!(requires_tool_args(&mote, &warrant(true), &name));
        assert!(
            !requires_tool_args(&mote, &warrant(false), &name),
            "empty grants (every legacy WM path) never require args"
        );
        // A granted tool under a DIFFERENT contract version is NOT the
        // observation shape (exact-equality grants, SN-8).
        let mut other = warrant(false);
        other.tool_grants.insert(ToolGrant {
            tool_id: ToolName("mcp-echo".into()),
            tool_version: ToolVersion("2".into()),
        });
        assert!(!requires_tool_args(&mote, &other, &name));
    }

    /// The args pair becomes the request's payload + egress; `None` keeps the
    /// legacy empty-payload / no-egress request byte-identical.
    #[test]
    fn effect_request_consumes_args_or_stays_legacy() {
        let mote = obs_mote();
        let legacy = effect_request_for(&mote, Some([3; INSTANCE_ID_LEN]), None);
        assert!(legacy.payload.is_empty());
        assert_eq!(legacy.net_scope, NetScope::None);

        let scope = NetScope::EgressAllowlist([Host("example.com".into())].into_iter().collect());
        let with_args = effect_request_for(
            &mote,
            Some([3; INSTANCE_ID_LEN]),
            Some((br#"{"q":"x"}"#.to_vec(), scope.clone())),
        );
        assert_eq!(with_args.payload, br#"{"q":"x"}"#.to_vec());
        assert_eq!(with_args.net_scope, scope);
        // The idempotency token is args-FREE (mote + instance only, D58 §7):
        // identical across the two requests.
        assert_eq!(with_args.idempotency_key, legacy.idempotency_key);
    }
}
