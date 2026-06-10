//! PR-2d-2 react spikes (Golden Rule 10): **M7a** — a live `ReAct` chain's
//! submit→`Answer`-settle latency; **M7b** — one full TOOL round (turn commit →
//! settle freezes the `Tool` fact → the observation leases WITH the
//! coordinator-validated args → the REAL `StdioTransport` fires the bundled
//! `kx-mcp-echo` bin through the REAL `LocalCapabilityBroker` warrant gate →
//! the observation commits → the chain advances → the next turn answers).
//!
//! Driven at the COORDINATOR layer, model-free (turn outputs are staged
//! directly — the `react_live.rs` pattern), so the spike measures the RUNTIME's
//! settle/materialize/lease/fire machinery, never model decode time, and stays
//! FFI-free (any contributor can profile their box). All timing is at the
//! client/dispatch boundary — never inside the sole-writer commit path or the
//! digest fold (Golden Rule 10(b)). M7b runtime-skips (empty samples) when the
//! bundled bin is absent — `just profile` builds it first.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::proto::{CommitOutcome, ExecutorClass as ProtoExecutorClass};
use kx_coordinator::{CoordinatorService, InMemoryWorkerRegistry, WorkerRegistry};
use kx_journal::{Journal, JournalEntry, ReactBranch, SqliteJournal};
use kx_mcp::{McpCapability, StdioTransport};
use kx_mote::{
    ConfigKey, ConfigVal, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
    PROMPT_KEY,
};
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, InputSchema, McpEndpointId, ParamSpec, ParamType,
    ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::Request;

use crate::error::ProfileError;

const MAC_OR_LINUX: ProtoExecutorClass = ProtoExecutorClass::MacosSandbox;
const MODEL: &str = "kx-profile-react";
const ENVELOPE: &[u8] = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"q":"x"}}}"#;
const SETTLE_POLL: Duration = Duration::from_millis(2);
const SETTLE_TRIES: u32 = 2_500; // ≤ 5 s per settle wait

/// Raw per-iteration latency samples (milliseconds).
#[derive(Debug, Clone)]
pub struct ReactSamples {
    /// M7a — react submit (seed-swap + anchor) → the `Answer` fact folded.
    pub answer_ms: Vec<f64>,
    /// M7b — turn-0 envelope commit → tool round (observation materialize +
    /// REAL echo fire + commit + advance) → turn-1 answer settled. EMPTY when
    /// the bundled `kx-mcp-echo` bin is unavailable.
    pub tool_round_ms: Vec<f64>,
}

/// Measure M7a/M7b over `iterations` fresh coordinators (fresh journal/store
/// per iteration — no cross-run dedup).
///
/// # Errors
/// Returns [`ProfileError`] on a coordinator/journal/broker failure or a
/// settle timeout.
pub async fn measure(iterations: usize) -> Result<ReactSamples, ProfileError> {
    let echo_bin = echo_binary_path();
    if echo_bin.is_none() {
        eprintln!(
            "kx-profile: kx-mcp-echo bin not found — M7b react_tool_round skipped \
             (cargo build -p kx-mcp, or set KX_MCP_ECHO_PATH)"
        );
    }
    let mut answer_ms = Vec::with_capacity(iterations);
    let mut tool_round_ms = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        // M7a — answer-only chain: submit → lease → prose commit → Answer fact.
        {
            let dir = tempdir()?;
            let (svc, store) = coordinator(&dir)?;
            let worker = register(&svc).await?;
            let t0 = Instant::now();
            submit_react(&svc, &warrant(false)).await?;
            let turn0 = lease_one(&svc, worker).await?;
            commit_raw(&svc, &store, &turn0, &warrant(false), b"the answer", worker).await?;
            wait_for_branch(&svc, &dir, |b| matches!(b, ReactBranch::Answer)).await?;
            answer_ms.push(elapsed_ms(t0));
        }

        // M7b — one full tool round, the REAL broker + stdio fire in the middle.
        if let Some(bin) = &echo_bin {
            let dir = tempdir()?;
            let (svc, store) = coordinator(&dir)?;
            let broker = echo_broker(&store, bin);
            let worker = register(&svc).await?;
            let w = warrant(true);
            submit_react(&svc, &w).await?;
            let turn0 = lease_one(&svc, worker).await?;

            let t0 = Instant::now();
            commit_raw(&svc, &store, &turn0, &w, ENVELOPE, worker).await?;
            // The settle freezes Tool + materializes the observation; lease it
            // (it carries the coordinator-validated args) and FIRE for real.
            let (obs, args) = lease_observation(&svc, worker).await?;
            let staged_ref = fire_echo(&broker, &obs, &w, args)?;
            report_commit_ref(&svc, &obs, &w, staged_ref, worker).await?;
            // The advance spawns turn 1; a prose commit settles the chain.
            let turn1 = lease_one(&svc, worker).await?;
            commit_raw(&svc, &store, &turn1, &w, b"done", worker).await?;
            wait_for_branch(&svc, &dir, |b| matches!(b, ReactBranch::Answer)).await?;
            tool_round_ms.push(elapsed_ms(t0));
        }
    }

    Ok(ReactSamples {
        answer_ms,
        tool_round_ms,
    })
}

