//! PR-2d-1 — the LIVE ReAct substrate inside the coordinator (answer-only).
//!
//! Proves the runtime-side capability: a `react_seed` submit swaps in the RUN-SALTED
//! turn-0 Mote (server-derived identity, SN-8) and anchors a durable `ReactRound`
//! fact; the sole-writer coordinator settles each committed turn by decoding its RAW
//! output through the ONE authority gate (`kx-toolcall`), freezes the branch as a
//! durable fact, advances the chain under the fold-re-derived budget, serves the
//! trajectory to the next turn via F-7 — and the chain SURVIVES a coordinator
//! restart (re-derived from committed facts alone, never re-sampled — R49). The
//! model that PRODUCES each turn's output is a gateway concern; here outputs are
//! staged directly so the test is deterministic + model-free (the `replan_live.rs`
//! pattern).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::proto::{CommitOutcome, ExecutorClass as ProtoExecutorClass};
use kx_coordinator::{CoordinatorService, InMemoryWorkerRegistry, MoteState, WorkerRegistry};
use kx_journal::{Journal, JournalEntry, ReactBranch, SqliteJournal};
use kx_mote::{
    ConfigKey, ConfigVal, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY, REACT_TURN_KEY,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::Request;

const MAC: ProtoExecutorClass = ProtoExecutorClass::MacosSandbox;
const INSTRUCTION: &str = "List the files, then answer.";
const MODEL: &str = "react-v1";
const TOOL_ENVELOPE: &[u8] = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"q":"x"}}}"#;

/// The client's SEED Mote: an ordinary ROND model Mote carrying the instruction.
/// Its identity is advisory — the coordinator swaps in the run-salted turn 0.
fn seed_mote() -> Mote {
    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(INSTRUCTION.as_bytes().to_vec()),
    );
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef([7u8; 32]),
        model_id: ModelId(MODEL.into()),
        prompt_template_hash: PromptTemplateHash([7u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([7u8; 32]),
        GraphPosition(vec![7u8]),
        SmallVec::new(),
    )
}

/// A react warrant. `granted = true` adds the `mcp-echo@1` grant so the settle
/// decode can return `Ok(Some)` (the PR-2d-2 shape, exercised model-free here).
fn warrant(granted: bool) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    if granted {
        tool_grants.insert(ToolGrant {
            tool_id: kx_mote::ToolName("mcp-echo".into()),
            tool_version: kx_mote::ToolVersion("1".into()),
        });
    }
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId(MODEL.into()),
            max_input_tokens: 1024,
            max_output_tokens: 1024,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 20,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 20,
        },
        environment_ref: None,
        executor_class: ExecutorClass::MacOsSandbox,
        ..Default::default()
    }
}

fn coordinator(dir: &TempDir) -> (CoordinatorService, Arc<LocalFsContentStore>) {
    let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let registry: Arc<dyn WorkerRegistry> = Arc::new(InMemoryWorkerRegistry::new());
    let svc = CoordinatorService::with_shaper_materialization(
        journal,
        registry,
        store.clone(),
        Arc::new(kx_coordinator::SystemClock),
        Arc::new(kx_coordinator::OsRandomNonce),
        Arc::new(kx_tool_registry::InMemoryToolRegistry::with_builtins()),
        Arc::new(kx_warrant::InMemoryRoleRegistry::new()),
    );
    (svc, store)
}

/// Submit `mote` with `react_seed = true`; returns `(turn0_mote_id, instance_id)`.
async fn submit_react(
    svc: &CoordinatorService,
    mote: &Mote,
    w: &WarrantSpec,
) -> (Vec<u8>, Vec<u8>) {
    let _ = common::register_run(svc, [0x5a; 32]).await;
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: true,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        kx_coordinator::proto::SubmitStatus::Accepted as i32
    );
    (resp.mote_id, resp.instance_id)
}

async fn commit_raw(
    svc: &CoordinatorService,
    store: &LocalFsContentStore,
    turn: &Mote,
    w: &WarrantSpec,
    bytes: &[u8],
    worker: u64,
) {
    let result_ref = store.put(bytes).unwrap();
    let id = turn.id.as_bytes().to_vec();
    let outcome = svc
        .report_commit(Request::new(kx_coordinator::proto::ReportCommitRequest {
            mote_id: id.clone(),
            idempotency_key: id,
            result_ref: result_ref.as_bytes().to_vec(),
            warrant_ref: warrant_ref_of(w).as_bytes().to_vec(),
            mote_def_hash: turn.def.hash().as_bytes().to_vec(),
            nd_class: kx_coordinator::proto::NdClass::from(turn.def.nd_class) as i32,
            parents: Vec::new(), // a react turn is EDGE-FREE
            worker_id: worker,
        }))
        .await
        .unwrap()
        .into_inner()
        .outcome;
    assert_eq!(outcome, CommitOutcome::Committed as i32);
}

