//! Property tests on the PR 9b-4 `StandardCommitProtocol`
//! `StageThenCommit` path. SN-4 v2 mandate: ≥3 proptest properties × 64
//! cases.
//!
//! Properties:
//! 1. Stage-then-commit ordering — for any input, the journal's first
//!    EffectStaged entry's seq strictly precedes the Committed entry's
//!    seq.
//! 2. EffectStaged persistence on broker failure — when the broker
//!    refuses dispatch, the EffectStaged entry remains in the journal
//!    (the recovery hint is durably recorded).
//! 3. EffectStaged + Committed share idempotency_key — under the v2 dedup
//!    index `{1, 2, 4}`, both kinds participate but on different keys
//!    (distinct kinds → both land).
//! 4. R-11 fires AFTER EffectStaged in StageThenCommit, leaving exactly
//!    one entry in the journal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_executor::{CommitInput, CommitProtocol, CommitProtocolError, StandardCommitProtocol};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteDefHash,
    NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use proptest::prelude::*;
use smallvec::SmallVec;

fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

fn wm_stc_mote(seed: u8) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

fn empty_request() -> EffectRequest {
    EffectRequest {
        payload: Vec::new(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

struct HappyBroker {
    store: Arc<InMemoryContentStore>,
    response_bytes: Vec<u8>,
}
impl std::fmt::Debug for HappyBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HappyBroker").finish()
    }
}
impl CapabilityBroker for HappyBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        let r = self.store.put(&self.response_bytes).expect("put");
        Ok(BrokerHandle {
            staged_ref: r,
            capability: ToolName("happy-stc".into()),
            capability_version: ToolVersion("0.1.0".into()),
        })
    }
    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

struct FailingBroker;
impl std::fmt::Debug for FailingBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailingBroker").finish()
    }
}
impl CapabilityBroker for FailingBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        Err(BrokerError::SandboxRefused {
            capability: ToolName("failing".into()),
            reason: "test failure".into(),
        })
    }
    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

struct HostileBroker {
    fake_ref: ContentRef,
}
impl std::fmt::Debug for HostileBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostileBroker").finish()
    }
}
impl CapabilityBroker for HostileBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        Ok(BrokerHandle {
            staged_ref: self.fake_ref,
            capability: ToolName("hostile".into()),
            capability_version: ToolVersion("0".into()),
        })
    }
    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

fn input_for<'a>(
    mote: &'a Mote,
    warrant: &'a WarrantSpec,
    idempotency_key: [u8; 32],
) -> CommitInput<'a> {
    CommitInput {
        mote,
        warrant,
        capability: ToolName("test".into()),
        effect_request: empty_request(),
        warrant_ref: ContentRef::from_bytes([4; 32]),
        mote_def_hash: MoteDefHash::from_bytes([3; 32]),
        idempotency_key,
        parents: SmallVec::new(),
        diagnostic_context: "proptest",
    }
}

