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
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId,
    Mote, MoteDef, NdClass, ParentRef, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY,
    REACT_TURN_KEY,
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
/// PR-9a (the BUG-28 regression pin): Gemma-4's NATIVE tool-call shape
/// (`<|tool_call>call:NAME{ARGS}<tool_call|>`) — `mcp_echo` (underscore) exercises
/// the parser's separator-only `_`→`-` name normalization. BUG-28 was a real-model
/// tool loop that NEVER fired because no e2e drove this shape through the settle's
/// decode→Tool-freeze→observation-fire→commit path; this const lets the fire-commits
/// test assert that invariant for BOTH the JSON envelope and the native shape.
const GEMMA_NATIVE: &[u8] = br#"<|tool_call>call:mcp_echo{"q":"x"}<tool_call|>"#;
/// PR-9c-1 (dynamic multi-format tool-calling): Llama-3.1/3.2's native
/// `<|python_tag|>{"name":…,"parameters":…}` shape — drives the new accept-side arm
/// through the SAME settle→Tool-freeze→observation-fire→commit path. `parameters`
/// (Llama's alias) exercises the tolerant args-key resolution.
const PYTHON_TAG_NATIVE: &[u8] = br#"<|python_tag|>{"name":"mcp-echo","parameters":{"q":"x"}}"#;
/// PR-9c-1: Qwen3/Hermes's native `<tool_call>\n{"name":…,"arguments":…}\n</tool_call>`
/// XML-ish shape (newline-wrapped, as Qwen3 emits) — the other new accept-side arm.
const XML_TOOL_NATIVE: &[u8] =
    b"<tool_call>\n{\"name\":\"mcp-echo\",\"arguments\":{\"q\":\"x\"}}\n</tool_call>";
/// PR-R1: the markerless `{"name":…,"arguments":…}` (OpenAI / Hermes) shape — fires
/// via the new COMMITMENT-AWARE accept-side arm (a granted name + an explicit args bag).
const MARKERLESS_NATIVE: &[u8] = br#"{"name":"mcp-echo","arguments":{"q":"x"}}"#;
/// PR-R1: the single-element `{"tool_calls":[…]}` (OpenAI plural) wrapper shape.
const TOOL_CALLS_NATIVE: &[u8] = br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}}]}"#;

/// The client's SEED Mote: an ordinary ROND model Mote carrying the instruction.
/// Its identity is advisory — the coordinator swaps in the run-salted turn 0. Since
/// PR-R1 the swapped chain is SALTED by this seed's `MoteId` (a content hash that
/// includes the instruction), so a distinct instruction ⇒ a distinct chain.
fn seed_mote() -> Mote {
    seed_mote_with(INSTRUCTION)
}

/// A SEED Mote carrying `instruction` — distinct instructions yield distinct seed
/// `MoteId`s (the def hash folds `config_subset[PROMPT_KEY]`), hence distinct PR-R1
/// chain salts. Used by the per-invocation-identity proofs.
fn seed_mote_with(instruction: &str) -> Mote {
    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(instruction.as_bytes().to_vec()),
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

/// The `mcp-echo@1` ToolDef (typed schema: one required `q: Str`) the live tests
/// register — the settle's validate-at-freeze (PR-2d-2) resolves a proposed tool
/// against the registry BEFORE freezing a `Tool` fact, so the tool the tests
/// propose must be registered. Extracted so the PR-9a deregister-mid-chain test
/// can register it durably (deregisterable) and remove it after the freeze.
fn echo_tool_def() -> kx_tool_registry::ToolDef {
    use kx_tool_registry::{
        IdempotencyClass, InputSchema, McpEndpointId, ParamSpec, ParamType, ToolDef, ToolKind,
    };
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
    }
}

fn registry_with_mcp() -> Arc<dyn kx_tool_registry::ToolRegistry> {
    use kx_tool_registry::{InMemoryToolRegistry, ToolProvenance, ToolRegistry};
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        echo_tool_def(),
        ToolProvenance::HumanAuthored {
            author: "test".into(),
        },
    );
    Arc::new(reg)
}

fn coordinator_with(
    dir: &TempDir,
    tool_registry: Arc<dyn kx_tool_registry::ToolRegistry>,
) -> (CoordinatorService, Arc<LocalFsContentStore>) {
    let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let registry: Arc<dyn WorkerRegistry> = Arc::new(InMemoryWorkerRegistry::new());
    let svc = CoordinatorService::with_shaper_materialization(
        journal,
        registry,
        store.clone(),
        Arc::new(kx_coordinator::SystemClock),
        Arc::new(kx_coordinator::OsRandomNonce),
        tool_registry,
        Arc::new(kx_warrant::InMemoryRoleRegistry::new()),
    );
    (svc, store)
}

fn coordinator(dir: &TempDir) -> (CoordinatorService, Arc<LocalFsContentStore>) {
    coordinator_with(dir, registry_with_mcp())
}

/// Submit `mote` with `react_seed = true`; asserts Accepted; returns
/// `(turn0_mote_id, instance_id)`.
async fn submit_react(
    svc: &CoordinatorService,
    mote: &Mote,
    w: &WarrantSpec,
) -> (Vec<u8>, Vec<u8>) {
    let (mote_id, instance_id, status) = submit_react_status(svc, mote, w).await;
    assert_eq!(status, kx_coordinator::proto::SubmitStatus::Accepted as i32);
    (mote_id, instance_id)
}