/// All `ReactRound` facts currently in the journal, in seq order. The wire
/// `ReadEntries` carries only Committed entries, so this opens a second reader
/// connection on the journal file (SQLite WAL). `svc` is used as an ORDERING
/// BARRIER first: the settle pass runs at the end of the drain that folded the
/// commit, and any subsequent RPC is processed in a later drain — so once the
/// barrier returns, every settle the test caused is durable.
async fn react_facts(svc: &CoordinatorService, dir: &TempDir) -> Vec<JournalEntry> {
    let _ = svc.committed_count().await; // ordering barrier (a later drain)
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let head = journal.current_seq().unwrap();
    journal
        .read_entries_by_seq(0..head + 1)
        .unwrap()
        .filter(|e| matches!(e, JournalEntry::ReactRound { .. }))
        .collect()
}

/// Flagship: a `react_seed` submit swaps in the run-salted turn 0 (server-derived
/// identity — never the client's advisory id), anchors the durable fact with the
/// budget caps, and the leased turn carries the marker + instruction, edge-free.
#[tokio::test]
async fn react_seed_swaps_in_a_salted_turn0_and_anchors() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(false);
    let seed = seed_mote();

    let (turn0_id, instance_id) = submit_react(&svc, &seed, &w).await;
    assert_ne!(
        turn0_id,
        seed.id.as_bytes().to_vec(),
        "the admitted identity is SERVER-derived (the seed-swap), never the client's"
    );

    // The durable anchor: turn 0, Pending, the run's instance_id, the 8/8 caps.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 1, "exactly the turn-0 anchor");
    match &facts[0] {
        JournalEntry::ReactRound {
            turn,
            turn_mote_id,
            instance_id: fact_instance,
            branch,
            max_turns,
            max_tool_calls,
            ..
        } => {
            assert_eq!(*turn, 0);
            assert_eq!(turn_mote_id.as_bytes().to_vec(), turn0_id);
            assert_eq!(fact_instance.to_vec(), instance_id);
            assert_eq!(*branch, ReactBranch::Pending);
            assert_eq!((*max_turns, *max_tool_calls), (8, 8));
        }
        other => panic!("expected a ReactRound anchor, got {other:?}"),
    }

    // The leased turn: run-salted id, the marker (value = the salt), the
    // instruction, EDGE-FREE, not a shaper.
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "turn 0 is immediately leasable");
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(turn0.id.as_bytes().to_vec(), turn0_id);
    assert!(turn0.parents.is_empty(), "a react turn is edge-free");
    assert!(!turn0.def.is_topology_shaper);
    assert_eq!(
        turn0
            .def
            .config_subset
            .get(&ConfigKey(REACT_TURN_KEY.to_string()))
            .map(|v| v.0.clone()),
        Some(instance_id),
        "the routing marker carries the run-salt"
    );
    assert_eq!(
        turn0
            .def
            .config_subset
            .get(&ConfigKey(PROMPT_KEY.to_string()))
            .map(|v| v.0.clone()),
        Some(INSTRUCTION.as_bytes().to_vec())
    );
}

/// `react_seed = false` keeps today's behavior byte-identical: the admitted
/// identity IS the client Mote's re-derived id (proto-compat — an old client
/// sends the default `false` and nothing changes).
#[tokio::test]
async fn flag_false_admits_the_client_mote_unchanged() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(false);
    let seed = seed_mote();

    let resp = common::submit(&svc, &seed, &w).await;
    assert_eq!(resp.mote_id, seed.id.as_bytes().to_vec());
    assert!(
        react_facts(&svc, &dir).await.is_empty(),
        "no anchor without the flag"
    );
}

/// A promptless seed is refused LOUDLY (failed_precondition) — the flag is
/// explicit intent; a chain that cannot reason or recover must never half-start.
#[tokio::test]
async fn promptless_seed_is_refused_loudly() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(false);
    let mut seed = seed_mote();
    seed.def.config_subset.clear();
    let seed = Mote::new(
        seed.def,
        InputDataId::from_bytes([7u8; 32]),
        GraphPosition(vec![7u8]),
        SmallVec::new(),
    );

    let _ = common::register_run(&svc, [0x5a; 32]).await;
    let err = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(seed.into()),
            warrant: Some(w.into()),
            accept_at_least_once: false,
            react_seed: true,
        }))
        .await
        .expect_err("a promptless react seed is refused");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        react_facts(&svc, &dir).await.is_empty(),
        "nothing was anchored"
    );
}