fn tempdir() -> Result<TempDir, ProfileError> {
    TempDir::new().map_err(|e| ProfileError::Gateway(e.to_string()))
}

fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1_000.0
}

/// A coordinator with the react-capable registry (built-ins + `mcp-echo@1`
/// with its typed schema) over a fresh journal + content store under `dir`.
fn coordinator(
    dir: &TempDir,
) -> Result<(CoordinatorService, Arc<LocalFsContentStore>), ProfileError> {
    let store = Arc::new(
        LocalFsContentStore::open(dir.path().join("content"))
            .map_err(|e| ProfileError::Gateway(e.to_string()))?,
    );
    let journal = SqliteJournal::open(dir.path().join("journal.db"))
        .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    let registry: Arc<dyn WorkerRegistry> = Arc::new(InMemoryWorkerRegistry::new());
    let svc = CoordinatorService::with_shaper_materialization(
        journal,
        registry,
        store.clone(),
        Arc::new(kx_coordinator::SystemClock),
        Arc::new(kx_coordinator::OsRandomNonce),
        Arc::new(registry_with_echo()),
        Arc::new(kx_warrant::InMemoryRoleRegistry::new()),
    );
    Ok((svc, store))
}

fn registry_with_echo() -> InMemoryToolRegistry {
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        ToolDef {
            tool_id: ToolName("mcp-echo".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Mcp {
                endpoint: McpEndpointId("stdio://kx-mcp-echo".into()),
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
            description: "profile echo".into(),
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
            author: "kx-profile".into(),
        },
    );
    reg
}

/// The chain warrant; `granted` adds the `mcp-echo@1` grant (the M7b shape).
fn warrant(granted: bool) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    if granted {
        tool_grants.insert(ToolGrant {
            tool_id: ToolName("mcp-echo".into()),
            tool_version: ToolVersion("1".into()),
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

/// The client's SEED Mote (identity advisory — the coordinator swaps it).
fn seed_mote() -> Mote {
    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(b"profile the chain".to_vec()),
    );
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7; 32]),
        model_id: ModelId(MODEL.into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([7; 32]),
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
        InputDataId::from_bytes([7; 32]),
        GraphPosition(vec![7]),
        SmallVec::new(),
    )
}

async fn register(svc: &CoordinatorService) -> Result<u64, ProfileError> {
    let resp = svc
        .register_worker(Request::new(kx_coordinator::proto::RegisterWorkerRequest {
            executor_class: MAC_OR_LINUX as i32,
            endpoint: "inproc://kx-profile".into(),
        }))
        .await
        .map_err(|s| status_err(&s))?
        .into_inner();
    Ok(resp.worker_id)
}

async fn submit_react(svc: &CoordinatorService, w: &WarrantSpec) -> Result<(), ProfileError> {
    let _ = svc
        .register_run(Request::new(kx_coordinator::proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5a; 32],
        }))
        .await
        .map_err(|s| status_err(&s))?;
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(seed_mote().into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: true,
        }))
        .await
        .map_err(|s| status_err(&s))?
        .into_inner();
    if resp.status != kx_coordinator::proto::SubmitStatus::Accepted as i32 {
        return Err(ProfileError::Gateway("react seed not accepted".into()));
    }
    Ok(())
}

/// Lease until exactly one item is offered; convert its Mote.
async fn lease_one(svc: &CoordinatorService, worker: u64) -> Result<Mote, ProfileError> {
    for _ in 0..SETTLE_TRIES {
        let resp = svc
            .lease_work(Request::new(kx_coordinator::proto::LeaseWorkRequest {
                worker_id: worker,
                executor_class: MAC_OR_LINUX as i32,
                max_motes: 16,
            }))
            .await
            .map_err(|s| status_err(&s))?
            .into_inner();
        if let Some(item) = resp.items.into_iter().next() {
            let mote: Mote = item
                .mote
                .ok_or_else(|| ProfileError::Gateway("leased item missing mote".into()))?
                .try_into()
                .map_err(|e| ProfileError::Gateway(format!("mote convert: {e}")))?;
            return Ok(mote);
        }
        tokio::time::sleep(SETTLE_POLL).await;
    }
    Err(ProfileError::Timeout {
        what: "a leasable react Mote".into(),
        elapsed_ms: 5_000,
    })
}

/// Lease until the OBSERVATION is offered; return it with its validated args.
async fn lease_observation(
    svc: &CoordinatorService,
    worker: u64,
) -> Result<(Mote, Vec<u8>), ProfileError> {
    for _ in 0..SETTLE_TRIES {
        let resp = svc
            .lease_work(Request::new(kx_coordinator::proto::LeaseWorkRequest {
                worker_id: worker,
                executor_class: MAC_OR_LINUX as i32,
                max_motes: 16,
            }))
            .await
            .map_err(|s| status_err(&s))?
            .into_inner();
        if let Some(item) = resp.items.into_iter().next() {
            let args = item
                .tool_args
                .as_ref()
                .map(|ta| ta.args_bytes.clone())
                .ok_or_else(|| {
                    ProfileError::Gateway("react observation leased without args".into())
                })?;
            let mote: Mote = item
                .mote
                .ok_or_else(|| ProfileError::Gateway("leased item missing mote".into()))?
                .try_into()
                .map_err(|e| ProfileError::Gateway(format!("mote convert: {e}")))?;
            return Ok((mote, args));
        }
        tokio::time::sleep(SETTLE_POLL).await;
    }
    Err(ProfileError::Timeout {
        what: "the react observation to lease".into(),
        elapsed_ms: 5_000,
    })
}

