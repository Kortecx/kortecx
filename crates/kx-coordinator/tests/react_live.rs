//! PR-2d-1/PR-2d-2 — the LIVE ReAct chain inside the coordinator.
//!
//! Proves the runtime-side capability: a `react_seed` submit swaps in the RUN-SALTED
//! turn-0 Mote (server-derived identity, SN-8) and anchors a durable `ReactRound`
//! fact; the sole-writer coordinator settles each committed turn by decoding its RAW
//! output through the ONE authority gate (`kx-toolcall`), freezes the branch as a
//! durable fact, drives the TOOL ROUND (PR-2d-2: materialize the observation,
//! lease it WITH the coordinator-validated args, advance only once it commits),
//! bounds the chain under the fold-re-derived budget, serves the interleaved
//! trajectory (`[turn0, obs0, turn1, …]`) via F-7 — and the chain SURVIVES a
//! coordinator restart (re-derived from committed facts alone, never re-sampled —
//! R49). The model that PRODUCES each turn's output is a gateway concern; here
//! outputs are staged directly so the test is deterministic + model-free (the
//! `replan_live.rs` pattern).

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

/// The builtins PLUS `mcp-echo@1` (typed schema: one required `q: Str`) — the
/// settle's validate-at-freeze (PR-2d-2) resolves the proposed tool against the
/// registry BEFORE freezing a `Tool` fact, so the tool the tests propose must
/// be registered (the harness `registry_with_mcp` mirror).
fn registry_with_mcp() -> Arc<dyn kx_tool_registry::ToolRegistry> {
    use kx_tool_registry::{
        IdempotencyClass, InMemoryToolRegistry, InputSchema, McpEndpointId, ParamSpec, ParamType,
        ToolDef, ToolKind, ToolProvenance, ToolRegistry,
    };
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        ToolDef {
            tool_id: kx_mote::ToolName("mcp-echo".into()),
            tool_version: kx_mote::ToolVersion("1".into()),
            kind: ToolKind::Mcp {
                endpoint: McpEndpointId("stdio://test".into()),
                remote_name: "echo".into(),
            },
            required_capability: kx_warrant::ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: "deterministic echo (ReAct live tests).".into(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: Some(InputSchema {
                params: vec![ParamSpec {
                    name: "q".into(),
                    ty: ParamType::Str { max_len: 256 },
                    required: true,
                }],
                deny_unknown: true,
            }),
        },
        ToolProvenance::HumanAuthored {
            author: "test".into(),
        },
    );
    Arc::new(reg)
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
        registry_with_mcp(),
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
    mote: &Mote,
    w: &WarrantSpec,
    bytes: &[u8],
    worker: u64,
) {
    let result_ref = store.put(bytes).unwrap();
    let id = mote.id.as_bytes().to_vec();
    let outcome = svc
        .report_commit(Request::new(kx_coordinator::proto::ReportCommitRequest {
            mote_id: id.clone(),
            idempotency_key: id,
            result_ref: result_ref.as_bytes().to_vec(),
            warrant_ref: warrant_ref_of(w).as_bytes().to_vec(),
            mote_def_hash: mote.def.hash().as_bytes().to_vec(),
            nd_class: kx_coordinator::proto::NdClass::from(mote.def.nd_class) as i32,
            // A react TURN is edge-free; an OBSERVATION carries its Data edge
            // to the proposing turn — pass whatever the Mote declares.
            parents: mote.parents.iter().map(|p| (*p).into()).collect(),
            worker_id: worker,
        }))
        .await
        .unwrap()
        .into_inner()
        .outcome;
    assert_eq!(outcome, CommitOutcome::Committed as i32);
}

