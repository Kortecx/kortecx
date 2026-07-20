//! Resolved-version run metadata (M1.2, D79): at submit, the coordinator resolves
//! the warrant's `tool_grants` + reads `model_id` and captures them as an off-DAG
//! `RunVersionsResolved` journal fact anchored to the run's `instance_id` —
//! **metadata, never identity**. The run's `instance_id` is surfaced on the
//! `SubmitMote` response (the resume key) and on `LeaseWork` (the worker derives
//! the run-scoped idempotency token from it). These are the M1-exit-gate clauses
//! "resolved versions appear as metadata" + "cross-boundary exactly-once under the
//! new token" (the lease-side half; the token derivation itself is unit-tested in
//! kx-capability).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, RunNonceSource, WorkerRegistry,
};
use kx_journal::{InMemoryJournal, Journal, ResolvedKindTag, SqliteJournal};
use kx_mote::{ModelId, NdClass, ToolName, ToolVersion};
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, ToolRequirement, WarrantSpec,
};
use tempfile::tempdir;
use tonic::{Code, Request};

// --- seams (mirror run_registration.rs) ---------------------------------------

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

fn registry() -> Arc<dyn WorkerRegistry> {
    Arc::new(InMemoryWorkerRegistry::new())
}

/// Coordinator over the DEFAULT built-in tool registry (the production path),
/// with a fixed nonce so the registered `instance_id` is deterministic.
fn coordinator<J: Journal + Send + 'static>(
    journal: J,
    instance_id: [u8; 16],
) -> CoordinatorService {
    CoordinatorService::with_seams(
        journal,
        registry(),
        None,
        Arc::new(FixedClock(7)),
        Arc::new(FixedNonce(instance_id)),
    )
}

/// Coordinator with an INJECTED tool registry (for the capability-exceeds test).
fn coordinator_with_tools<J: Journal + Send + 'static>(
    journal: J,
    instance_id: [u8; 16],
    tool_registry: Arc<dyn ToolRegistry>,
) -> CoordinatorService {
    CoordinatorService::with_tool_registry_and_seams(
        journal,
        registry(),
        None,
        Arc::new(FixedClock(7)),
        Arc::new(FixedNonce(instance_id)),
        tool_registry,
    )
}

async fn register_run(svc: &CoordinatorService, fingerprint: [u8; 32]) -> [u8; 16] {
    svc.register_run(Request::new(proto::RegisterRunRequest {
        recipe_fingerprint: fingerprint.to_vec(),
    }))
    .await
    .unwrap()
    .into_inner()
    .instance_id
    .as_slice()
    .try_into()
    .unwrap()
}

/// A warrant granting the named `(tool_id, tool_version)` builtins + a model id.
/// Permissive scopes so any empty-requirement builtin resolves.
fn warrant_granting(tools: &[(&str, &str)], model_id: &str) -> WarrantSpec {
    let tool_grants: BTreeSet<ToolGrant> = tools
        .iter()
        .map(|(id, ver)| ToolGrant {
            tool_id: ToolName((*id).into()),
            tool_version: ToolVersion((*ver).into()),
        })
        .collect();
    let mut mounts = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/in"), kx_warrant::FsMode::ReadOnly);
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())])),
        // Matches the OSS built-ins' syscall_profile_ref (exact-match required by
        // `check_tool_requirement`), so fs-read/fs-write resolve.
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

// === EXIT GATE: resolved versions appear as metadata ==========================

#[tokio::test]
async fn submit_captures_resolved_versions_as_metadata() {
    let instance_id = [0xa1u8; 16];
    let svc = coordinator(InMemoryJournal::new(), instance_id);
    register_run(&svc, [0xb2u8; 32]).await;

    let mote = common::mote(1, NdClass::Pure, &[]);
    // Two builtins (resolve under any warrant) + a known model id.
    let warrant = warrant_granting(&[("fs-read", "1"), ("fs-write", "1")], "qwen2.5-0.5b");
    let resp = common::submit(&svc, &mote, &warrant).await;
    assert_eq!(resp.status, proto::SubmitStatus::Accepted as i32);

    let records = svc.run_resolved_versions().await.unwrap();
    // One record per resolved capability (BTreeSet order: fs-read, fs-write).
    assert_eq!(
        records.len(),
        2,
        "one metadata record per resolved capability"
    );
    let warrant_ref = warrant_ref_of(&warrant);
    for rec in &records {
        assert_eq!(
            rec.instance_id, instance_id,
            "anchored to the registered run"
        );
        assert_eq!(
            rec.warrant_ref, warrant_ref,
            "captures the resolved warrant_ref"
        );
        assert_eq!(
            rec.model_id, "qwen2.5-0.5b",
            "captures the resolved model_id"
        );
    }
    let caps: Vec<&str> = records
        .iter()
        .map(|r| r.capability.as_ref().unwrap().tool_id.as_str())
        .collect();
    assert_eq!(caps, ["fs-read", "fs-write"], "ordered, both captured");
    for rec in &records {
        let cap = rec.capability.as_ref().unwrap();
        assert_eq!(cap.tool_version, "1");
        assert_eq!(cap.resolved_kind, ResolvedKindTag::Builtin);
        // resolved_def_hash is a real content ref (not the zero sentinel).
        assert_ne!(cap.resolved_def_hash.as_bytes(), &[0u8; 32]);
    }
}