/// The broker holding the REAL `McpCapability` over the bundled stdio bin.
fn echo_broker(
    store: &LocalFsContentStore,
    bin: &std::path::Path,
) -> LocalCapabilityBroker<LocalFsContentStore> {
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(McpCapability::new(
        ToolName("mcp-echo".into()),
        ToolVersion("1".into()),
        McpEndpointId("stdio://kx-mcp-echo".into()),
        "echo",
        Box::new(StdioTransport::new(bin.to_string_lossy().as_ref())),
    )));
    broker
}

/// Fire the observation through the broker's warrant gate (the worker's
/// `run_wm` shape: the leased args become the payload; egress `None`).
fn fire_echo(
    broker: &LocalCapabilityBroker<LocalFsContentStore>,
    obs: &Mote,
    w: &WarrantSpec,
    args: Vec<u8>,
) -> Result<ContentRef, ProfileError> {
    let request = kx_capability::EffectRequest {
        payload: args,
        pattern: obs.effect_pattern(),
        idempotency_key: Some(kx_capability::idempotency_token_for(obs)),
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
        secret_scope: kx_warrant::SecretScope::None,
    };
    let handle = broker
        .dispatch(obs, w, &ToolName("mcp-echo".into()), request)
        .map_err(|e| ProfileError::Gateway(format!("echo dispatch: {e}")))?;
    Ok(handle.staged_ref)
}

/// Commit `bytes` as `mote`'s result (stage into the shared store first).
async fn commit_raw(
    svc: &CoordinatorService,
    store: &LocalFsContentStore,
    mote: &Mote,
    w: &WarrantSpec,
    bytes: &[u8],
    worker: u64,
) -> Result<(), ProfileError> {
    let result_ref = store
        .put(bytes)
        .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    report_commit_ref(svc, mote, w, result_ref, worker).await
}

async fn report_commit_ref(
    svc: &CoordinatorService,
    mote: &Mote,
    w: &WarrantSpec,
    result_ref: ContentRef,
    worker: u64,
) -> Result<(), ProfileError> {
    let id = mote.id.as_bytes().to_vec();
    let outcome = svc
        .report_commit(Request::new(kx_coordinator::proto::ReportCommitRequest {
            mote_id: id.clone(),
            idempotency_key: id,
            result_ref: result_ref.as_bytes().to_vec(),
            warrant_ref: warrant_ref_of(w).as_bytes().to_vec(),
            mote_def_hash: mote.def.hash().as_bytes().to_vec(),
            nd_class: kx_coordinator::proto::NdClass::from(mote.def.nd_class) as i32,
            parents: mote.parents.iter().map(|p| (*p).into()).collect(),
            worker_id: worker,
        }))
        .await
        .map_err(|s| status_err(&s))?
        .into_inner()
        .outcome;
    if outcome != CommitOutcome::Committed as i32 {
        return Err(ProfileError::Gateway("commit not accepted".into()));
    }
    Ok(())
}

/// Poll the durable react facts (a second `SQLite` reader; `committed_count` as
/// the drain-ordering barrier) until a branch matches `pred`.
async fn wait_for_branch(
    svc: &CoordinatorService,
    dir: &TempDir,
    pred: impl Fn(&ReactBranch) -> bool,
) -> Result<(), ProfileError> {
    for _ in 0..SETTLE_TRIES {
        let _ = svc.committed_count().await; // ordering barrier (a later drain)
        let journal = SqliteJournal::open(dir.path().join("journal.db"))
            .map_err(|e| ProfileError::Gateway(e.to_string()))?;
        let head = journal
            .current_seq()
            .map_err(|e| ProfileError::Gateway(e.to_string()))?;
        let matched = journal
            .read_entries_by_seq(0..head + 1)
            .map_err(|e| ProfileError::Gateway(e.to_string()))?
            .any(|e| matches!(&e, JournalEntry::ReactRound { branch, .. } if pred(branch)));
        if matched {
            return Ok(());
        }
        tokio::time::sleep(SETTLE_POLL).await;
    }
    Err(ProfileError::Timeout {
        what: "the react chain to settle".into(),
        elapsed_ms: 5_000,
    })
}

fn status_err(s: &tonic::Status) -> ProfileError {
    ProfileError::Gateway(s.to_string())
}

/// Locate the bundled `kx-mcp-echo` bin (`KX_MCP_ECHO_PATH` → target walk).
fn echo_binary_path() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("KX_MCP_ECHO_PATH") {
        let path = PathBuf::from(over);
        if path.exists() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["release", "debug"] {
                let candidate = ancestor.join(profile).join("kx-mcp-echo");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
