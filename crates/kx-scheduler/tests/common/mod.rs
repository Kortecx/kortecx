//! Shared fixtures across the kx-scheduler integration tests.
//!
//! Provides a [`MockExecutor`] (records dispatched MoteIds, returns
//! deterministic results), warrant + Mote builders, and helpers to
//! synthesize `Committed` / `Failed` journal entries for the test harness
//! to fold into a [`Projection`].
//!
//! The fixture deliberately keeps the test code itself responsible for
//! constructing and folding journal entries — the scheduler under test
//! never sees a `Journal` handle (the production crate doesn't depend on
//! `kx-journal`), so the test layer plays the role the executor's
//! lifecycle layer plays in production.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use kx_content::ContentRef;
use kx_executor::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use kx_journal::{FailureReason, JournalEntry, ParentEntry};
use kx_mote::{
    EdgeKind, EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteDefHash, MoteId, NdClass, ParentRef, PromptTemplateHash,
};
use kx_projection::Projection;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

/// A test executor that records every dispatched MoteId and returns a
/// deterministic `result_ref` derived from `mote.id`. Does NOT actually
/// execute anything; serves as the seam-respecting dispatch target.
///
/// The executor's outputs are owned by the executor — the scheduler never
/// asks this struct for a result_ref or a parent lookup; it only passes
/// the executor's verbatim outcome through `DispatchedMote.result`.
#[derive(Default)]
pub(crate) struct MockExecutor {
    pub(crate) calls: Mutex<Vec<MoteId>>,
}

impl MoteExecutor for MockExecutor {
    fn run(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        self.calls.lock().unwrap().push(mote.id);
        let result_ref = ContentRef::of(mote.id.as_bytes());
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }

    fn supports(&self, _executor_class: kx_warrant::ExecutorClass) -> bool {
        true
    }
}

impl MockExecutor {
    pub(crate) fn dispatched_ids(&self) -> Vec<MoteId> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

/// Construct a permissive warrant suitable for the scheduler's PURE
/// integration tests. The mock executor doesn't enforce any of these,
/// but real `WarrantSpec` requires every field, so the helper exists to
/// keep the test code tight.
pub(crate) fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("test".into()),
            max_input_tokens: 1000,
            max_output_tokens: 1000,
            max_calls: 1,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 28,
            wall_clock_ms: 5000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

/// Construct a PURE Mote with the given `position` and `parents`. The
/// `input_data_id` is derived from `position` so different positions
/// yield distinct `MoteId`s without ceremony.
pub(crate) fn pure_mote(position: &[u8], parents: SmallVec<[ParentRef; 4]>) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([0u8; 32]),
        model_id: ModelId("test".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: 3,
    };
    let mut input_data = [0u8; 32];
    let len = position.len().min(32);
    input_data[..len].copy_from_slice(&position[..len]);
    Mote::new(
        def,
        InputDataId::from_bytes(input_data),
        GraphPosition(position.to_vec()),
        parents,
    )
}

/// Build a Data-edge [`ParentRef`] pointing at `parent`.
pub(crate) fn data_parent(parent: &Mote) -> ParentRef {
    ParentRef {
        parent_id: parent.id,
        edge: EdgeMeta {
            kind: EdgeKind::Data,
            non_cascade: false,
        },
    }
}

/// Synthesize a `Committed` entry for the given Mote at the given `seq`.
///
/// In production the executor's lifecycle layer builds and appends this;
/// the scheduler's tests do it themselves so the projection has the
/// committed fact to surface through `ready_set()`. The test owns the
/// `result_ref` end-to-end — the scheduler never asks anyone for it.
pub(crate) fn committed_entry(mote: &Mote, seq: u64) -> JournalEntry {
    let parents = mote
        .parents
        .iter()
        .map(ParentEntry::from_parent_ref)
        .collect();
    JournalEntry::Committed {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq,
        nondeterminism: mote.def.nd_class,
        result_ref: ContentRef::of(mote.id.as_bytes()),
        parents,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([0u8; 32]),
    }
}

/// Synthesize a non-terminal (pre-commit-crash) `Failed` entry at the
/// given `seq`. Per `kx_journal::is_pre_commit_crash`, `WorkerCrashed`
/// leaves `terminal_failure_observed` unset — the projection treats the
/// Mote as still-Pending (retry-allowed); ready_set still excludes
/// children because the Mote is not Committed.
pub(crate) fn failed_worker_crashed(mote: &Mote, seq: u64) -> JournalEntry {
    JournalEntry::Failed {
        mote_id: mote.id,
        idempotency_key: *mote.id.as_bytes(),
        seq,
        reason_class: FailureReason::WorkerCrashed,
        reporter_id: 0,
    }
}

/// Fold an entry into the projection, panicking on error (test helper).
pub(crate) fn fold_or_panic(projection: &mut Projection, entry: &JournalEntry) {
    projection
        .fold(entry)
        .expect("test-synthesized journal entry must fold cleanly");
}