#[tokio::test]
async fn submit_response_surfaces_instance_id() {
    // The exit-gate resume key + server-derived identity (D53): the SubmitMote
    // response carries exactly the run's registered instance_id.
    let instance_id = [0xc3u8; 16];
    let svc = coordinator(InMemoryJournal::new(), instance_id);
    let registered = register_run(&svc, [0x44u8; 32]).await;
    assert_eq!(registered, instance_id);

    let mote = common::mote(2, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("fs-read", "1")], "m");
    let resp = common::submit(&svc, &mote, &warrant).await;
    assert_eq!(
        resp.instance_id, instance_id,
        "SubmitMote surfaces the registered run id (the M2 resume key)"
    );
}

#[tokio::test]
async fn duplicate_submit_response_also_surfaces_instance_id() {
    // Regression — idempotent re-invoke. A DUPLICATE submit (the same Mote
    // re-submitted before commit — e.g. an Invoke of the same recipe+args, which
    // re-derives the same terminal Mote) MUST still surface the registered run's
    // instance_id. Before the fix the duplicate path returned `instance_id = None`,
    // so the gateway decoded an empty 16-byte id and failed the whole call with
    // `UNAVAILABLE("non-16-byte instance_id")` — breaking exactly-once-per-input.
    let instance_id = [0xe7u8; 16];
    let svc = coordinator(InMemoryJournal::new(), instance_id);

    let mote = common::mote(2, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("fs-read", "1")], "m");

    let first = common::submit(&svc, &mote, &warrant).await;
    assert_eq!(first.status, proto::SubmitStatus::Accepted as i32);
    assert_eq!(first.instance_id, instance_id);

    // Re-submit the SAME Mote → Duplicate, but the instance_id must be present.
    let second = common::submit(&svc, &mote, &warrant).await;
    assert_eq!(
        second.status,
        proto::SubmitStatus::Duplicate as i32,
        "re-submitting the same Mote before commit is a duplicate"
    );
    assert_eq!(
        second.instance_id, instance_id,
        "a duplicate submit surfaces the SAME registered run id (not an empty one)"
    );
    assert_eq!(second.mote_id, first.mote_id, "same canonical Mote id");
    // Capture stays fresh-only: the duplicate appended no second metadata fact.
    assert_eq!(
        svc.run_resolved_versions().await.unwrap().len(),
        1,
        "duplicate submit must NOT re-capture run-version metadata"
    );
}

#[tokio::test]
async fn unregistered_submit_is_refused() {
    // M1.3 (was M1.1/M1.2 `unregistered_run_captures_nothing`): submit-before-register
    // is now REFUSED (failed_precondition). An unregistered run has no journaled
    // identity to anchor capture or the run-scoped idempotency token (D64/D98 —
    // identity is the explicit RegisterRun, never lazy-on-submit), so NOTHING is
    // written: no Mote, no metadata, no registration.
    let svc = coordinator(InMemoryJournal::new(), [0xd4u8; 16]);
    let mote = common::mote(3, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("fs-read", "1")], "m");
    let err = common::submit_unregistered(&svc, &mote, &warrant)
        .await
        .expect_err("submit before RegisterRun must be refused (M1.3)");
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(
        svc.committed_count().await.unwrap(),
        0,
        "a refused submit writes nothing"
    );
    assert!(
        svc.run_resolved_versions().await.unwrap().is_empty(),
        "no metadata captured for a refused submit"
    );
    assert_eq!(
        svc.run_registration().await.unwrap(),
        None,
        "the run is still unregistered"
    );
}

// === EXIT GATE: instance_id flows to the worker (token root) ==================