/// Submit a react seed WITHOUT asserting the status — the per-invocation-identity
/// proofs need the raw `(mote_id, instance_id, status)` to distinguish a fresh chain
/// (Accepted) from an idempotent re-submit of the SAME goal (Duplicate).
async fn submit_react_status(
    svc: &CoordinatorService,
    mote: &Mote,
    w: &WarrantSpec,
) -> (Vec<u8>, Vec<u8>, i32) {
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
    (resp.mote_id, resp.instance_id, resp.status)
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

    // The leased turn: run-salted id, the marker (value = the COMPOUND chain key,
    // PR-R1), the instruction, EDGE-FREE, not a shaper.
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "turn 0 is immediately leasable");
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(turn0.id.as_bytes().to_vec(), turn0_id);
    assert!(turn0.parents.is_empty(), "a react turn is edge-free");
    assert!(!turn0.def.is_topology_shaper);
    // PR-R1: a run-level chain is now salted by its SEED MoteId (so distinct Invokes
    // split), so the routing marker is the 48-byte `instance_id ‖ chain_salt`
    // (chain_salt = the seed's MoteId) — NOT the bare 16-byte instance_id.
    let mut expected_marker = instance_id.clone();
    expected_marker.extend_from_slice(seed.id.as_bytes());
    assert_eq!(
        turn0
            .def
            .config_subset
            .get(&ConfigKey(REACT_TURN_KEY.to_string()))
            .map(|v| v.0.clone()),
        Some(expected_marker),
        "the routing marker carries instance_id ‖ chain_salt (the per-invocation key)"
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

    // PR-2 def persistence: every coordinator-materialized Mote's definition
    // lands content-addressed at exactly def.hash() (the GetMoteDetail read).
    let obs_def_blob = store
        .get(&ContentRef::from_bytes(*obs.def.hash().as_bytes()))
        .expect("observation def blob persisted at admission");
    assert_eq!(
        kx_mote::MoteDef::decode(&obs_def_blob).unwrap(),
        obs.def,
        "observation def bytes round-trip"
    );

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
    // PR-2 def persistence: the ADVANCED turn's def blob too.
    let turn1_def_blob = store
        .get(&ContentRef::from_bytes(*turn1.def.hash().as_bytes()))
        .expect("advanced turn def blob persisted at admission");
    assert_eq!(
        kx_mote::MoteDef::decode(&turn1_def_blob).unwrap(),
        turn1.def
    );
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

/// PR-9a (the BUG-28 regression pin, model-free + deterministic): the SAME
/// fire-commits invariant as `tool_branch_advances_the_chain_with_trajectory`, but
/// the turn-0 output is Gemma-4's NATIVE shape (`<|tool_call>call:mcp_echo{…}`).
/// This drives the REAL coordinator settle — which calls `kx-toolcall` to decode
/// the staged bytes — and asserts the native shape (a) freezes a `Tool` fact for
/// `mcp-echo@1` (name separator-normalized), (b) the observation leases WITH the
/// args decoded from the native shape, and (c) the observation commits ⇒ turn 1
/// spawns. BUG-28 was exactly this path being silently dead: the loop only ever
/// asserted an ANSWER settling, never a tool FIRING through the native arm.
#[tokio::test]
async fn tool_branch_fires_and_commits_via_gemma_native_shape() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    // Stage the NATIVE shape (not the JSON envelope) — the settle must decode it.
    commit_raw(&svc, &store, &turn0, &w, GEMMA_NATIVE, worker).await;

    // (a) the settle froze a `Tool` fact for the separator-normalized `mcp-echo@1`.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "anchor + the Tool settle (the native shape FIRED)"
    );
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { tool_id, tool_version },
                ..
            } if tool_id == "mcp-echo" && tool_version == "1"
        ),
        "the Gemma-native `<|tool_call>call:mcp_echo{{…}}` decodes + freezes a Tool fact"
    );

    // (b) the observation leases WITH the args decoded from the native shape.
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(
        args,
        br#"{"q":"x"}"#.to_vec(),
        "args decode from the native shape"
    );

    // (c) the observation commits (the tool FIRED) ⇒ turn 1 spawns — the world
    // mutating observation reaching Committed is the BUG-28 invariant.
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        3,
        "anchor + Tool + turn-1 Pending (the fire advanced the chain)"
    );
    assert!(matches!(
        &facts[2],
        JournalEntry::ReactRound {
            turn: 1,
            branch: ReactBranch::Pending,
            ..
        }
    ));
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the world-mutating observation COMMITTED — a tool genuinely fired"
    );
}

/// PR-9c-1: the Llama `<|python_tag|>{…}` shape FIRES + commits through the real
/// settle — proving the new accept-side arm is wired end-to-end (not just unit
/// tested in the parser leaf). The `parameters` alias + a non-namespaced grant.
#[tokio::test]
async fn tool_branch_fires_and_commits_via_python_tag_shape() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, PYTHON_TAG_NATIVE, worker).await;

    // (a) the settle decoded the python_tag shape + froze a Tool fact for mcp-echo@1.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "anchor + the Tool settle (python_tag FIRED)"
    );
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { tool_id, tool_version },
                ..
            } if tool_id == "mcp-echo" && tool_version == "1"
        ),
        "the Llama `<|python_tag|>{{…}}` shape decodes + freezes a Tool fact"
    );

    // (b) the observation leases WITH the args decoded from `parameters`.
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(
        args,
        br#"{"q":"x"}"#.to_vec(),
        "args decode from `parameters`"
    );

    // (c) the observation commits (the tool FIRED) ⇒ turn 1 spawns.
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the world-mutating observation COMMITTED — a python_tag tool genuinely fired"
    );
}

/// PR-9c-1: the Qwen3/Hermes `<tool_call>\n{…}\n</tool_call>` shape FIRES + commits
/// through the real settle — the other new accept-side arm, end-to-end. The
/// `arguments` alias + the newline-wrapped form Qwen3 actually emits.
#[tokio::test]
async fn tool_branch_fires_and_commits_via_xml_tool_call_shape() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, XML_TOOL_NATIVE, worker).await;

    // (a) the settle decoded the `<tool_call>` shape + froze a Tool fact for mcp-echo@1.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "anchor + the Tool settle (`<tool_call>` FIRED)"
    );
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { tool_id, tool_version },
                ..
            } if tool_id == "mcp-echo" && tool_version == "1"
        ),
        "the Qwen3 `<tool_call>{{…}}</tool_call>` shape decodes + freezes a Tool fact"
    );

    // (b) the observation leases WITH the args decoded from `arguments`.
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(
        args,
        br#"{"q":"x"}"#.to_vec(),
        "args decode from `arguments`"
    );

    // (c) the observation commits (the tool FIRED) ⇒ turn 1 spawns.
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the world-mutating observation COMMITTED — an `<tool_call>` tool genuinely fired"
    );
}

/// PR-R1: drive a single staged tool-call `shape` through the REAL settle and assert
/// it FIRES end-to-end — the settle freezes a `mcp-echo@1` Tool fact, the observation
/// leases WITH `{"q":"x"}`, and it COMMITS (the tool genuinely fired). The accept-side
/// complement to the per-format `kx-toolcall` unit tests.
async fn assert_shape_fires(shape: &[u8]) {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED
    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, shape, worker).await;

    let facts = react_facts(&svc, &dir).await;
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { tool_id, tool_version },
                ..
            } if tool_id == "mcp-echo" && tool_version == "1"
        ),
        "the markerless shape decodes + freezes a Tool fact (it FIRED)"
    );
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(
        args,
        br#"{"q":"x"}"#.to_vec(),
        "args decode from the markerless shape"
    );
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the observation COMMITTED — the markerless tool genuinely fired"
    );
}