/// Lease the single ready item and assert it is the OBSERVATION for `turn` of
/// the chain: a WM tool Mote (`mcp-echo@1` contract, EMPTY config, one Data
/// edge to the proposing turn) carried WITH its coordinator-validated args
/// (PR-2d-2: a react observation leases with args or not at all). Returns the
/// observation Mote and the leased args bytes.
async fn lease_observation(
    svc: &CoordinatorService,
    worker: u64,
    proposing_turn: &Mote,
) -> (Mote, Vec<u8>) {
    let leased = common::lease_work(svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the observation is leasable");
    let item = &leased[0];
    let obs: Mote = item.mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        obs.def
            .tool_contract
            .get(&kx_mote::ToolName("mcp-echo".into()))
            .map(|v| v.0.clone()),
        Some("1".to_string()),
        "the observation declares the frozen tool"
    );
    assert!(
        obs.def.config_subset.is_empty(),
        "args travel OUT-OF-BAND — the observation identity never moves"
    );
    assert_eq!(obs.parents.len(), 1);
    assert_eq!(obs.parents[0].parent_id, proposing_turn.id);
    let args = item
        .tool_args
        .as_ref()
        .expect("a react observation leases WITH its validated args");
    (obs, args.args_bytes.clone())
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

    // The durable anchor: turn 0, Pending, the run's instance_id, the default
    // 8-turns / 6-tool-calls caps (PR-2d-2: a useful budget leaves a turn to
    // read the last observation and answer).
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
            assert_eq!((*max_turns, *max_tool_calls), (8, 6));
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

/// A granted tool proposal drives the FULL tool round (PR-2d-2): a frozen
/// `Tool` fact, then the OBSERVATION (the WM tool Mote, leased WITH the
/// coordinator-validated args — the args oracle), and only once the observation
/// COMMITS does the next turn spawn — whose F-7 trajectory interleaves turn 0's
/// output AND the observation's, in transcript order (`[turn0, obs0]`).
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

    // The frozen decision — and NO next turn yet: the observation gates it.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "anchor + Tool settle (the obs gates turn 1)"
    );
    assert!(matches!(
        &facts[1],
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::Tool { tool_id, tool_version },
            ..
        } if tool_id == "mcp-echo" && tool_version == "1"
    ));

    // The ARGS ORACLE: the observation leases with the model's proposed args
    // (decoded + schema-validated on the sole writer), byte-identical to the
    // committed envelope's args.
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(args, br#"{"q":"x"}"#.to_vec());

    // The observation commits (the worker fired the tool) ⇒ turn 1 spawns.
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 3, "anchor + Tool + turn-1 Pending");
    assert!(matches!(
        &facts[2],
        JournalEntry::ReactRound {
            turn: 1,
            branch: ReactBranch::Pending,
            ..
        }
    ));

    // Turn 1 is leasable: distinct salted id, edge-free — and its F-7
    // parent_results INTERLEAVE the trajectory: turn 0's output then the
    // observation's (transcript order, the harness `[turn, obs]` pairs).
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "turn 1 is leasable");
    let turn1: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_ne!(turn1.id.as_bytes().to_vec(), turn0_id);
    assert!(turn1.parents.is_empty());
    let parents = &leased[0].parent_results;
    assert_eq!(parents.len(), 2, "F-7 serves [turn0, obs0]");
    assert_eq!(parents[0].parent_mote_id, turn0_id);
    assert_eq!(parents[1].parent_mote_id, obs.id.as_bytes().to_vec());
    let served_turn = store
        .get(&ContentRef::from_bytes(
            parents[0].result_ref.clone().try_into().unwrap(),
        ))
        .unwrap();
    assert_eq!(served_turn.as_ref(), TOOL_ENVELOPE);
    let served_obs = store
        .get(&ContentRef::from_bytes(
            parents[1].result_ref.clone().try_into().unwrap(),
        ))
        .unwrap();
    assert_eq!(served_obs.as_ref(), br#"{"echoed":{"q":"x"}}"#);
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
/// (the default 6 tool calls), then quiesces — no runaway chain / unbounded
/// journal growth. PR-2d-2: each round now alternates turn → observation (the
/// observation FIRES even on the final tool call — the harness order: fire,
/// THEN bound the loop). The gate is the harness mirror: tool-budget first,
/// `>=`, fold-re-derived.
#[tokio::test]
async fn chain_is_bounded_by_the_durable_budget() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;

    let mut turns_leased = 0u32;
    let mut observations_committed = 0u32;
    for _ in 0..24 {
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let Some(item) = leased.into_iter().next() else {
            break;
        };
        let mote: Mote = item.mote.unwrap().try_into().unwrap();
        if mote.def.tool_contract.is_empty() {
            // A TURN: commit the tool-proposing envelope.
            turns_leased += 1;
            commit_raw(&svc, &store, &mote, &w, TOOL_ENVELOPE, worker).await;
        } else {
            // The OBSERVATION: leases with args, commits the staged result.
            assert!(item.tool_args.is_some(), "observations lease WITH args");
            observations_committed += 1;
            commit_raw(&svc, &store, &mote, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
        }
    }
    // turns 0..5 each propose + fire a tool (6 tool calls = the default cap);
    // after observation 5 commits the gate fires (tool_calls = 6 >= 6) and no
    // turn 6 spawns — but the final observation DID fire (harness parity).
    assert_eq!(
        turns_leased, 6,
        "exactly max_tool_calls turns, then quiesce"
    );
    assert_eq!(
        observations_committed, 6,
        "every frozen Tool decision fired its observation, the last included"
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
            assert_eq!((max_turns, max_tool_calls), (8, 6));
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

/// Crash with the OBSERVATION in flight (the Tool fact frozen, the observation
/// leased-but-uncommitted) ⇒ recovery re-derives the chain from the frozen fact
/// alone: the SAME observation re-materializes (the deterministic derivation IS
/// the durable marker — red-team BLOCKER #2), re-leases WITH byte-identical
/// re-derived args, commits, the chain advances, the budget holds (no duplicate
/// facts), and a cold re-fold reproduces the same folded chain (R49).
#[tokio::test]
async fn crash_resume_mid_chain_converges_without_duplicates() {
    let dir = TempDir::new().unwrap();
    let w = warrant(true);
    let obs_id;
    let obs_args;
    let facts_before;
    {
        let (svc, store) = coordinator(&dir);
        let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
        let worker = common::register(&svc, "w").await;
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
        commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;
        let (obs, args) = lease_observation(&svc, worker, &turn0).await;
        obs_id = obs.id;
        obs_args = args;
        facts_before = react_facts(&svc, &dir).await.len();
        assert_eq!(facts_before, 2, "anchor + Tool (the obs gates turn 1)");
        // svc dropped here → crash with the observation in flight.
    }

    let (svc, store) = coordinator(&dir);
    // No duplicate facts on recovery (idempotent re-drive of the settle).
    assert_eq!(react_facts(&svc, &dir).await.len(), facts_before);
    let worker = common::register(&svc, "w2").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the in-flight observation re-leases");
    let obs: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(obs.id, obs_id, "same observation identity (R49)");
    assert_eq!(
        leased[0].tool_args.as_ref().map(|a| a.args_bytes.clone()),
        Some(obs_args),
        "the re-lease re-derives byte-identical args (pure function of facts)"
    );
    // The observation commits ⇒ turn 1 spawns; an Answer terminates the chain.
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "turn 1 spawned after the obs commit");
    let turn1: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        leased[0].parent_results.len(),
        2,
        "F-7 serves [turn0, obs0] after restart"
    );
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

/// A TOOL-EXECUTION failure (the worker F4 dead-letters the OBSERVATION — an
/// MCP error / non-resolvable tool) freezes a same-turn `DeadLettered` fact and
/// settles the chain: the harness fail-closed stop ("tool dispatch did not
/// commit — stopping the loop", `react.rs`) — a non-existent observation is
/// never fed into a next turn's assemble.
#[tokio::test]
async fn failed_observation_dead_letters_the_chain_same_turn() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;

    let (obs, _args) = lease_observation(&svc, worker, &turn0).await;
    common::report_failure(
        &svc,
        &obs,
        worker,
        kx_coordinator::proto::FailureReason::DeadLettered,
    )
    .await
    .unwrap();

    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 3, "anchor + Tool + the same-turn DeadLettered");
    assert!(matches!(
        facts.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::DeadLettered,
            ..
        }
    ));
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "the chain is dead — no next turn, no re-fire"
    );
}