#[tokio::test]
async fn lease_surfaces_instance_id_for_the_run_scoped_token() {
    let instance_id = [0xe5u8; 16];
    let svc = coordinator(InMemoryJournal::new(), instance_id);
    register_run(&svc, [0x66u8; 32]).await;
    let worker = common::register(&svc, "http://w1").await;

    // A PURE root Mote is ready immediately (no parents) → leasable.
    let mote = common::mote(4, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("fs-read", "1")], "m");
    common::submit(&svc, &mote, &warrant).await;

    let resp = svc
        .lease_work(Request::new(proto::LeaseWorkRequest {
            worker_id: worker,
            executor_class: proto::ExecutorClass::MacosSandbox as i32,
            max_motes: 16,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.items.len(), 1, "the PURE Mote is leased");
    assert_eq!(
        resp.instance_id, instance_id,
        "LeaseWork carries the run id → the worker derives the run-scoped token"
    );
}

// === SECURITY: metadata is independent of identity ============================

#[tokio::test]
async fn resolved_model_version_is_independent_of_mote_identity() {
    // The SAME Mote submitted under two runs with DIFFERENT resolved model
    // versions yields the SAME MoteId (resolved versions are metadata, never an
    // identity input — D64/D79/D70) while the captured model_id differs per run.
    let mote = common::mote(5, NdClass::Pure, &[]);

    let svc_a = coordinator(InMemoryJournal::new(), [0x0au8; 16]);
    register_run(&svc_a, [0x01u8; 32]).await;
    let resp_a = common::submit(
        &svc_a,
        &mote,
        &warrant_granting(&[("fs-read", "1")], "model-A"),
    )
    .await;

    let svc_b = coordinator(InMemoryJournal::new(), [0x0bu8; 16]);
    register_run(&svc_b, [0x01u8; 32]).await;
    let resp_b = common::submit(
        &svc_b,
        &mote,
        &warrant_granting(&[("fs-read", "1")], "model-B"),
    )
    .await;

    assert_eq!(
        resp_a.mote_id, resp_b.mote_id,
        "identity is stable across resolved-version changes"
    );

    let model_a = svc_a.run_resolved_versions().await.unwrap()[0]
        .model_id
        .clone();
    let model_b = svc_b.run_resolved_versions().await.unwrap()[0]
        .model_id
        .clone();
    assert_eq!(model_a, "model-A");
    assert_eq!(model_b, "model-B");
    assert_ne!(
        model_a, model_b,
        "metadata varies independently of identity"
    );
}

// === SECURITY: a capability-exceeds-warrant grant journals NO metadata ========

#[tokio::test]
async fn capability_exceeds_warrant_skips_capture() {
    // A tool whose required net egress is NOT in the warrant fails to resolve →
    // resolution is Unresolved → NOTHING is journaled (no over-privileged or partial
    // tuple is ever recorded). The mote here is PURE, so M1.3's D66 fail-closed
    // refusal does NOT fire (D66 is WORLD-MUTATING-only — a non-mutating Mote has no
    // double-fire hazard); the submit still succeeds and capture is skipped.
    let mut tools = InMemoryToolRegistry::new();
    tools
        .register(
            ToolDef {
                tool_id: ToolName("greedy".into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: ToolRequirement {
                    // Requires egress to a host the warrant below does NOT permit.
                    net_scope_required: NetScope::EgressAllowlist(BTreeSet::from([Host(
                        "evil.example.com:443".into(),
                    )])),
                    fs_scope_required: FsScope::empty(),
                    syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: ResourceCeiling {
                        cpu_milli: 0,
                        mem_bytes: 0,
                        wall_clock_ms: 0,
                        fd_count: 0,
                        disk_bytes: 0,
                    },
                },
                description: "needs egress the warrant lacks".into(),
                idempotency_class: IdempotencyClass::Token,
                input_schema: None,
            },
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();

    let instance_id = [0xf6u8; 16];
    let svc = coordinator_with_tools(InMemoryJournal::new(), instance_id, Arc::new(tools));
    register_run(&svc, [0x77u8; 32]).await;

    let mote = common::mote(6, NdClass::Pure, &[]);
    // net_scope = None → the greedy tool's egress requirement is NOT a subset.
    let mut warrant = warrant_granting(&[("greedy", "1")], "m");
    warrant.net_scope = NetScope::None;
    let resp = common::submit(&svc, &mote, &warrant).await;
    assert_eq!(
        resp.status,
        proto::SubmitStatus::Accepted as i32,
        "submit still succeeds in M1.2"
    );
    assert!(
        svc.run_resolved_versions().await.unwrap().is_empty(),
        "a capability-exceeds-warrant grant journals NO metadata (fail-closed capture)"
    );
}

// === DURABILITY: replay reconstructs the metadata verbatim ====================

#[tokio::test]
async fn replay_reconstructs_run_versions_metadata() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let instance_id = [0xabu8; 16];
    let warrant = warrant_granting(&[("fs-read", "1"), ("fs-write", "1")], "qwen");
    let mote = common::mote(7, NdClass::Pure, &[]);

    let before = {
        let svc = coordinator(SqliteJournal::open(&path).unwrap(), instance_id);
        register_run(&svc, [0xcdu8; 32]).await;
        common::submit(&svc, &mote, &warrant).await;
        svc.run_resolved_versions().await.unwrap()
    }; // svc dropped → core thread exits → Sqlite handle closed

    assert_eq!(before.len(), 2);

    // Fresh coordinator over the SAME journal → folds from scratch.
    let svc2 = coordinator(SqliteJournal::open(&path).unwrap(), [0x00u8; 16]);
    let after = svc2.run_resolved_versions().await.unwrap();
    assert_eq!(
        after, before,
        "the resolved-version metadata is reconstructed byte-identically on replay"
    );
    // The registered identity also survives (read, never recomputed).
    assert_eq!(
        svc2.run_registration().await.unwrap().unwrap().0,
        instance_id
    );
}
