//! M1.3 — submission-refusal predicates wired at `SubmitMote`:
//!
//! - **Registration gate** (D64/D98): submit-before-register → `failed_precondition`,
//!   nothing written.
//! - **D66 fail-closed** (StageThenCommit is the #1 no-double-fire seam): a
//!   WORLD-MUTATING Mote whose tools do not resolve (unregistered tool, or a tool
//!   whose required capability exceeds the warrant) → `SUBMIT_STATUS_REJECTED`,
//!   nothing journaled.
//! - **R-10** (D38 §2c): a WM Mote resolving to an `AtLeastOnce` tool is refused
//!   unless the submission sets `accept_at_least_once`.
//! - **PURE/ROND are never refused on resolution grounds** (no double-fire hazard):
//!   an unresolvable grant skips capture (M1.2) but the submit succeeds.
//!
//! All assertions go through the real gRPC service (`Coordinator` trait) with a
//! fixed nonce/clock so the registered identity is deterministic.

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
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{
    ExecutorClass, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolGrant,
    ToolRequirement, WarrantSpec,
};
use tonic::{Code, Request};

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

/// Coordinator over the default OSS built-in tool registry.
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

/// Coordinator with an injected tool registry (for the AtLeastOnce / exceeds tests).
fn coordinator_with_tools<J: Journal + Send + 'static>(
    journal: J,
    instance_id: [u8; 16],
    tools: Arc<dyn ToolRegistry>,
) -> CoordinatorService {
    CoordinatorService::with_tool_registry_and_seams(
        journal,
        registry(),
        None,
        Arc::new(FixedClock(7)),
        Arc::new(FixedNonce(instance_id)),
        tools,
    )
}

/// A warrant granting the named `(tool_id, tool_version)` pairs + a model id, with
/// the syscall profile the OSS built-ins require (exact-match).
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

