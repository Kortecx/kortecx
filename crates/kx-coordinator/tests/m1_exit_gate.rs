//! The M1 EXIT GATE (v2 pivot, D63/D64/D66/D79): with M1.1 (run registration) +
//! M1.2 (resolved-version metadata + run-scoped token) + M1.3 (submission refusals
//! + registration-before-submit) all wired, the following hold end-to-end:
//!
//! 1. a re-submitted identical recipe produces a FRESH registered run (new id);
//! 2. the recipe fingerprint round-trips as discovery metadata (≠ identity);
//! 3. resolved versions appear as off-DAG metadata;
//! 4. a tool-resolution miss is refused (D66 fail-closed);
//! 5. cross-boundary exactly-once: the lease surfaces the run id (the run-scoped
//!    token root), a double commit dedupes to one, and a distinct run of the same
//!    recipe commits independently (per-run namespacing).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, RunNonceSource, WorkerRegistry,
};
use kx_journal::{InMemoryJournal, Journal};
use kx_mote::{EffectPattern, ModelId, NdClass, ToolName, ToolVersion};
use kx_warrant::{
    ExecutorClass, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolGrant,
    WarrantSpec,
};
use tonic::Request;

#[derive(Debug)]
struct FixedNonce([u8; 16]);
impl RunNonceSource for FixedNonce {
    fn fresh_instance_id(&self) -> [u8; 16] {
        self.0
    }
}
#[derive(Debug)]
struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.0
    }
}

fn coordinator<J: Journal + Send + 'static>(
    journal: J,
    instance_id: [u8; 16],
) -> CoordinatorService {
    let registry: Arc<dyn WorkerRegistry> = Arc::new(InMemoryWorkerRegistry::new());
    CoordinatorService::with_seams(
        journal,
        registry,
        None,
        Arc::new(FixedClock(7)),
        Arc::new(FixedNonce(instance_id)),
    )
}

fn warrant_granting(tools: &[(&str, &str)], model_id: &str) -> WarrantSpec {
    let tool_grants: BTreeSet<ToolGrant> = tools
        .iter()
        .map(|(id, ver)| ToolGrant {
            tool_id: ToolName((*id).into()),
            tool_version: ToolVersion((*ver).into()),
        })
        .collect();
    let mut mounts = BTreeMap::new();
    mounts.insert(
        std::path::PathBuf::from("/tmp/in"),
        kx_warrant::FsMode::ReadOnly,
    );
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())])),
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId(model_id.into()),
            max_input_tokens: 4096,
            max_output_tokens: 512,
            max_calls: 3,
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
        ..Default::default()
    }
}

async fn register(svc: &CoordinatorService, fp: [u8; 32]) -> Vec<u8> {
    svc.register_run(Request::new(proto::RegisterRunRequest {
        recipe_fingerprint: fp.to_vec(),
    }))
    .await
    .unwrap()
    .into_inner()
    .instance_id
}

// === Clauses 1–3: re-submit → fresh run; fingerprint matches; metadata present =

#[tokio::test]
async fn resubmitting_a_recipe_starts_a_fresh_registered_run_with_metadata() {
    let recipe = [0x9cu8; 32]; // the SAME recipe fingerprint for both runs
    let mote = common::mote(1, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("fs-read", "1")], "qwen2.5-0.5b");

    // Run A.
    let id_a = [0xa1u8; 16];
    let svc_a = coordinator(InMemoryJournal::new(), id_a);
    let reg_a = register(&svc_a, recipe).await;
    assert_eq!(reg_a, id_a.to_vec());
    let resp_a = common::submit(&svc_a, &mote, &warrant).await;
    assert_eq!(resp_a.status, proto::SubmitStatus::Accepted as i32);
    assert_eq!(
        resp_a.instance_id,
        id_a.to_vec(),
        "the response carries the run id"
    );

    // Clause 2 — fingerprint round-trips (discovery metadata, not identity).
    assert_eq!(
        svc_a.run_registration().await.unwrap(),
        Some((id_a, recipe))
    );
    // Clause 3 — resolved versions captured as metadata.
    let recs = svc_a.run_resolved_versions().await.unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].instance_id, id_a, "metadata anchored to the run id");

    // Run B — SAME recipe, fresh journal + fresh nonce.
    let id_b = [0xb2u8; 16];
    let svc_b = coordinator(InMemoryJournal::new(), id_b);
    let reg_b = register(&svc_b, recipe).await;
    let resp_b = common::submit(&svc_b, &mote, &warrant).await;
    assert_eq!(resp_b.status, proto::SubmitStatus::Accepted as i32);

    // Clause 1 — re-running the SAME recipe yields a DISTINCT registered identity.
    assert_ne!(
        reg_a, reg_b,
        "a re-submitted recipe starts a fresh registered run"
    );
    assert_eq!(
        svc_b.run_registration().await.unwrap(),
        Some((id_b, recipe))
    );
}