/// A committed prose answer settles the chain: a frozen `Answer` fact, no next
/// turn, and the committed turn fact IS the final answer (the harness oracle:
/// `empty_tool_grants_is_pure_reasoning` — with no grants, ANY output answers).
#[tokio::test]
async fn answer_on_turn0_settles_the_chain() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(false);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Even a perfectly-formed envelope is an ANSWER under empty grants (SN-8).
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;

    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 2, "anchor + the Answer settle");
    assert!(matches!(
        &facts[1],
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::Answer,
            ..
        }
    ));
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "Answer is terminal — no next turn"
    );
}

/// A granted tool proposal advances the chain: a frozen `Tool` fact, then the
/// next turn (run-salted, distinct id, same instruction) — and F-7 serves the
/// committed trajectory (turn 0's output) to turn 1 in transcript order.
#[tokio::test]
async fn tool_branch_advances_the_chain_with_trajectory() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (turn0_id, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;

    // The frozen decision + the next Pending turn.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 3, "anchor + Tool settle + turn-1 Pending");
    assert!(matches!(
        &facts[1],
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::Tool { tool_id, tool_version },
            ..
        } if tool_id == "mcp-echo" && tool_version == "1"
    ));
    assert!(matches!(
        &facts[2],
        JournalEntry::ReactRound {
            turn: 1,
            branch: ReactBranch::Pending,
            ..
        }
    ));

    // Turn 1 is leasable: distinct salted id, same instruction, edge-free — and
    // its F-7 parent_results carry turn 0's committed output (transcript order).
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "turn 1 is leasable");
    let turn1: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_ne!(turn1.id.as_bytes().to_vec(), turn0_id);
    assert!(turn1.parents.is_empty());
    let parents = &leased[0].parent_results;
    assert_eq!(parents.len(), 1, "F-7 serves the trajectory out-of-band");
    assert_eq!(parents[0].parent_mote_id, turn0_id);
    let served = store
        .get(&ContentRef::from_bytes(
            parents[0].result_ref.clone().try_into().unwrap(),
        ))
        .unwrap();
    assert_eq!(
        served.as_ref(),
        TOOL_ENVELOPE,
        "the committed turn-0 output"
    );
}

/// A dead-lettered turn settles the chain `DeadLettered` (terminal — no next turn).
#[tokio::test]
async fn failed_turn_dead_letters_the_chain() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(false);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    common::report_failure(
        &svc,
        &turn0,
        worker,
        kx_coordinator::proto::FailureReason::DeadLettered,
    )
    .await
    .unwrap();
    assert_eq!(svc.state_of(turn0.id).await.unwrap(), MoteState::Failed);

    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 2, "anchor + the DeadLettered settle");
    assert!(matches!(
        &facts[1],
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::DeadLettered,
            ..
        }
    ));
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "DeadLettered is terminal"
    );
}

/// Every turn proposes a tool ⇒ the chain is bounded by the durable budget
/// (8 tool calls), then quiesces — no runaway chain / unbounded journal growth.
/// The gate is the harness mirror: tool-budget first, `>=`, fold-re-derived.
#[tokio::test]
async fn chain_is_bounded_by_the_durable_budget() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;

    let mut turns_leased = 0u32;
    for _ in 0..12 {
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let Some(item) = leased.into_iter().next() else {
            break;
        };
        turns_leased += 1;
        let turn: Mote = item.mote.unwrap().try_into().unwrap();
        commit_raw(&svc, &store, &turn, &w, TOOL_ENVELOPE, worker).await;
    }
    // turns 0..7 lease + commit a Tool each; after turn 7 the gate fires
    // (tool_calls = 8 >= max_tool_calls) and no turn 8 spawns.
    assert_eq!(
        turns_leased, 8,
        "exactly max_tool_calls turns, then quiesce"
    );
    assert!(common::lease_work(&svc, worker, MAC, 16).await.is_empty());
    // Every recorded fact carries the durable caps the run was admitted under.
    for fact in react_facts(&svc, &dir).await {
        if let JournalEntry::ReactRound {
            max_turns,
            max_tool_calls,
            ..
        } = fact
        {
            assert_eq!((max_turns, max_tool_calls), (8, 8));
        }
    }
}