/// PR-R1: the markerless `{"name":…,"arguments":…}` (OpenAI / Hermes) shape FIRES +
/// commits through the live settle — the new commitment-aware accept-side arm wired
/// end-to-end (not just unit-tested in the parser leaf).
#[tokio::test]
async fn tool_branch_fires_and_commits_via_markerless_shape() {
    assert_shape_fires(MARKERLESS_NATIVE).await;
}

/// PR-R1: the single-element `{"tool_calls":[…]}` (OpenAI plural) wrapper FIRES too.
#[tokio::test]
async fn tool_branch_fires_and_commits_via_tool_calls_wrapper_shape() {
    assert_shape_fires(TOOL_CALLS_NATIVE).await;
}

/// PR-R1 — FINDING-REACT-SHARED-INSTANCE FIXED (the headline reliability proof).
/// `kx serve` shares ONE journal / `instance_id` across every Invoke, so two DISTINCT
/// react goals must NOT dedup-collide at turn 0 (pre-fix the 2nd chain reused the
/// 1st's turns + answer — `run_agent` returned the first goal's answer for every later
/// run). Salting the run-level chain by its seed `MoteId` SPLITS them into distinct
/// chains with distinct answers — model-free + deterministic.
#[tokio::test]
async fn distinct_goals_split_into_distinct_chains_on_one_journal() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(false); // answer-only ⇒ each chain settles on its own Answer

    let seed_a = seed_mote_with("What is 2 + 2?");
    let seed_b = seed_mote_with("What is the capital of France?");
    let (turn0_a, inst_a) = submit_react(&svc, &seed_a, &w).await;
    let (turn0_b, inst_b) = submit_react(&svc, &seed_b, &w).await;

    // ONE shared journal ⇒ ONE instance_id; PR-R1 splits the CHAINS by their seed salt.
    assert_eq!(
        inst_a, inst_b,
        "a shared serve journal hands every Invoke the SAME instance_id"
    );
    assert_ne!(
        turn0_a, turn0_b,
        "distinct goals ⇒ distinct turn-0 ids (no dedup-collision — the fix)"
    );

    // BOTH chains' turn-0 are ready (the 2nd was NOT deduped onto the 1st).
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 2, "two independent chains, both leasable");

    // Settle each chain to its OWN distinct answer.
    for item in &leased {
        let turn0: Mote = item.mote.clone().unwrap().try_into().unwrap();
        let answer: &[u8] = if turn0.id.as_bytes().to_vec() == turn0_a {
            b"4"
        } else {
            b"Paris"
        };
        commit_raw(&svc, &store, &turn0, &w, answer, worker).await;
    }

    // Two DISTINCT run-level chains, each salted (PR-R1) + each with its OWN answer.
    let facts = react_facts(&svc, &dir).await;
    let anchor_salts: Vec<Option<[u8; 32]>> = facts
        .iter()
        .filter_map(|f| match f {
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Pending,
                step_salt,
                is_agentic_launch,
                ..
            } => {
                assert!(
                    !is_agentic_launch,
                    "a run-level chain is never an agentic launch"
                );
                Some(*step_salt)
            }
            _ => None,
        })
        .collect();
    assert_eq!(anchor_salts.len(), 2, "two run-level chain anchors");
    assert!(
        anchor_salts.iter().all(Option::is_some),
        "each run-level chain is now SALTED (PR-R1), never the bare None"
    );
    assert_ne!(
        anchor_salts[0], anchor_salts[1],
        "the two chains carry DISTINCT step_salts (= their distinct seed MoteIds)"
    );
    let answer_ids: Vec<_> = facts
        .iter()
        .filter_map(|f| match f {
            JournalEntry::ReactRound {
                branch: ReactBranch::Answer,
                turn_mote_id,
                ..
            } => Some(*turn_mote_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        answer_ids.len(),
        2,
        "each chain settled its OWN answer (no dedup)"
    );
    assert_ne!(
        answer_ids[0], answer_ids[1],
        "the two answers are DISTINCT committed motes (distinct goals, distinct results)"
    );
}

/// PR-R1: an IDENTICAL goal re-submitted on the same journal DEDUPS to one chain —
/// Invoke exactly-once is PRESERVED (a network retry of the same agent run is a no-op,
/// not a second chain). The complement to the distinct-goals split above.
#[tokio::test]
async fn identical_goal_dedups_to_one_chain() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(false);

    let seed = seed_mote_with("the very same question, twice");
    let (turn0_1, _, st1) = submit_react_status(&svc, &seed, &w).await;
    let (turn0_2, _, st2) = submit_react_status(&svc, &seed, &w).await;
    assert_eq!(st1, kx_coordinator::proto::SubmitStatus::Accepted as i32);
    assert_eq!(
        st2,
        kx_coordinator::proto::SubmitStatus::Duplicate as i32,
        "an identical goal is an idempotent re-submit (Invoke exactly-once)"
    );
    assert_eq!(
        turn0_1, turn0_2,
        "identical goal ⇒ the SAME chain (same turn-0 id)"
    );

    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "one chain only — the re-submit deduped");
}

/// The PR-9b-2b LIVE Gemma-4-12B shape that BUG-32 made fail-closed: the model
/// proposed `mcp-echo:echo` (the `<id>:<remote>` join) with an EMPTY version against
/// the `mcp-echo@1` grant. Before the fix the JSON-envelope arm's exact
/// `(name, version)` membership refused it ⇒ `UngrantedTool` ⇒ the chain
/// dead-lettered (honest, no fabricated answer). After the fix the `:remote` tail is
/// dropped, the head resolves to the unique grant, the GRANT's version is taken, and
/// the tool FIRES + commits — driven through the REAL coordinator settle here.
#[tokio::test]
async fn bug32_envelope_versionless_drift_fires_and_commits() {
    const TOOL_ENVELOPE_DRIFT: &[u8] =
        br#"{"tool_call":{"name":"mcp-echo:echo","version":"","args":{"q":"x"}}}"#;
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE_DRIFT, worker).await;

    // (a) the settle resolved the drifted name to `mcp-echo@1` and froze a Tool fact.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "anchor + the Tool settle (the drift shape FIRED)"
    );
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { tool_id, tool_version },
                ..
            } if tool_id == "mcp-echo" && tool_version == "1"
        ),
        "`mcp-echo:echo`+empty-version resolves to the granted `mcp-echo@1`"
    );

    // (b) + (c) the observation leases with the args and COMMITS (the tool fired).
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(args, br#"{"q":"x"}"#.to_vec());
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the world-mutating observation COMMITTED — a drifted-name tool genuinely fired"
    );
}

