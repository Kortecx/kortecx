//! The demo's deterministic capability broker.
//!
//! The runtime's WORLD-MUTATING / READ-ONLY-NONDET path dispatches effects
//! through a [`CapabilityBroker`]. For the reproducible kill-and-replay proof
//! the broker must be **deterministic**: the same Mote always stages the same
//! response bytes, so the committed `result_ref` (= the staged ref, by
//! content-addressing) is byte-identical across runs, processes, and machines.
//!
//! [`DemoBroker`] also carries the scenario-1 crash injection: when dispatching
//! the designated `StageThenCommit` Mote with [`CrashPoint::PreCommitStc`], it
//! stages the effect (modelling the external side-effect happening) and then
//! aborts — leaving `Proposed` + `EffectStaged` in the journal but no
//! `Committed`, exactly the window recovery must survive.

use std::collections::BTreeMap;
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::ContentStore;
use kx_mote::{Mote, MoteId, ToolName, ToolVersion};
use kx_warrant::WarrantSpec;

use crate::crash::CrashPoint;

/// The pinned capability version every demo dispatch reports.
const DEMO_CAPABILITY_VERSION: &str = "demo-0.1.0";

/// A deterministic in-process broker for the demo workflow.
pub struct DemoBroker<S: ContentStore> {
    store: Arc<S>,
    /// Explicit per-Mote response payloads (e.g. a shaper's `TopologyDecision`
    /// bytes). Motes not listed get a deterministic default payload.
    responses: BTreeMap<MoteId, Vec<u8>>,
    /// Optional crash injection (scenario 1).
    crash_at: Option<CrashPoint>,
    /// The `StageThenCommit` Mote to crash on under [`CrashPoint::PreCommitStc`].
    stc_crash_target: Option<MoteId>,
}

impl<S: ContentStore> std::fmt::Debug for DemoBroker<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DemoBroker")
            .field("responses", &self.responses.len())
            .field("crash_at", &self.crash_at)
            .field("stc_crash_target", &self.stc_crash_target)
            .finish_non_exhaustive()
    }
}

impl<S: ContentStore> DemoBroker<S> {
    /// Build a broker over `store` with explicit per-Mote `responses`.
    #[must_use]
    pub fn new(
        store: Arc<S>,
        responses: BTreeMap<MoteId, Vec<u8>>,
        crash_at: Option<CrashPoint>,
        stc_crash_target: Option<MoteId>,
    ) -> Self {
        Self {
            store,
            responses,
            crash_at,
            stc_crash_target,
        }
    }

    /// The deterministic default response bytes for a Mote with no explicit
    /// entry: a stable tag bound to the Mote's identity, so two runs of the
    /// same Mote stage byte-identical payloads (→ identical `result_ref`).
    fn default_response(mote_id: &MoteId) -> Vec<u8> {
        let mut bytes = b"kx-demo-effect:".to_vec();
        bytes.extend_from_slice(mote_id.as_bytes());
        bytes
    }
}

impl<S: ContentStore + Send + Sync> CapabilityBroker for DemoBroker<S> {
    fn dispatch(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        let bytes = self
            .responses
            .get(&mote.id)
            .cloned()
            .unwrap_or_else(|| Self::default_response(&mote.id));

        // The external effect "happens" here — its payload is staged in the
        // content store. Content-addressing means a re-dispatch on recovery
        // stages byte-identical bytes → the same ref (dedup), so the effect is
        // observed exactly once.
        let staged_ref = self
            .store
            .put(&bytes)
            .map_err(|e| BrokerError::StageWriteFailed {
                capability: capability.clone(),
                diagnostic: format!("{e}"),
            })?;

        // Scenario-1 injection: abort AFTER staging (effect happened, and
        // `EffectStaged` is already in the journal because StageThenCommit
        // writes it before calling dispatch) but BEFORE the commit protocol
        // appends `Committed`.
        if self.crash_at == Some(CrashPoint::PreCommitStc) && self.stc_crash_target == Some(mote.id)
        {
            CrashPoint::PreCommitStc.abort_now();
        }

        Ok(BrokerHandle {
            staged_ref,
            capability: capability.clone(),
            capability_version: ToolVersion(DEMO_CAPABILITY_VERSION.into()),
        })
    }

    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        // The demo broker does not implement effect read-back; recovery relies
        // on the deterministic idempotency-key dedup at re-dispatch.
        Ok(None)
    }
}