// === Clause 4: a tool-resolution miss is refused ==============================

#[tokio::test]
async fn world_mutating_resolution_miss_is_refused() {
    let svc = coordinator(InMemoryJournal::new(), [0xc3u8; 16]);
    register(&svc, [0x4cu8; 32]).await;
    let wm = common::wm_mote(2, EffectPattern::StageThenCommit);
    let warrant = warrant_granting(&[("ghost", "1")], "m"); // unresolvable

    let resp = common::submit(&svc, &wm, &warrant).await;
    assert_eq!(resp.status, proto::SubmitStatus::Rejected as i32);
    assert_eq!(svc.committed_count().await.unwrap(), 0, "nothing committed");
}

// === Clause 5: cross-boundary exactly-once under the run-scoped token =========

#[tokio::test]
async fn cross_boundary_exactly_once_per_run() {
    let id_a = [0xd4u8; 16];
    let svc_a = coordinator(InMemoryJournal::new(), id_a);
    register(&svc_a, [0x5du8; 32]).await;
    let worker_a = common::register(&svc_a, "wa").await;

    let wm = common::wm_mote(3, EffectPattern::StageThenCommit);
    let warrant = warrant_granting(&[("fs-write", "1")], "m"); // fs-write (Staged) resolves
    assert_eq!(
        common::submit(&svc_a, &wm, &warrant).await.status,
        proto::SubmitStatus::Accepted as i32
    );

    // The lease surfaces the run id → the worker derives the run-scoped token.
    let lease = svc_a
        .lease_work(Request::new(proto::LeaseWorkRequest {
            worker_id: worker_a,
            executor_class: proto::ExecutorClass::MacosSandbox as i32,
            max_motes: 16,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(lease.items.len(), 1, "the WM Mote is leased");
    assert_eq!(
        lease.instance_id,
        id_a.to_vec(),
        "LeaseWork carries the run-scoped token root"
    );

    // A double commit dedupes to exactly one (journal first-wins by identity).
    assert_eq!(
        common::commit(&svc_a, &wm, worker_a).await.outcome,
        proto::CommitOutcome::Committed as i32
    );
    assert_eq!(
        common::commit(&svc_a, &wm, worker_a).await.outcome,
        proto::CommitOutcome::AlreadyCommitted as i32
    );
    assert_eq!(
        svc_a.committed_count().await.unwrap(),
        1,
        "exactly-once within the run"
    );

    // A DISTINCT run of the SAME Mote def commits independently (per-run namespacing:
    // the run-scoped token root id_b ≠ id_a, so cross-boundary effects do not collide).
    let id_b = [0xe5u8; 16];
    let svc_b = coordinator(InMemoryJournal::new(), id_b);
    register(&svc_b, [0x5du8; 32]).await;
    let worker_b = common::register(&svc_b, "wb").await;
    common::submit(&svc_b, &wm, &warrant).await;
    assert_eq!(
        common::commit(&svc_b, &wm, worker_b).await.outcome,
        proto::CommitOutcome::Committed as i32
    );
    assert_eq!(
        svc_b.committed_count().await.unwrap(),
        1,
        "the distinct run has its own commit"
    );
    assert_ne!(
        id_a, id_b,
        "the two runs have distinct run-scoped token roots"
    );
}