/// The headline BUG-32 shape (V2b): a dialed/local tool is granted NAMESPACED
/// (`kxlocal-<hash>/echo`) but the model proposes the BARE LEAF (`echo`). The
/// leaf must resolve to the namespaced grant (unambiguously) and the tool must
/// fire + commit. Uses a registry + warrant whose ONLY echo tool is namespaced.
#[tokio::test]
async fn bug32_native_bare_leaf_against_namespaced_grant_fires_and_commits() {
    const NS_TOOL: &str = "kxlocal-a1b2c3d4/echo";
    const BARE_LEAF_NATIVE: &[u8] = br#"<|tool_call>call:echo{"q":"x"}<tool_call|>"#;

    let namespaced_def = {
        let mut d = echo_tool_def();
        d.tool_id = kx_mote::ToolName(NS_TOOL.into());
        d
    };
    let registry: Arc<dyn kx_tool_registry::ToolRegistry> = {
        use kx_tool_registry::{InMemoryToolRegistry, ToolProvenance, ToolRegistry};
        let mut reg = InMemoryToolRegistry::with_builtins();
        let _ = reg.register(
            namespaced_def,
            ToolProvenance::HumanAuthored {
                author: "test".into(),
            },
        );
        Arc::new(reg)
    };
    let mut w = warrant(false);
    w.tool_grants.insert(ToolGrant {
        tool_id: kx_mote::ToolName(NS_TOOL.into()),
        tool_version: kx_mote::ToolVersion("1".into()),
    });

    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator_with(&dir, registry);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn0, &w, BARE_LEAF_NATIVE, worker).await;

    // (a) the bare leaf resolved to the NAMESPACED grant and froze a Tool fact.
    let facts = react_facts(&svc, &dir).await;
    assert_eq!(
        facts.len(),
        2,
        "anchor + the Tool settle (the bare leaf FIRED)"
    );
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { tool_id, tool_version },
                ..
            } if tool_id == NS_TOOL && tool_version == "1"
        ),
        "the bare `echo` resolves to the namespaced grant `{NS_TOOL}@1`"
    );

    // (b) the observation leases (declaring the namespaced tool) WITH its args, and
    // (c) commits — a bare-leaf model call against a namespaced grant genuinely fired.
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the observation is leasable");
    let item = &leased[0];
    let obs: Mote = item.mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        obs.def
            .tool_contract
            .get(&kx_mote::ToolName(NS_TOOL.into()))
            .map(|v| v.0.clone()),
        Some("1".to_string()),
        "the observation declares the resolved NAMESPACED tool"
    );
    let args = item
        .tool_args
        .as_ref()
        .expect("the observation leases WITH its validated args");
    assert_eq!(args.args_bytes, br#"{"q":"x"}"#.to_vec());
    commit_raw(&svc, &store, &obs, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the world-mutating observation COMMITTED — a bare-leaf dialed tool fired"
    );
}

/// PR-9a (the format-drift fail-closed invariant): an UNRECOGNIZED tool-shaped
/// completion under a GRANTING warrant commits as an ANSWER and fires NOTHING —
/// the SN-8 default. This is the durable invariant a format guard can hold: a
/// future model's novel tool-call syntax that NO parser arm recognizes never
/// mis-fires a tool; it degrades to an honest, committed answer (vs. the two
/// RECOGNIZED shapes — the JSON envelope and the Gemma-native delimiter — which DO
/// fire, proven by the two tests above).
#[tokio::test]
async fn unrecognized_tool_shape_under_grant_answers_and_fires_nothing() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED — yet an unparseable shape must NOT fire

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    // A made-up "tool call" in a format no parser arm recognizes — neither the JSON
    // envelope (must start with `{`) nor the Gemma-native `<|tool_call>` delimiter.
    commit_raw(
        &svc,
        &store,
        &turn0,
        &w,
        b"TOOL: mcp-echo(q=x)  # please run this for me",
        worker,
    )
    .await;

    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 2, "anchor + the Answer settle");
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Answer,
                ..
            }
        ),
        "an unrecognized tool-shape under a grant ANSWERS — it never mis-fires a tool"
    );
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "Answer is terminal — no observation was materialized, no tool fired"
    );
}

/// PR-9c-1 deferral pin (coordinator surface): a multi-element `{"tool_calls":[…,…]}`
/// body is NOT yet run in a single turn — one Tool fact per turn (the loop-semantics
/// change is deferred to its own PR). It degrades to a normal `Answer` at the SETTLE
/// authority site (not just the parser leaf), NEVER a silent first-element fire.
#[tokio::test]
async fn multi_element_tool_calls_settles_as_answer_not_fire() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // mcp-echo@1 GRANTED — yet a multi-element body must NOT fire

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(
        &svc,
        &store,
        &turn0,
        &w,
        br#"{"tool_calls":[{"name":"mcp-echo","arguments":{"q":"x"}},{"name":"mcp-echo","arguments":{"q":"y"}}]}"#,
        worker,
    )
    .await;

    let facts = react_facts(&svc, &dir).await;
    assert_eq!(facts.len(), 2, "anchor + the Answer settle");
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Answer,
                ..
            }
        ),
        "a multi-element tool_calls body is deferred — it ANSWERS, never fires a tool"
    );
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "no observation materialized — no tool fired (no silent first-element cap)"
    );
}