/// Crash with turn 0 leased-but-uncommitted ⇒ recovery re-inserts the SAME
/// run-salted turn (rebuilt from the anchor — R49: identical bytes), it re-leases,
/// commits, and the chain settles. Committed facts are SERVED, never re-sampled.
#[tokio::test]
async fn crash_resume_releases_the_inflight_turn_with_the_same_identity() {
    let dir = TempDir::new().unwrap();
    let w = warrant(false);
    let turn0_id;
    {
        let (svc, _store) = coordinator(&dir);
        let (id, _) = submit_react(&svc, &seed_mote(), &w).await;
        turn0_id = id;
        let worker = common::register(&svc, "w").await;
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        assert_eq!(leased.len(), 1);
        // svc dropped here → simulated crash before any commit.
    }

    let (svc, store) = coordinator(&dir);
    let worker = common::register(&svc, "w2").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(
        leased.len(),
        1,
        "the in-flight turn is re-leased after restart"
    );
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        turn0.id.as_bytes().to_vec(),
        turn0_id,
        "the rebuilt turn has the SAME run-salted identity (R49)"
    );
    commit_raw(&svc, &store, &turn0, &w, b"the answer", worker).await;
    let facts = react_facts(&svc, &dir).await;
    assert!(matches!(
        facts.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::Answer,
            ..
        }
    ));
}

/// Crash after a Tool advance (turn 1 Pending, materialized but uncommitted) ⇒
/// recovery re-derives the chain: turn 1 re-leases with the same identity, the
/// budget holds (no duplicate facts), and a cold re-fold reproduces the same
/// folded chain (R49 cold-refold).
#[tokio::test]
async fn crash_resume_mid_chain_converges_without_duplicates() {
    let dir = TempDir::new().unwrap();
    let w = warrant(true);
    let turn1_id;
    let facts_before;
    {
        let (svc, store) = coordinator(&dir);
        let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
        let worker = common::register(&svc, "w").await;
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
        commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        turn1_id = leased[0].mote.clone().unwrap().mote_id;
        facts_before = react_facts(&svc, &dir).await.len();
        assert_eq!(facts_before, 3, "anchor + Tool + turn-1 Pending");
        // svc dropped here → crash with turn 1 in flight.
    }

    let (svc, store) = coordinator(&dir);
    // No duplicate facts on recovery (idempotent re-drive).
    assert_eq!(react_facts(&svc, &dir).await.len(), facts_before);
    let worker = common::register(&svc, "w2").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1);
    let turn1: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        turn1.id.as_bytes().to_vec(),
        turn1_id,
        "same identity (R49)"
    );
    // F-7 still serves turn 0's output after restart.
    assert_eq!(leased[0].parent_results.len(), 1);
    // Answer on turn 1 terminates the chain.
    commit_raw(&svc, &store, &turn1, &w, b"done", worker).await;
    assert!(matches!(
        react_facts(&svc, &dir).await.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 1,
            branch: ReactBranch::Answer,
            ..
        }
    ));

    // R49 cold re-fold: two independent folds of the journal agree byte-for-byte.
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let p1 = kx_projection::Projection::from_journal(&journal).unwrap();
    let p2 = kx_projection::Projection::from_journal(&journal).unwrap();
    assert_eq!(p1.state_digest(), p2.state_digest());
    assert_eq!(
        p1.react_rounds().len(),
        4,
        "anchor, Tool@0, Pending@1, Answer@1"
    );
    let _ = store; // keep the store alive through the asserts
}

/// A malformed (committed-to-but-garbled) proposal settles `DeadLettered` —
/// fail-closed, mirroring the harness `malformed_proposal_dead_letters_no_effect`.
#[tokio::test]
async fn malformed_committed_proposal_dead_letters_the_chain() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // grants make the envelope path live

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Truncated envelope: committed to a call but malformed (the live gateway
    // fences this pre-commit; the substrate must STILL fail closed if such bytes
    // ever reach the journal — defense-in-depth).
    commit_raw(
        &svc,
        &store,
        &turn0,
        &w,
        br#"{"tool_call":{"name":"mcp-echo","#,
        worker,
    )
    .await;
    let facts = react_facts(&svc, &dir).await;
    assert!(matches!(
        facts.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::DeadLettered,
            ..
        }
    ));
    assert!(common::lease_work(&svc, worker, MAC, 16).await.is_empty());
}

/// An UNGRANTED tool proposal settles `DeadLettered` (SN-8: the model cannot
/// conjure a tool the warrant withheld — prompt injection cannot escalate).
#[tokio::test]
async fn ungranted_proposal_dead_letters_the_chain() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // grants mcp-echo@1 ONLY

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    commit_raw(
        &svc,
        &store,
        &turn0,
        &w,
        br#"{"tool_call":{"name":"mcp-danger","version":"1","args":{}}}"#,
        worker,
    )
    .await;
    assert!(matches!(
        react_facts(&svc, &dir).await.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::DeadLettered,
            ..
        }
    ));
}