/// A proposal whose ARGS fail the tool's typed `inputSchema` is refused AT THE
/// FREEZE (the settle's validate-at-freeze, the ONE authority site): the branch
/// freezes `DeadLettered`, never `Tool` — so no observation is ever
/// materialized and no effect can fire on schema-invalid args (D110.4).
#[tokio::test]
async fn schema_invalid_args_dead_letter_at_the_freeze() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Granted tool, well-formed envelope — but the args violate the schema
    // (missing the required `q`, smuggling an undeclared key).
    commit_raw(
        &svc,
        &store,
        &turn0,
        &w,
        br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"zz":"x"}}}"#,
        worker,
    )
    .await;

    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 2, "anchor + the DeadLettered freeze");
    assert!(matches!(
        facts.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::DeadLettered,
            ..
        }
    ));
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "no observation was materialized — nothing fires"
    );
}

/// The `kx/recipes/react` caps plumbing: a seed carrying canonical-JSON
/// `max_turns` / `max_tool_calls` config keys anchors THOSE durable caps; a
/// degenerate budget (`max_tool_calls >= max_turns`, or a cap above the hard
/// ceiling 8) is refused LOUDLY before anything is written.
#[tokio::test]
async fn seed_caps_are_anchored_and_validated() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(true);

    // Valid explicit caps (the recipe-bound shape: canonical JSON ints).
    let mut seed = seed_mote();
    seed.def.config_subset.insert(
        ConfigKey(kx_mote::REACT_MAX_TURNS_KEY.to_string()),
        ConfigVal(b"4".to_vec()),
    );
    seed.def.config_subset.insert(
        ConfigKey(kx_mote::REACT_MAX_TOOL_CALLS_KEY.to_string()),
        ConfigVal(b"2".to_vec()),
    );
    let (_, _) = submit_react(&svc, &seed, &w).await;
    match react_facts(&svc, &dir).await.first().unwrap() {
        JournalEntry::ReactRound {
            max_turns,
            max_tool_calls,
            ..
        } => assert_eq!((*max_turns, *max_tool_calls), (4, 2)),
        other => panic!("expected the anchor, got {other:?}"),
    }

    // A degenerate budget is refused (no turn left to read the observation).
    let mut bad = seed_mote();
    bad.def.config_subset.insert(
        ConfigKey(kx_mote::REACT_MAX_TURNS_KEY.to_string()),
        ConfigVal(b"2".to_vec()),
    );
    bad.def.config_subset.insert(
        ConfigKey(kx_mote::REACT_MAX_TOOL_CALLS_KEY.to_string()),
        ConfigVal(b"2".to_vec()),
    );
    let err = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(bad.into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: true,
        }))
        .await
        .expect_err("max_tool_calls >= max_turns is refused");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);

    // A cap above the hard ceiling is refused.
    let mut over = seed_mote();
    over.def.config_subset.insert(
        ConfigKey(kx_mote::REACT_MAX_TURNS_KEY.to_string()),
        ConfigVal(b"9".to_vec()),
    );
    let err = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(over.into()),
            warrant: Some(w.into()),
            accept_at_least_once: false,
            react_seed: true,
        }))
        .await
        .expect_err("a cap above the hard ceiling (8) is refused");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