/// PR-9a (BUG-27 Path 2, end-to-end): when the tool a frozen `Tool` branch
/// references is DEREGISTERED before its observation can lease, the chain
/// DEAD-LETTERS (a loud terminal) instead of WEDGING forever — the pre-PR-9a
/// behavior re-materialized an unleaseable observation on every settle pass with
/// no terminal. Uses a `SqliteToolRegistry` (interior-mutable deregistration) so
/// the tool can be removed AFTER the settle freezes the `Tool` fact.
#[tokio::test]
async fn deregistering_a_tool_mid_chain_dead_letters_instead_of_wedging() {
    let dir = TempDir::new().unwrap();
    let reg =
        Arc::new(kx_tool_registry::SqliteToolRegistry::open(dir.path().join("tools.db")).unwrap());
    // Register mcp-echo@1 DURABLY (deregisterable — not a non-removable built-in).
    reg.register_durable(
        echo_tool_def(),
        kx_tool_registry::ToolProvenance::HumanAuthored {
            author: "test".into(),
        },
        None,
    )
    .unwrap();
    let (svc, store) = coordinator_with(&dir, reg.clone());
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    // Turn 0 proposes the tool ⇒ the settle freezes a Tool branch + materializes
    // the observation (Pending, NOT yet leased) — the tool is still registered.
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;
    let facts = react_facts(&svc, &dir).await;
    assert!(
        matches!(
            &facts[1],
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Tool { .. },
                ..
            }
        ),
        "the tool proposal froze a Tool branch"
    );

    // DEREGISTER the tool: the frozen branch now references a tool that is gone, so
    // the observation's args can never resolve (a PERMANENT fault).
    assert!(
        reg.deregister(
            &kx_mote::ToolName("mcp-echo".into()),
            &kx_mote::ToolVersion("1".into())
        )
        .unwrap(),
        "the tool was deregistered"
    );

    // The next drain: the lease arm skips the unresolvable observation (Permanent),
    // and the settle pass DEAD-LETTERS the chain (instead of re-materializing it
    // forever). The observation is never leased.
    let leased_after = common::lease_work(&svc, worker, MAC, 16).await;
    assert!(
        leased_after.is_empty(),
        "the unresolvable observation is never leased"
    );
    let facts = react_facts(&svc, &dir).await;
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                branch: ReactBranch::DeadLettered,
                ..
            }
        )),
        "the chain DEAD-LETTERED instead of wedging (BUG-27)"
    );
    // Terminal: bounded, no runaway re-lease of the retired observation.
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "terminal — the retired observation never re-enters the ready set"
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
/// (the default 6 tool calls), then dead-letters honestly — no runaway chain /
/// unbounded journal growth. PR-2d-2: each round now alternates turn → observation
/// (the observation FIRES even on the final tool call — the harness order: fire,
/// THEN bound the loop). The gate is the harness mirror: tool-budget first,
/// `>=`, fold-re-derived. W2 (this PR): a no-answer `Tool` tail at exhaustion now
/// freezes a LOUD terminal `DeadLettered` (was a silent quiesce) so a run-level
/// chain's terminal is honest (`agent run` → exit 1, not a masquerading timeout).
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
    // W2: the no-answer Tool tail at exhaustion freezes a LOUD terminal DeadLettered
    // (the honest terminal — never a silent quiesce that masquerades as a resumable
    // timeout). The terminal lands on the LAST tool turn (index max_tool_calls - 1).
    let facts = react_facts(&svc, &dir).await;
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                turn: 5,
                branch: ReactBranch::DeadLettered,
                ..
            }
        )),
        "a budget-exhausted Tool tail dead-letters honestly (W2) — no silent quiesce"
    );
    // Every recorded fact carries the durable caps the run was admitted under.
    for fact in facts {
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

/// Read a turn Mote's instruction (the bytes that ride `config_subset[PROMPT_KEY]`).
fn turn_prompt(mote: &Mote) -> String {
    mote.def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
        .map(|v| String::from_utf8_lossy(&v.0).into_owned())
        .unwrap_or_default()
}

const NUDGE_MARK: &str = "Do NOT call another tool";

/// W2 (settle-nudge): a model that proposes a `Tool` every turn gets ONE explicit
/// "answer now, do not call a tool" turn on its LAST useful round (turn index
/// `max_tool_calls - 1`), so a tool-looping model can settle instead of quiescing
/// answerless. Model-free + deterministic.
#[tokio::test]
async fn last_useful_turn_is_settle_nudged() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // default caps: 8 turns / 6 tool calls

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;

    let mut turn_prompts: Vec<String> = Vec::new();
    for _ in 0..24 {
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let Some(item) = leased.into_iter().next() else {
            break;
        };
        let mote: Mote = item.mote.unwrap().try_into().unwrap();
        if mote.def.tool_contract.is_empty() {
            // A TURN — record its instruction, then propose a tool again.
            turn_prompts.push(turn_prompt(&mote));
            commit_raw(&svc, &store, &mote, &w, TOOL_ENVELOPE, worker).await;
        } else {
            // The OBSERVATION fires its staged result.
            commit_raw(&svc, &store, &mote, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
        }
    }

    // 6 turns (max_tool_calls), and EXACTLY the last one (turn index 5) is nudged.
    assert_eq!(turn_prompts.len(), 6, "exactly max_tool_calls turns");
    let nudged: Vec<usize> = turn_prompts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.contains(NUDGE_MARK))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        nudged,
        vec![5],
        "exactly one nudged turn, on the last useful round (index max_tool_calls-1)"
    );
    assert!(
        turn_prompts[5].contains("give your FINAL answer"),
        "the nudged turn carries the answer-now steer"
    );
    // Bounded + honest terminal (the model never answered ⇒ DeadLettered, W2).
    assert!(common::lease_work(&svc, worker, MAC, 16).await.is_empty());
    assert!(
        react_facts(&svc, &dir).await.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                branch: ReactBranch::DeadLettered,
                ..
            }
        )),
        "a looping model that ignores the nudge dead-letters honestly"
    );
}

/// W2 (settle-nudge): the POSITIVE path — a model that loops on tools but heeds the
/// nudge on its last useful turn settles on an `Answer` (no dead-letter). This is
/// the user-visible value: a tool-looper that would have exited-3 now returns.
#[tokio::test]
async fn settle_nudge_lets_a_looping_model_answer() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;

    let mut answered_on_nudge = false;
    for _ in 0..24 {
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let Some(item) = leased.into_iter().next() else {
            break;
        };
        let mote: Mote = item.mote.unwrap().try_into().unwrap();
        if mote.def.tool_contract.is_empty() {
            // When the turn is the nudged one, ANSWER instead of calling a tool.
            if turn_prompt(&mote).contains(NUDGE_MARK) {
                answered_on_nudge = true;
                commit_raw(&svc, &store, &mote, &w, b"the final answer", worker).await;
            } else {
                commit_raw(&svc, &store, &mote, &w, TOOL_ENVELOPE, worker).await;
            }
        } else {
            commit_raw(&svc, &store, &mote, &w, br#"{"echoed":{"q":"x"}}"#, worker).await;
        }
    }

    assert!(answered_on_nudge, "the model reached the nudged turn");
    let facts = react_facts(&svc, &dir).await;
    // The chain settled on an Answer — NOT a dead-letter.
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                branch: ReactBranch::Answer,
                ..
            }
        )),
        "heeding the nudge settles the chain on an Answer"
    );
    assert!(
        !facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                branch: ReactBranch::DeadLettered,
                ..
            }
        )),
        "a settled chain never dead-letters"
    );
    assert!(common::lease_work(&svc, worker, MAC, 16).await.is_empty());
}