proptest! {
    /// Order invariant: in the StageThenCommit happy path, the journal's
    /// EffectStaged entry seq strictly precedes the Committed entry seq.
    #[test]
    fn prop_stage_then_commit_ordering(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
        response in any::<Vec<u8>>().prop_filter("non-empty", |v| !v.is_empty()),
    ) {
        let mote = wm_stc_mote(seed);
        let w = warrant();
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HappyBroker {
            store: store.clone(),
            response_bytes: response.clone(),
        });
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let _ = protocol.commit(input_for(&mote, &w, idempotency_key))
            .expect("happy-path commit must succeed");

        let entries: Vec<JournalEntry> = journal
            .read_entries_by_seq(0..u64::MAX)
            .expect("scan").collect();
        prop_assert_eq!(entries.len(), 2);
        let staged_seq = match &entries[0] {
            JournalEntry::EffectStaged { seq, .. } => *seq,
            other => { prop_assert!(false, "first must be EffectStaged, got {:?}", other); 0 }
        };
        let committed_seq = match &entries[1] {
            JournalEntry::Committed { seq, .. } => *seq,
            other => { prop_assert!(false, "second must be Committed, got {:?}", other); 0 }
        };
        prop_assert!(staged_seq < committed_seq, "EffectStaged seq must precede Committed seq");
    }

    /// EffectStaged persistence: when the broker fails, the journal
    /// contains exactly one EffectStaged entry (no Committed entry).
    #[test]
    fn prop_effect_staged_persists_on_broker_failure(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
    ) {
        let mote = wm_stc_mote(seed);
        let w = warrant();
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(FailingBroker);
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let err = protocol.commit(input_for(&mote, &w, idempotency_key))
            .expect_err("broker failure must propagate");
        let is_dispatch_failed = matches!(err, CommitProtocolError::BrokerDispatchFailed { .. });
        prop_assert!(is_dispatch_failed);

        let entries: Vec<JournalEntry> = journal
            .read_entries_by_seq(0..u64::MAX)
            .expect("scan").collect();
        prop_assert_eq!(entries.len(), 1);
        let first_is_staged = matches!(entries[0], JournalEntry::EffectStaged { .. });
        prop_assert!(first_is_staged);
        prop_assert!(journal.read_committed(&mote.id).unwrap().is_none());
    }

    /// R-11 leaves the EffectStaged entry in place (and exactly that one
    /// entry); the Committed entry never lands.
    #[test]
    fn prop_r11_in_stc_path_leaves_exactly_effect_staged(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
        fake_ref_bytes in any::<[u8; 32]>(),
    ) {
        let mote = wm_stc_mote(seed);
        let w = warrant();
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HostileBroker {
            fake_ref: ContentRef::from_bytes(fake_ref_bytes),
        });
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let err = protocol.commit(input_for(&mote, &w, idempotency_key))
            .expect_err("R-11 must fire");
        let is_r11 = matches!(err, CommitProtocolError::R11ResultRefIncomplete { .. });
        prop_assert!(is_r11);

        let entries: Vec<JournalEntry> = journal
            .read_entries_by_seq(0..u64::MAX)
            .expect("scan").collect();
        prop_assert_eq!(entries.len(), 1);
        let first_is_staged = matches!(entries[0], JournalEntry::EffectStaged { .. });
        prop_assert!(first_is_staged);
    }

    /// EffectStaged and Committed entries share the same idempotency_key.
    /// Under the v2 dedup index {1, 2, 4} they are distinct kinds, so
    /// both land — but they share the per-Mote identity key.
    #[test]
    fn prop_effect_staged_and_committed_share_idempotency_key(
        seed in any::<u8>(),
        idempotency_key in any::<[u8; 32]>(),
    ) {
        let mote = wm_stc_mote(seed);
        let w = warrant();
        let store = Arc::new(InMemoryContentStore::new());
        let journal = Arc::new(InMemoryJournal::new());
        let broker = Arc::new(HappyBroker {
            store: store.clone(),
            response_bytes: b"key-share".to_vec(),
        });
        let protocol = StandardCommitProtocol::new(store, journal.clone(), broker);

        let _ = protocol.commit(input_for(&mote, &w, idempotency_key))
            .expect("commit must succeed");

        let entries: Vec<JournalEntry> = journal
            .read_entries_by_seq(0..u64::MAX)
            .expect("scan").collect();
        prop_assert_eq!(entries.len(), 2);
        let staged_key = match &entries[0] {
            JournalEntry::EffectStaged { idempotency_key, .. } => *idempotency_key,
            _ => { prop_assert!(false, "first must be EffectStaged"); [0u8; 32] }
        };
        let committed_key = match &entries[1] {
            JournalEntry::Committed { idempotency_key, .. } => *idempotency_key,
            _ => { prop_assert!(false, "second must be Committed"); [0u8; 32] }
        };
        prop_assert_eq!(staged_key, committed_key);
        prop_assert_eq!(staged_key, idempotency_key);
    }
}