/// The `instruction` free-param key (the `kx/recipes/react` slot name) seeds
/// the chain exactly like `PROMPT_KEY`, INCLUDING the recipe binder's
/// JSON-quoted encoding — the swapped turn 0 carries the CLEAN string.
#[tokio::test]
async fn instruction_key_seeds_the_chain_json_decoded() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(false);

    let mut seed = seed_mote();
    seed.def.config_subset.clear();
    seed.def.config_subset.insert(
        ConfigKey(kx_mote::REACT_INSTRUCTION_KEY.to_string()),
        // The kx-invoke binder writes a bound Str arg JSON-encoded (quoted).
        ConfigVal(br#""List the files, then answer.""#.to_vec()),
    );
    let (turn0_id, _) = submit_react(&svc, &seed, &w).await;

    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(turn0.id.as_bytes().to_vec(), turn0_id);
    assert_eq!(
        turn0
            .def
            .config_subset
            .get(&ConfigKey(PROMPT_KEY.to_string()))
            .map(|v| v.0.clone()),
        Some(INSTRUCTION.as_bytes().to_vec()),
        "the swapped turn carries the CLEAN (unquoted) instruction"
    );
}

/// A PRE-COMMIT-CRASH flavor failure on the OBSERVATION (the heartbeat REAP)
/// must NOT dead-letter the chain — the reaped worker's commit may still land
/// (the fold lets a later Committed win) and then the chain ADVANCES: the
/// PR-2d-1 adversarial-review flavor guard, mirrored onto the tool round.
#[tokio::test]
async fn observation_crash_flavor_stays_active_and_a_late_commit_advances() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[derive(Debug)]
    struct FakeClock(AtomicU64);
    impl kx_coordinator::Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    let dir = TempDir::new().unwrap();
    let clock = Arc::new(FakeClock(AtomicU64::new(1_000)));
    let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        kx_coordinator::InMemoryWorkerRegistry::with_clock_and_timeout(
            clock.clone(),
            Duration::from_secs(6),
        ),
    );
    let svc = CoordinatorService::with_shaper_materialization(
        journal,
        registry,
        store.clone(),
        clock.clone(),
        Arc::new(kx_coordinator::OsRandomNonce),
        registry_with_mcp(),
        Arc::new(kx_warrant::InMemoryRoleRegistry::new()),
    );
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let dying = common::register(&svc, "dying").await;
    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, dying).await;
    let (obs, _args) = lease_observation(&svc, dying, &turn0).await;

    // The reap fires on the next poll: the observation folds
    // Failed{WorkerCrashed} (the crash flavor — no recorded failure_reason).
    clock.0.store(1_000 + 6_001, Ordering::Relaxed);
    let live = common::register(&svc, "live").await;
    let released = common::lease_work(&svc, live, MAC, 16).await;
    assert!(
        released.is_empty(),
        "a crash-failed StageThenCommit observation is NOT auto-re-leased \
         (no staged hint — the effect may already have fired, R-13)"
    );
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "no DeadLettered fact for a crash flavor — the tool round stays open"
    );

    // The reaped worker's in-flight commit lands — Committed wins, the round
    // completes, and the chain ADVANCES to turn 1.
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, dying).await;
    let facts = react_facts(&svc, &dir).await;
    assert!(
        matches!(
            facts.last().unwrap(),
            JournalEntry::ReactRound {
                turn: 1,
                branch: ReactBranch::Pending,
                ..
            }
        ),
        "the late observation commit advanced the chain — never discarded"
    );
}