/// W2 vs A2 precedence: a chain that REJECTS every turn re-prompts with the
/// rejection reason and NEVER gets the settle-nudge — the reject arm takes
/// precedence (the nudge requires `prev_reject.is_none()`), so a model is never
/// told both "you were rejected" and "stop calling tools" in the same turn.
#[tokio::test]
async fn reject_tail_takes_precedence_over_the_nudge() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;

    let mut turn_prompts: Vec<String> = Vec::new();
    for _ in 0..16 {
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let Some(item) = leased.into_iter().next() else {
            break;
        };
        let turn: Mote = item.mote.unwrap().try_into().unwrap();
        turn_prompts.push(turn_prompt(&turn));
        // Every turn emits schema-invalid args → Rejected (the A2 re-prompt path).
        commit_raw(
            &svc,
            &store,
            &turn,
            &w,
            br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"zz":"x"}}}"#,
            worker,
        )
        .await;
    }

    // turns 1.. all re-prompt with REJECTED; NONE ever carries the settle-nudge.
    assert!(
        turn_prompts.iter().skip(1).all(|p| p.contains("REJECTED")),
        "every post-rejection turn carries the A2 re-prompt"
    );
    assert!(
        turn_prompts.iter().all(|p| !p.contains(NUDGE_MARK)),
        "a rejected tail never gets the settle-nudge (reject precedence)"
    );
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

/// PR-3 (A2): a malformed (committed-to-but-garbled) proposal is NOT terminal —
/// it freezes a `Rejected` round (reason names the malformation) and the chain
/// re-prompts the next turn (the model self-corrects), bounded by the budget.
#[tokio::test]
async fn malformed_committed_proposal_rejects_and_re_prompts() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // grants make the envelope path live

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Truncated envelope: committed to a call but malformed (the live gateway
    // fences this pre-commit; the substrate must STILL recover gracefully if such
    // bytes ever reach the journal — defense-in-depth).
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
    // turn-0 settled REJECTED (not DeadLettered), and a turn-1 Pending spawned.
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound { turn: 0, branch: ReactBranch::Rejected { reason }, .. }
                if reason.contains("malformed")
        )),
        "the malformed proposal freezes a Rejected round naming the malformation"
    );
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                turn: 1,
                branch: ReactBranch::Pending,
                ..
            }
        )),
        "the chain re-prompts: a next (turn-1) turn is materialized"
    );
    // The re-prompted turn IS leasable and carries the rejection steer.
    let next = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(next.len(), 1, "the re-prompted turn is leasable");
    let turn1: Mote = next[0].mote.clone().unwrap().try_into().unwrap();
    let prompt = turn1
        .def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
        .map(|v| String::from_utf8_lossy(&v.0).into_owned())
        .unwrap_or_default();
    assert!(
        prompt.contains("REJECTED"),
        "the re-prompted turn's instruction carries the rejection steer: {prompt}"
    );
}

/// PR-3 (A2): an UNGRANTED tool proposal (SN-8: the model cannot conjure a tool
/// the warrant withheld) freezes a `Rejected` round and re-prompts — the grant
/// set is never widened, but the model gets a bounded chance to pick a granted
/// tool or answer directly instead of the whole chain dying.
#[tokio::test]
async fn ungranted_proposal_rejects_and_re_prompts() {
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
    let facts = react_facts(&svc, &dir).await;
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound { turn: 0, branch: ReactBranch::Rejected { reason }, .. }
                if reason.contains("not granted")
        )),
        "an ungranted name freezes a Rejected round (SN-8 — never a grant widening)"
    );
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                turn: 1,
                branch: ReactBranch::Pending,
                ..
            }
        )),
        "the chain re-prompts the next turn"
    );
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

/// PR-3 (A2): a proposal whose ARGS fail the tool's typed `inputSchema` is
/// refused AT THE FREEZE (the settle's validate-at-freeze, the ONE authority
/// site) — the branch freezes `Rejected`, NEVER `Tool`, so no observation is
/// ever materialized and no effect can fire on schema-invalid args (D110.4).
/// The chain then re-prompts (A2), so the freeze invariant is preserved while
/// the model gets a bounded chance to fix its arguments.
#[tokio::test]
async fn schema_invalid_args_reject_at_the_freeze_then_re_prompt() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Granted tool, well-formed envelope — but the args violate the schema
    // (missing the required param, smuggling an undeclared key).
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
    // anchor Pending@0 + Rejected@0 + Pending@1 — NO Tool, NO observation.
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound { turn: 0, branch: ReactBranch::Rejected { reason }, .. }
                if reason.contains("inputSchema")
        )),
        "schema-invalid args freeze a Rejected round naming the inputSchema"
    );
    assert!(
        !facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                branch: ReactBranch::Tool { .. },
                ..
            }
        )),
        "a schema-invalid proposal NEVER freezes a Tool fact (the freeze invariant)"
    );
    // The re-prompted next turn is leasable; no observation Mote was made (the
    // only leasable work is the turn, never an observation).
    let next = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(next.len(), 1, "only the re-prompted turn is leasable");
    let turn1: Mote = next[0].mote.clone().unwrap().try_into().unwrap();
    assert!(
        turn1.def.tool_contract.is_empty(),
        "the leasable work is a TURN (no tool_contract), never an observation"
    );
}

/// PR-3 (A2) — the headline recovery: a bad-args proposal is REJECTED, the model
/// reads the reason on the re-prompted turn and ANSWERS, and the chain settles
/// with a real answer (no dead-letter). This is the live-tool-calling fix the
/// §2.246 campaign filed (A1 necessary-but-not-sufficient → A2).
#[tokio::test]
async fn bad_args_reject_then_the_model_recovers_with_an_answer() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();

    // Turn 0: bad args → Rejected → re-prompt.
    commit_raw(
        &svc,
        &store,
        &turn0,
        &w,
        br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"zz":"x"}}}"#,
        worker,
    )
    .await;

    // Turn 1: the re-prompted turn — the model now ANSWERS (no tool envelope).
    let next = common::lease_work(&svc, worker, MAC, 16).await;
    let turn1: Mote = next[0].mote.clone().unwrap().try_into().unwrap();
    commit_raw(&svc, &store, &turn1, &w, b"the answer is 42", worker).await;

    let facts = react_facts(&svc, &dir).await;
    assert!(
        facts.iter().any(|f| matches!(
            f,
            JournalEntry::ReactRound {
                turn: 0,
                branch: ReactBranch::Rejected { .. },
                ..
            }
        )),
        "turn 0 was rejected"
    );
    assert!(
        matches!(
            facts.last().unwrap(),
            JournalEntry::ReactRound {
                turn: 1,
                branch: ReactBranch::Answer,
                ..
            }
        ),
        "turn 1 recovered with an Answer — the chain settled with a real result"
    );
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "the chain is terminal on the answer"
    );
}

