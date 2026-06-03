//! Native deterministic-critic execution (P4.2-2) integration tests.
//!
//! Proves the runtime can turn "a producer committed some bytes" into "the
//! runtime verified them against a declared, replayable check" — the
//! SN-8-compliant, model-decorrelated trust primitive (D60). A critic Mote
//! reads its producer's committed output, evaluates a `CheckSpec` in-process,
//! and commits a `CriticVerdict` as its own content-addressed `result_ref`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_critic_types::{CheckSpec, CriticVerdict, SchemaSpec, SchemaTag};
use kx_executor::{
    run_native_critic_mote, validate_submission, SubmissionRefusal, WorkflowSubmission,
};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
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
        ..Default::default()
    }
}

fn submission(motes: BTreeMap<MoteId, Mote>) -> WorkflowSubmission {
    WorkflowSubmission {
        run_id: [0u8; 32],
        master_warrant: warrant(),
        motes,
        accept_at_least_once: BTreeMap::new(),
    }
}

/// A PURE producer Mote whose committed bytes the critic will inspect.
fn producer_mote() -> Mote {
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
        inference_params: kx_mote::InferenceParams::default(),
        critic_check: None,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([10; 32]),
        GraphPosition("/producer".into()),
        SmallVec::new(),
    )
}

/// A deterministic-critic Mote validating `producer` with `check`.
fn critic_mote(producer: MoteId, check: CheckSpec) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([3; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([4; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: Some(producer),
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        critic_check: Some(check),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([20; 32]),
        GraphPosition("/critic".into()),
        SmallVec::new(),
    )
}

/// Commit `bytes` as `producer`'s committed result_ref in `journal` + `store`.
fn commit_producer(
    journal: &InMemoryJournal,
    store: &InMemoryContentStore,
    producer: &Mote,
    bytes: &[u8],
) {
    let result_ref = store.put(bytes).unwrap();
    journal
        .append(JournalEntry::Committed {
            mote_id: producer.id,
            idempotency_key: *producer.id.as_bytes(),
            seq: 0,
            nondeterminism: NdClass::WorldMutating,
            result_ref,
            parents: SmallVec::new(),
            warrant_ref: kx_warrant::warrant_ref_of(&warrant()),
            mote_def_hash: producer.def.hash(),
        })
        .unwrap();
}

fn json_check() -> CheckSpec {
    CheckSpec::Schema(SchemaSpec {
        expected: SchemaTag::Json,
    })
}

/// Read the committed verdict for `critic` from the journal + store.
fn committed_verdict(
    journal: &InMemoryJournal,
    store: &InMemoryContentStore,
    critic: &Mote,
) -> CriticVerdict {
    let Some(JournalEntry::Committed { result_ref, .. }) =
        journal.read_committed(&critic.id).unwrap()
    else {
        panic!("critic not committed");
    };
    let bytes = store.get(&result_ref).unwrap();
    CriticVerdict::decode(&bytes).unwrap()
}

#[test]
fn critic_commits_valid_verdict_when_producer_output_conforms() {
    let journal = InMemoryJournal::new();
    let store = InMemoryContentStore::new();
    let producer = producer_mote();
    commit_producer(&journal, &store, &producer, br#"{"ok": true}"#);

    let critic = critic_mote(producer.id, json_check());
    let commit = run_native_critic_mote(&critic, &warrant(), &journal, &store).unwrap();
    assert_eq!(commit.mote_id, critic.id);

    let verdict = committed_verdict(&journal, &store, &critic);
    assert!(verdict.is_valid(), "valid JSON must yield a Valid verdict");
    // The committed ref IS the verdict's content address (SN-8 exact equality).
    assert_eq!(commit.result_ref.as_bytes(), &verdict.content_ref_bytes());
}

#[test]
fn critic_commits_invalid_verdict_when_producer_output_violates() {
    let journal = InMemoryJournal::new();
    let store = InMemoryContentStore::new();
    let producer = producer_mote();
    commit_producer(&journal, &store, &producer, b"not json at all {{{");

    let critic = critic_mote(producer.id, json_check());
    run_native_critic_mote(&critic, &warrant(), &journal, &store).unwrap();

    let verdict = committed_verdict(&journal, &store, &critic);
    assert!(
        !verdict.is_valid(),
        "malformed JSON must yield an Invalid verdict (a SUCCESSFUL critic commit, not a Failed)"
    );
}

#[test]
fn verdict_is_byte_identical_across_independent_runs() {
    let check = json_check();
    let payload = br#"{"a":[1,2,3]}"#;

    let run = || {
        let journal = InMemoryJournal::new();
        let store = InMemoryContentStore::new();
        let producer = producer_mote();
        commit_producer(&journal, &store, &producer, payload);
        let critic = critic_mote(producer.id, check.clone());
        run_native_critic_mote(&critic, &warrant(), &journal, &store)
            .unwrap()
            .result_ref
    };
    assert_eq!(
        run(),
        run(),
        "same producer bytes + check => byte-identical verdict ref"
    );
}

#[test]
fn p04_gate_serves_committed_verdict_without_reevaluating() {
    let journal = InMemoryJournal::new();
    let store = InMemoryContentStore::new();
    let producer = producer_mote();
    commit_producer(&journal, &store, &producer, br#"{"ok":1}"#);
    let critic = critic_mote(producer.id, json_check());

    let first = run_native_critic_mote(&critic, &warrant(), &journal, &store).unwrap();
    let second = run_native_critic_mote(&critic, &warrant(), &journal, &store).unwrap();
    assert_eq!(
        first.committed_seq, second.committed_seq,
        "P0.4 gate serves the same commit"
    );
    assert_eq!(first.result_ref, second.result_ref);
    // Exactly one Committed entry for the critic.
    assert!(matches!(
        journal.read_committed(&critic.id).unwrap(),
        Some(JournalEntry::Committed { .. })
    ));
}

#[test]
fn r15_refuses_native_critic_without_critic_for() {
    // A critic_check with no critic_for is an ill-formed native critic. Submitted
    // alone (a PURE Mote, so no earlier WM predicate fires) it trips R-15.
    let producer = producer_mote();
    let mut def = critic_mote(producer.id, json_check()).def.clone();
    def.critic_for = None; // critic_check present but no producer => R-15
    let bad = Mote::new(
        def,
        InputDataId::from_bytes([20; 32]),
        GraphPosition("/critic".into()),
        SmallVec::new(),
    );

    let mut motes = BTreeMap::new();
    motes.insert(bad.id, bad.clone());
    let submission = submission(motes);
    let err = validate_submission(&submission).unwrap_err();
    assert!(
        matches!(err, SubmissionRefusal::R15NativeCheckShape { .. }),
        "expected R-15, got {err:?}"
    );
}

#[test]
fn run_native_critic_rejects_non_pure_mote() {
    // Defense-in-depth: the execution path itself refuses a non-PURE native
    // critic (the run-time half of R-15), so a misrouted WM Mote can never
    // execute the in-process check.
    let journal = InMemoryJournal::new();
    let store = InMemoryContentStore::new();
    let producer = producer_mote();
    commit_producer(&journal, &store, &producer, br#"{"ok":1}"#);

    let mut def = critic_mote(producer.id, json_check()).def.clone();
    def.nd_class = NdClass::WorldMutating; // illegal native-critic shape
    let bad = Mote::new(
        def,
        InputDataId::from_bytes([20; 32]),
        GraphPosition("/critic".into()),
        SmallVec::new(),
    );

    assert!(
        run_native_critic_mote(&bad, &warrant(), &journal, &store).is_err(),
        "a non-PURE native critic must be refused at execution (run-time R-15)"
    );
}
