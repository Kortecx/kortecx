//! Commit-path throughput benchmark — the precise, repeatable measurement behind
//! the group-commit work (run with `cargo bench -p kx-coordinator`).
//!
//! Four axes, all measuring the `ReportCommit` path end-to-end through the gRPC
//! service trait (assemble → channel → owner thread → `append_batch` → fold →
//! reply):
//!
//! - **in-memory vs on-disk** journal — isolates the fsync cost group commit
//!   amortizes (in-memory has no fsync; on-disk runs `synchronous=FULL`);
//! - **sequential vs concurrent** — sequential awaits each commit (one transaction
//!   per commit); concurrent fans out so the owner thread coalesces them into
//!   shared transactions (group commit).
//!
//! The on-disk concurrent vs sequential gap is the group-commit win; it scales
//! with the platform's real fsync cost (small on macOS's weak fsync, large on a
//! Linux disk with true `F_FULLFSYNC`-grade flushes).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    missing_docs
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use kx_content::ContentRef;
use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::CoordinatorService;
use kx_journal::SqliteJournal;
use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    NdClass, ParentRef, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::Request;

fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 4096,
            max_output_tokens: 512,
            max_calls: 1,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::MacOsSandbox,
    }
}

fn mote(index: u64) -> Mote {
    let mut input = [0u8; 32];
    input[..8].copy_from_slice(&index.to_le_bytes());
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams {
            max_output_tokens: 256,
            temperature_bps: 0,
            top_p_bps: 9000,
            top_k: 40,
            seed: 1,
            stop_tokens: SmallVec::new(),
            grammar: None,
        },
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes(input),
        GraphPosition(index.to_le_bytes().to_vec()),
        SmallVec::<[ParentRef; 4]>::new(),
    )
}

fn report_req(m: &Mote, worker_id: u64) -> proto::ReportCommitRequest {
    let id = m.id.as_bytes().to_vec();
    proto::ReportCommitRequest {
        mote_id: id.clone(),
        idempotency_key: id,
        result_ref: vec![3u8; 32],
        warrant_ref: vec![4u8; 32],
        mote_def_hash: vec![5u8; 32],
        nd_class: proto::NdClass::Pure as i32,
        parents: vec![],
        worker_id,
    }
}

/// Fresh coordinator + `n` submitted Motes + a registered worker. The on-disk
/// variant gets a unique journal file inside `dir` per call (so each iteration
/// starts clean).
fn setup(
    rt: &tokio::runtime::Runtime,
    n: u64,
    dir: Option<&TempDir>,
) -> (CoordinatorService, Vec<Mote>, u64) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let svc = match dir {
        Some(d) => {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path: PathBuf = d.path().join(format!("bench-{id}.db"));
            CoordinatorService::new(SqliteJournal::open(path).unwrap())
        }
        None => CoordinatorService::new(SqliteJournal::open_in_memory().unwrap()),
    };
    let motes: Vec<Mote> = (0..n).map(mote).collect();
    let w = warrant();
    let worker = rt.block_on(async {
        let worker = svc
            .register_worker(Request::new(proto::RegisterWorkerRequest {
                executor_class: proto::ExecutorClass::MacosSandbox as i32,
                endpoint: "bench".into(),
            }))
            .await
            .unwrap()
            .into_inner()
            .worker_id;
        for m in &motes {
            svc.submit_mote(Request::new(proto::SubmitMoteRequest {
                mote: Some(m.clone().into()),
                warrant: Some(w.clone().into()),
            }))
            .await
            .unwrap();
        }
        worker
    });
    (svc, motes, worker)
}

fn commit_all(
    rt: &tokio::runtime::Runtime,
    svc: &CoordinatorService,
    motes: &[Mote],
    worker: u64,
    concurrent: bool,
) {
    rt.block_on(async {
        if concurrent {
            let mut handles = Vec::with_capacity(motes.len());
            for m in motes {
                let s = svc.clone();
                let req = report_req(m, worker);
                handles.push(tokio::spawn(async move {
                    s.report_commit(Request::new(req)).await
                }));
            }
            for h in handles {
                h.await.unwrap().unwrap();
            }
        } else {
            for m in motes {
                svc.report_commit(Request::new(report_req(m, worker)))
                    .await
                    .unwrap();
            }
        }
    });
}

fn bench_commit(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let n: u64 = 512;

    let mut group = c.benchmark_group("report_commit");
    group.throughput(Throughput::Elements(n));

    for (backend, on_disk) in [("in_memory", false), ("on_disk", true)] {
        let dir = on_disk.then(|| TempDir::new().unwrap());
        for (shape, concurrent) in [("sequential", false), ("concurrent", true)] {
            group.bench_with_input(
                BenchmarkId::new(format!("{backend}_{shape}"), n),
                &n,
                |b, &n| {
                    b.iter_batched(
                        || setup(&rt, n, dir.as_ref()),
                        |(svc, motes, worker)| commit_all(&rt, &svc, &motes, worker, concurrent),
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_commit);
criterion_main!(benches);