/// PR-3 (A2) — the loop bound: a model that emits a bad proposal EVERY turn is
/// bounded by the durable tool-call budget (each Rejected round spends one), then
/// dead-letters LOUDLY (BUG-27: terminal, never silent; GR15: never a fabricated
/// answer). This is the invariant that keeps graceful recovery from becoming an
/// infinite re-prompt wedge.
#[tokio::test]
async fn repeated_bad_args_exhaust_the_budget_then_dead_letter() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // default caps: 8 turns / 6 tool calls

    let (_, _) = submit_react(&svc, &seed_mote(), &w).await;
    let worker = common::register(&svc, "w").await;

    let mut rejected_turns = 0u32;
    for _ in 0..16 {
        let leased = common::lease_work(&svc, worker, MAC, 16).await;
        let Some(item) = leased.into_iter().next() else {
            break;
        };
        let turn: Mote = item.mote.unwrap().try_into().unwrap();
        // Every turn emits schema-invalid args → Rejected.
        commit_raw(
            &svc,
            &store,
            &turn,
            &w,
            br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"zz":"x"}}}"#,
            worker,
        )
        .await;
        rejected_turns += 1;
    }

    let facts = react_facts(&svc, &dir).await;
    let rejected = facts
        .iter()
        .filter(|f| {
            matches!(
                f,
                JournalEntry::ReactRound {
                    branch: ReactBranch::Rejected { .. },
                    ..
                }
            )
        })
        .count();
    // Bounded by the tool-call budget (6), not the turn budget (8) — every round
    // is a refused proposal, so the tool-call cap fires first.
    assert_eq!(
        rejected, 6,
        "exactly max_tool_calls Rejected rounds, then bounded"
    );
    assert_eq!(rejected_turns, 6, "no turn 7 ever spawned");
    assert!(
        matches!(
            facts.last().unwrap(),
            JournalEntry::ReactRound {
                branch: ReactBranch::DeadLettered,
                ..
            }
        ),
        "budget exhaustion freezes a LOUD terminal DeadLettered — never silent"
    );
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "the chain is dead — no re-fire, no runaway"
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

// ============================================================================
// PR-9b-2b — the deterministic-AGENTIC launch step (the `@tool` execution lane).
// ============================================================================

/// A deterministic-agentic LAUNCH step: a frozen-DAG MODEL mote (ROND +
/// StageThenCommit) carrying an author-declared tool-grant SET + the instruction +
/// the per-step budget. `budget = Some((turns, calls))` writes the caps into the
/// config (`react_seed_params` reads them); `None` defaults to 8/6.
fn launch_mote(parents: SmallVec<[ParentRef; 4]>, budget: Option<(u32, u32)>) -> Mote {
    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(INSTRUCTION.as_bytes().to_vec()),
    );
    if let Some((turns, calls)) = budget {
        config_subset.insert(
            ConfigKey(kx_mote::REACT_MAX_TURNS_KEY.to_string()),
            ConfigVal(serde_json::to_vec(&turns).unwrap()),
        );
        config_subset.insert(
            ConfigKey(kx_mote::REACT_MAX_TOOL_CALLS_KEY.to_string()),
            ConfigVal(serde_json::to_vec(&calls).unwrap()),
        );
    }
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(
        kx_mote::ToolName("mcp-echo".into()),
        kx_mote::ToolVersion("1".into()),
    );
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef([0x9b; 32]),
        model_id: ModelId(MODEL.into()),
        prompt_template_hash: PromptTemplateHash([0x9b; 32]),
        tool_contract,
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0x9b; 32]),
        GraphPosition(vec![0x9b]),
        parents,
    )
}

/// A plain PURE `> review` consumer of the launch step (its lone DAG parent),
/// leasable only AFTER the launch commits.
fn review_child(launch: &Mote) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef([0x5c; 32]),
        model_id: ModelId(MODEL.into()),
        prompt_template_hash: PromptTemplateHash([0x5c; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0x5c; 32]),
        GraphPosition(vec![0x5c]),
        std::iter::once(ParentRef {
            parent_id: launch.id,
            edge: EdgeMeta::data(),
        })
        .collect(),
    )
}

/// Submit `mote` plainly (`react_seed = false`) under the ALREADY-registered run —
/// the SubmitWorkflow-mote path (vs `common::submit`, which registers a fresh run).
async fn submit_plain(svc: &CoordinatorService, mote: &Mote, w: &WarrantSpec) {
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        kx_coordinator::proto::SubmitStatus::Accepted as i32
    );
}