/// A registry with one `AtLeastOnce` builtin (`emailer@1`) — the R-10 trigger.
fn registry_with_at_least_once() -> Arc<dyn ToolRegistry> {
    let mut reg = InMemoryToolRegistry::with_builtins();
    reg.register(
        ToolDef {
            tool_id: ToolName("emailer".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: ToolRequirement {
                net_scope_required: NetScope::None,
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
            description: "sends an email; no closing mechanism (at-least-once)".into(),
            idempotency_class: IdempotencyClass::AtLeastOnce,
            input_schema: None,
        },
        ToolProvenance::HumanAuthored {
            author: "ops".into(),
        },
    )
    .unwrap();
    Arc::new(reg)
}

async fn register(svc: &CoordinatorService, fp: [u8; 32]) {
    svc.register_run(Request::new(proto::RegisterRunRequest {
        recipe_fingerprint: fp.to_vec(),
    }))
    .await
    .unwrap();
}

// === Registration gate ========================================================

#[tokio::test]
async fn submit_before_register_is_refused_failed_precondition() {
    let svc = coordinator(InMemoryJournal::new(), [0x11u8; 16]);
    let mote = common::wm_mote(1, EffectPattern::StageThenCommit);
    let warrant = warrant_granting(&[("fs-write", "1")], "m");

    let err = common::submit_unregistered(&svc, &mote, &warrant)
        .await
        .expect_err("submit before RegisterRun must be refused");
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(svc.committed_count().await.unwrap(), 0, "nothing committed");
    assert_eq!(
        svc.run_registration().await.unwrap(),
        None,
        "still unregistered"
    );
}

// === D66 — fail-closed on a tool-resolution miss ==============================

#[tokio::test]
async fn d66_refuses_world_mutating_with_unregistered_tool() {
    let svc = coordinator(InMemoryJournal::new(), [0x22u8; 16]);
    register(&svc, [0xb2u8; 32]).await;

    let mote = common::wm_mote(2, EffectPattern::StageThenCommit);
    // The warrant grants a tool that is NOT in the registry → resolution miss.
    let warrant = warrant_granting(&[("ghost-tool", "9")], "m");
    let resp = common::submit(&svc, &mote, &warrant).await;

    assert_eq!(resp.status, proto::SubmitStatus::Rejected as i32);
    assert!(
        resp.detail.contains("D66"),
        "detail names the D66 refusal: {}",
        resp.detail
    );
    assert_eq!(
        resp.refusal_code, "D66",
        "PR-2: the STRUCTURED code rides the response alongside the prose"
    );
    assert!(
        resp.instance_id.is_empty(),
        "a refused submit surfaces no instance_id"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 0);
    assert!(
        svc.run_resolved_versions().await.unwrap().is_empty(),
        "a refused submit journals no metadata"
    );
}

#[tokio::test]
async fn d66_security_refuses_capability_exceeding_warrant() {
    // A tool whose required net egress is NOT in the warrant fails to resolve
    // (CapabilityExceedsWarrant) → for a WM Mote, D66 refuses (anti-privilege-
    // laundering + anti-double-fire at the boundary).
    let mut reg = InMemoryToolRegistry::with_builtins();
    reg.register(
        ToolDef {
            tool_id: ToolName("greedy".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: ToolRequirement {
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

    let svc = coordinator_with_tools(InMemoryJournal::new(), [0x33u8; 16], Arc::new(reg));
    register(&svc, [0xb3u8; 32]).await;

    let mote = common::wm_mote(3, EffectPattern::StageThenCommit);
    let mut warrant = warrant_granting(&[("greedy", "1")], "m");
    warrant.net_scope = NetScope::None; // greedy's egress requirement is NOT a subset
    let resp = common::submit(&svc, &mote, &warrant).await;

    assert_eq!(
        resp.status,
        proto::SubmitStatus::Rejected as i32,
        "a WM Mote that grants a privilege-exceeding tool is refused (D66 fail-closed)"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 0);
}

// === R-10 — AtLeastOnce requires explicit accept ==============================

#[tokio::test]
async fn r10_refuses_at_least_once_without_accept_then_accepts() {
    let svc = coordinator_with_tools(
        InMemoryJournal::new(),
        [0x44u8; 16],
        registry_with_at_least_once(),
    );
    register(&svc, [0xb4u8; 32]).await;

    let mote = common::wm_mote(4, EffectPattern::IdempotentByConstruction);
    let warrant = warrant_granting(&[("emailer", "1")], "m");

    // accept_at_least_once defaults false → R-10 refuses.
    let refused = common::submit(&svc, &mote, &warrant).await;
    assert_eq!(refused.status, proto::SubmitStatus::Rejected as i32);
    assert!(
        refused.detail.contains("R-10"),
        "detail names R-10: {}",
        refused.detail
    );
    assert_eq!(
        refused.refusal_code, "R-10",
        "PR-2: the STRUCTURED code rides the response alongside the prose"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 0);

    // The operator opts in → accepted.
    let accepted = common::submit_accepting(&svc, &mote, &warrant).await;
    assert!(
        accepted.refusal_code.is_empty(),
        "an accepted submit carries no refusal code"
    );
    assert_eq!(
        accepted.status,
        proto::SubmitStatus::Accepted as i32,
        "accept_at_least_once=true admits the AtLeastOnce WM Mote"
    );
}

// === PURE / ROND are never refused on resolution grounds ======================

#[tokio::test]
async fn pure_with_unresolvable_grant_is_accepted_capture_skipped() {
    let svc = coordinator(InMemoryJournal::new(), [0x55u8; 16]);
    register(&svc, [0xb5u8; 32]).await;

    let mote = common::mote(5, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("ghost-tool", "9")], "m"); // unresolvable
    let resp = common::submit(&svc, &mote, &warrant).await;

    assert_eq!(
        resp.status,
        proto::SubmitStatus::Accepted as i32,
        "a PURE Mote with an unresolvable grant is admitted (D66 is WM-only)"
    );
    assert!(
        svc.run_resolved_versions().await.unwrap().is_empty(),
        "capture is skipped on the resolution miss (M1.2 behavior preserved)"
    );
}

#[tokio::test]
async fn pure_with_resolvable_grant_is_accepted_and_captured() {
    let svc = coordinator(InMemoryJournal::new(), [0x66u8; 16]);
    register(&svc, [0xb6u8; 32]).await;

    let mote = common::mote(6, NdClass::Pure, &[]);
    let warrant = warrant_granting(&[("fs-read", "1")], "qwen");
    let resp = common::submit(&svc, &mote, &warrant).await;

    assert_eq!(resp.status, proto::SubmitStatus::Accepted as i32);
    let records = svc.run_resolved_versions().await.unwrap();
    assert_eq!(
        records.len(),
        1,
        "the resolvable grant is captured as metadata"
    );
    assert_eq!(records[0].capability.as_ref().unwrap().tool_id, "fs-read");
}