/// A PRE-COMMIT-CRASH flavor failure (the heartbeat REAP's `WorkerCrashed`) must
/// NOT dead-letter the chain: the reaped worker's commit may still be in flight
/// (the fold deliberately lets a later Committed win), and a genuinely dead
/// worker leaves the turn stuck-but-operator-recoverable (the standing non-PURE
/// semantics). The adversarial-review race, end-to-end through the REAL reap
/// path: lease → heartbeat-timeout reap (a fake clock) → assert no DeadLettered
/// → the "late" commit lands → the chain settles `Answer`, never discarded.
#[tokio::test]
async fn worker_crash_does_not_dead_letter_and_a_late_commit_still_settles() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[derive(Debug)]
    struct FakeClock(AtomicU64);
    impl kx_coordinator::Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    let dir = TempDir::new().unwrap();
    let clock = Arc::new(FakeClock(AtomicU64::new(1_000)));
    let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        kx_coordinator::InMemoryWorkerRegistry::with_clock_and_timeout(
            clock.clone(),
            Duration::from_secs(6),
        ),
    );
    let svc = CoordinatorService::with_shaper_materialization(
        journal,
        registry,
        store.clone(),
        clock.clone(),
        Arc::new(kx_coordinator::OsRandomNonce),
        Arc::new(kx_tool_registry::InMemoryToolRegistry::with_builtins()),
        Arc::new(kx_warrant::InMemoryRoleRegistry::new()),
    );
    let w = warrant(false);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let dying = common::register(&svc, "dying").await;
    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1);
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Time passes the liveness window; the next lease poll REAPS the dying
    // worker → Failed{WorkerCrashed} folds (failed_pending_reattempt, NOT
    // terminal) and the drain-end settle runs. The chain must stay Pending.
    clock.0.store(1_000 + 6_001, Ordering::Relaxed);
    let live = common::register(&svc, "live").await;
    let released = common::lease_work(&svc, live, MAC, 16).await;
    assert!(
        released.is_empty(),
        "a crash-failed ROND turn is NOT auto-re-leased (standing non-PURE semantics)"
    );
    assert_eq!(svc.state_of(turn0.id).await.unwrap(), MoteState::Failed);
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        1,
        "no DeadLettered fact for a crash flavor — the frontier stays Pending"
    );
    assert!(matches!(
        facts.last().unwrap(),
        JournalEntry::ReactRound {
            turn: 0,
            branch: ReactBranch::Pending,
            ..
        }
    ));

    // The "late" commit (the reaped-but-alive worker's in-flight result) lands —
    // Committed wins over the crash flag, and the chain settles the ANSWER.
    commit_raw(&svc, &store, &turn0, &w, b"the answer survived", dying).await;
    let facts = react_facts(&svc, &dir).await;
    assert!(
        matches!(
            facts.last().unwrap(),
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Answer,
                ..
            }
        ),
        "the committed answer settles the chain — never discarded"
    );
}