/// Flagship: a deterministic-agentic launch step is PARKED at lease (never dispatched
/// as a plain model mote), its bounded reason→tool→observe loop is driven on a private
/// `step_salt`-keyed chain (a WORLD-MUTATING observation COMMITS — the PR-9a
/// effect-asserting pattern), and on the terminal Answer the LAUNCH mote COMMITS,
/// advancing the frozen DAG so its `> review` consumer becomes ready.
#[tokio::test]
async fn agentic_launch_drives_loop_commits_and_advances_dag() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true); // ROND + mcp-echo grant + the served route

    let _ = common::register_run(&svc, [0x5a; 32]).await;
    let launch = launch_mote(SmallVec::new(), None);
    let review = review_child(&launch);
    submit_plain(&svc, &launch, &w).await;
    submit_plain(&svc, &review, &w).await;

    // The launch is PARKED + the review child is Pending (parent uncommitted), so
    // `settle_agentic_launches` anchored the launch + materialized its salt-2 turn 0
    // — the ONLY leasable item.
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(
        leased.len(),
        1,
        "only the agentic turn 0 leases (launch parked, child pending)"
    );
    let turn0: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_ne!(
        turn0.id, launch.id,
        "the launch is NEVER dispatched; a server-derived salt-2 turn drives the loop"
    );
    let marker = turn0
        .def
        .config_subset
        .get(&ConfigKey(REACT_TURN_KEY.to_string()))
        .unwrap()
        .0
        .clone();
    assert_eq!(
        marker.len(),
        16 + 32,
        "the agentic marker is instance_id‖step_salt"
    );
    assert_eq!(
        &marker[16..],
        launch.id.as_bytes(),
        "step_salt = the launch MoteId"
    );

    // turn 0 proposes a tool call → settle freezes `Tool` + materializes the observation.
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;
    let (obs, args) = lease_observation(&svc, worker, &turn0).await;
    assert_eq!(
        args,
        br#"{"q":"x"}"#.to_vec(),
        "the observation leases WITH re-derived args"
    );
    // ★ the WORLD-MUTATING observation COMMITS (effect-asserting, not just "settled").
    commit_raw(&svc, &store, &obs, &w, b"echo: x", worker).await;
    assert_eq!(
        svc.state_of(obs.id).await.unwrap(),
        MoteState::Committed,
        "the tool observation committed (a real tool round fired)"
    );

    // turn 1 answers → settle freezes `Answer` → the LAUNCH mote commits.
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(
        leased.len(),
        1,
        "turn 1 leases after the observation commits"
    );
    let turn1: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_ne!(turn1.id, turn0.id);
    // ★ the F-7 trajectory is NON-EMPTY — turn 1's 48-byte agentic marker decoded to
    // the right `(instance_id, step_salt)` chain + the observation was rebuilt via
    // `build_agentic_tool` (a 16-byte-only decode would yield an EMPTY transcript — the
    // silent loop-break this guards). Interleaved `[turn0, obs0]`, transcript order.
    let traj = &leased[0].parent_results;
    assert_eq!(
        traj.len(),
        2,
        "turn 1 sees [turn0, obs0] (the agentic marker decoded)"
    );
    assert_eq!(traj[0].parent_mote_id, turn0.id.as_bytes().to_vec());
    assert_eq!(traj[1].parent_mote_id, obs.id.as_bytes().to_vec());
    commit_raw(&svc, &store, &turn1, &w, b"All done.", worker).await;

    // The launch COMMITTED (carrying the answer turn's result) ⇒ the frozen DAG
    // advanced ⇒ the `> review` consumer is now leasable.
    assert_eq!(
        svc.state_of(launch.id).await.unwrap(),
        MoteState::Committed,
        "the agentic launch step committed — the DAG advanced"
    );
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the `> review` child is now ready");
    let child: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        child.id, review.id,
        "the launch's DAG consumer leased only after the launch committed"
    );

    // Every fact of this chain is an AGENTIC fact (keyed by the launch's step_salt) —
    // disjoint from any run-level react chain in the shared journal.
    let facts = react_facts(&svc, &dir).await;
    assert!(
        facts.iter().all(|f| matches!(
            f,
            JournalEntry::ReactRound { step_salt: Some(s), .. } if s == launch.id.as_bytes()
        )),
        "every agentic ReactRound fact carries step_salt = the launch MoteId"
    );
}

/// Budget-exhaust: a launch whose loop only ever tool-calls (never answers) within its
/// declared budget fails CLOSED — the launch mote dead-letters (terminal `Failed`),
/// never fabricating an answer (GR15) and never wedging its DAG consumer in `Scheduled`.
#[tokio::test]
async fn agentic_launch_budget_exhaust_dead_letters() {
    let dir = TempDir::new().unwrap();
    let (svc, store) = coordinator(&dir);
    let w = warrant(true);

    let _ = common::register_run(&svc, [0x5b; 32]).await;
    // A 2-turn / 1-tool-call budget: after the first tool round commits, the chain is
    // budget-exhausted with no Answer.
    let launch = launch_mote(SmallVec::new(), Some((2, 1)));
    submit_plain(&svc, &launch, &w).await;

    let worker = common::register(&svc, "w").await;
    let turn0: Mote = common::lease_work(&svc, worker, MAC, 16).await[0]
        .mote
        .clone()
        .unwrap()
        .try_into()
        .unwrap();
    commit_raw(&svc, &store, &turn0, &w, TOOL_ENVELOPE, worker).await;
    let (obs, _) = lease_observation(&svc, worker, &turn0).await;
    commit_raw(&svc, &store, &obs, &w, b"echo: x", worker).await;

    // The tool budget (1) is now exhausted with no Answer ⇒ the launch dead-letters.
    assert_eq!(
        svc.state_of(launch.id).await.unwrap(),
        MoteState::Failed,
        "a budget-exhausted agentic launch fails closed (no fabricated answer)"
    );
    // No further work leases (the chain is terminal; nothing wedged in Scheduled).
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "a dead-lettered launch leaves no leasable work"
    );
}

/// A parked agentic launch whose DAG parent TERMINALLY fails is dead-lettered + its
/// in-memory park reclaimed — a `Failed` parent never makes the launch ready (it is not
/// `Committed`), so without fail-closing the launch would sit `Pending` forever and its
/// `parked_launches`/`dispatch.defs` entries would leak for the life of a long-lived serve.
#[tokio::test]
async fn agentic_launch_with_failed_parent_is_dead_lettered() {
    let dir = TempDir::new().unwrap();
    let (svc, _store) = coordinator(&dir);
    let w = warrant(true);

    let _ = common::register_run(&svc, [0x5d; 32]).await;
    // A plain producer P (root model mote, empty tool_contract ⇒ not a launch) + the
    // agentic launch wired downstream of it.
    let parent = seed_mote();
    let launch = launch_mote(
        std::iter::once(ParentRef {
            parent_id: parent.id,
            edge: EdgeMeta::data(),
        })
        .collect(),
        None,
    );
    submit_plain(&svc, &parent, &w).await;
    submit_plain(&svc, &launch, &w).await;

    // Only P leases (the launch is parked, its parent uncommitted). Lease P, then
    // TERMINALLY fail it.
    let worker = common::register(&svc, "w").await;
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "only the producer leases (launch parked)");
    let p: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(p.id, parent.id);
    common::report_failure(
        &svc,
        &p,
        worker,
        kx_coordinator::proto::FailureReason::DeadLettered,
    )
    .await
    .unwrap();

    // The next drain's settle sees the launch's parent terminally failed ⇒ dead-letters
    // the launch (it can NEVER become ready) + reclaims the park — no leaked entry.
    let _ = svc.committed_count().await; // ordering barrier (a later drain runs the settle)
    assert_eq!(
        svc.state_of(launch.id).await.unwrap(),
        MoteState::Failed,
        "the launch dead-lettered — its parent can never satisfy it (no Pending-forever leak)"
    );
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "nothing leasable — the park + def were reclaimed, not leaked"
    );
}
