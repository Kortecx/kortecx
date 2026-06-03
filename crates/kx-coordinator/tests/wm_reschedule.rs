//! P3.6c — R-13 under distribution: the coordinator gates a crash-failed *re-dispatch* of a
//! non-PURE Mote on the recovery oracle, so a WORLD-MUTATING effect that may have fired is
//! never re-leased without a durable `EffectStaged` hint.
//!
//! Single-node, `pick_next` / the executor's R-13 refuse to re-dispatch a WM Mote whose
//! `EffectStaged` was never recorded (the effect might already have fired — re-dispatch would
//! double it, which is unrecoverable). Distributed, reschedule (D57) re-leases dead workers'
//! in-flight Motes; without this gate it would re-offer a fired-but-unstaged VTC / IBC producer
//! (D58 lets those patterns dispatch WITHOUT staging) and a second worker would re-fire the
//! effect. The gate (`redispatch_admissible`) closes that window: PURE is always recomputable;
//! a non-PURE crash-failed Mote is re-offered ONLY with the `EffectStaged` hint
//! (`can_redispatch_world_effect`); otherwise it is left stuck (operator-recoverable).
//!
//! Time is driven by an injected [`kx_coordinator::Clock`] — death is deterministic, no sleeps.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto::ExecutorClass;
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, MoteState, RunNonceSource, WorkerRegistry,
};
use kx_journal::InMemoryJournal;
use kx_mote::{EffectPattern, Mote, NdClass, ToolName, ToolVersion};
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolGrant, ToolRequirement};

const TIMEOUT: Duration = Duration::from_secs(6);
const MAC: ExecutorClass = ExecutorClass::MacosSandbox;

/// The at-most-once tool the M2.3b distributed-quarantine test registers + grants.
const ALO_TOOL: &str = "at-most-once-effect";

#[derive(Debug)]
struct FixedNonce([u8; 16]);
impl RunNonceSource for FixedNonce {
    fn fresh_instance_id(&self) -> [u8; 16] {
        self.0
    }
}

/// A coordinator whose tool registry has [`ALO_TOOL`] registered as
/// `IdempotencyClass::AtLeastOnce` (empty requirements → resolves under any
/// warrant that grants it). Used to journal the durable class so
/// `redispatch_admissible` can read it back at reschedule.
fn coordinator_with_alo_tool(clock: Arc<FakeClock>) -> CoordinatorService {
    let worker_registry: Arc<dyn WorkerRegistry> = Arc::new(
        InMemoryWorkerRegistry::with_clock_and_timeout(clock.clone(), TIMEOUT),
    );
    let mut tools = InMemoryToolRegistry::new();
    tools
        .register(
            ToolDef {
                tool_id: ToolName(ALO_TOOL.into()),
                tool_version: ToolVersion("1".into()),
                kind: ToolKind::Builtin,
                required_capability: ToolRequirement {
                    net_scope_required: NetScope::None,
                    fs_scope_required: FsScope::empty(),
                    // Exact-match against the granting warrant's syscall ref ([0; 32]).
                    syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: ResourceCeiling {
                        cpu_milli: 0,
                        mem_bytes: 0,
                        wall_clock_ms: 0,
                        fd_count: 0,
                        disk_bytes: 0,
                    },
                },
                description: "a token-less, no-readback effect (at-most-once)".into(),
                idempotency_class: IdempotencyClass::AtLeastOnce,
                input_schema: None,
            },
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();
    CoordinatorService::with_tool_registry_and_seams(
        InMemoryJournal::new(),
        worker_registry,
        None,
        clock,
        Arc::new(FixedNonce([0x3b; 16])),
        Arc::new(tools),
    )
}

/// A deterministic clock the test advances by hand (mirrors `tests/reschedule.rs`).
#[derive(Debug)]
struct FakeClock(AtomicU64);
impl FakeClock {
    fn new(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }
    fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::Relaxed);
    }
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

fn coordinator(clock: Arc<FakeClock>) -> CoordinatorService {
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        InMemoryWorkerRegistry::with_clock_and_timeout(clock, TIMEOUT),
    );
    CoordinatorService::with_registry(InMemoryJournal::new(), registry)
}

/// R-13: a non-PURE Mote that crashed WITHOUT a durable `EffectStaged` hint (the
/// `ValidateThenCommit` / `IdempotentByConstruction` fire-then-crash window — those patterns
/// never stage) is NOT re-offered. Re-dispatch would risk doubling a real-world effect.
#[tokio::test]
async fn crash_failed_world_mutating_without_effect_staged_is_not_re_offered() {
    for pattern in [
        EffectPattern::ValidateThenCommit,
        EffectPattern::IdempotentByConstruction,
    ] {
        let clock = FakeClock::new(1_000);
        let svc = coordinator(clock.clone());
        let warrant = common::sample_warrant();

        let dying = common::register(&svc, "dying").await;
        let m = common::wm_mote(7, pattern);
        common::submit(&svc, &m, &warrant).await;

        // First dispatch is allowed (fresh ready Mote); the worker fires the effect then dies
        // WITHOUT staging — for VTC/IBC there is no EffectStaged step (D58 §4).
        let leased = common::lease_work(&svc, dying, MAC, 16).await;
        assert_eq!(leased.len(), 1, "{pattern:?}: first dispatch is offered");

        // Time advances past the timeout; a live worker polls → reap re-classifies the dead
        // lease as crash-failed, but the oracle gate refuses to re-offer it.
        clock.set(1_000 + 6_001);
        let live = common::register(&svc, "live").await;
        let offered = common::lease_work(&svc, live, MAC, 16).await;
        assert!(
            offered.is_empty(),
            "{pattern:?}: a crash-failed non-PURE Mote with no EffectStaged is NOT re-offered \
             (R-13 — re-dispatch could double the effect)"
        );
        assert_eq!(
            svc.state_of(m.id).await.unwrap(),
            MoteState::Failed,
            "{pattern:?}: it is left stuck (operator-recoverable via repudiation), not re-leased"
        );
    }
}

/// R-13: a `StageThenCommit` Mote that DID record `EffectStaged` before crashing IS re-offered —
/// the durable hint makes re-dispatch safe (the broker's tool-boundary idempotency dedupes the
/// re-fire). This is the safe path P3.6b's W-3 exercises end-to-end.
#[tokio::test]
async fn crash_failed_world_mutating_with_effect_staged_is_re_offered() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let dying = common::register(&svc, "dying").await;
    let m = common::wm_mote(9, EffectPattern::StageThenCommit);
    common::submit(&svc, &m, &warrant).await;

    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1, "first dispatch is offered");

    // The worker stages its intent (EffectStaged recorded) then crashes before committing.
    common::report_effect_staged(&svc, &m, dying).await;

    clock.set(1_000 + 6_001);
    let live = common::register(&svc, "live").await;
    let offered = common::lease_work(&svc, live, MAC, 16).await;
    assert_eq!(
        offered.len(),
        1,
        "with the EffectStaged hint, re-dispatch is safe → the Mote is re-offered"
    );
    let offered_mote: Mote = offered[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(offered_mote.id, m.id);
}

/// **M2.3b (D65 / D105.4) — the distributed class-aware quarantine.** An
/// `AtLeastOnce` tool has NO closing mechanism, so a re-dispatch would double-fire
/// — EVEN with a durable `EffectStaged` hint. Unlike the `StageThenCommit` Mote in
/// `crash_failed_world_mutating_with_effect_staged_is_re_offered` (which IS
/// re-offered), the coordinator reads the durable resolved class (journaled in
/// `RunVersionsResolved`) and refuses to re-offer it — the distributed quarantine
/// (left stuck, operator-recoverable). This closes the latent double-fire the
/// pre-M2.3b class-blind gate left open.
#[tokio::test]
async fn crash_failed_at_least_once_is_not_re_offered_even_with_effect_staged() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator_with_alo_tool(clock.clone());

    // A warrant granting ONLY the at-most-once tool (the custom registry has no
    // builtins). The tool's empty requirements resolve under these scopes.
    let mut warrant = common::sample_warrant();
    let mut grants = std::collections::BTreeSet::new();
    grants.insert(ToolGrant {
        tool_id: ToolName(ALO_TOOL.into()),
        tool_version: ToolVersion("1".into()),
    });
    warrant.tool_grants = grants;

    let dying = common::register(&svc, "dying").await;
    // A StageThenCommit WM Mote whose contract names the at-most-once tool.
    let m = common::wm_mote_with_tool(7, ALO_TOOL, "1");
    // accept_at_least_once = true — else R-10 refuses the AtLeastOnce grant at submit.
    common::submit_accepting(&svc, &m, &warrant).await;

    // The resolved class was journaled durably (the recovery decision reads this).
    let recs = svc.run_resolved_versions().await.unwrap();
    let cap = recs
        .iter()
        .find_map(|r| r.capability.as_ref())
        .expect("a resolved capability record");
    assert_eq!(cap.tool_id, ALO_TOOL);
    assert_eq!(
        cap.idempotency_class,
        kx_journal::IdempotencyClassTag::AtLeastOnce,
        "the durable class is captured on the RunVersionsResolved fact"
    );

    // First dispatch is offered; the worker stages its intent then dies.
    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1, "first dispatch is offered");
    common::report_effect_staged(&svc, &m, dying).await;

    // Time advances past the timeout; a live worker polls → reap crash-fails the
    // dead lease, but the class-aware gate refuses to re-offer the at-most-once Mote.
    clock.set(1_000 + 6_001);
    let live = common::register(&svc, "live").await;
    let offered = common::lease_work(&svc, live, MAC, 16).await;
    assert!(
        offered.is_empty(),
        "an at-most-once Mote is NOT re-offered even WITH EffectStaged (M2.3b quarantine — \
         a re-dispatch would double-fire)"
    );
    // Never progressed to Committed (left stuck, operator-recoverable).
    assert_ne!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);
}

/// Regression guard: a crash-failed PURE Mote is always re-offered (recomputable — no
/// world-effect hazard). The gate must not over-reach into the PURE reschedule path (D57).
#[tokio::test]
async fn crash_failed_pure_mote_is_still_re_offered() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let dying = common::register(&svc, "dying").await;
    let m = common::mote(3, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;

    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1);

    clock.set(1_000 + 6_001);
    let live = common::register(&svc, "live").await;
    let offered = common::lease_work(&svc, live, MAC, 16).await;
    assert_eq!(
        offered.len(),
        1,
        "PURE is recomputable — a crash-failed PURE Mote is still re-leased (D57 unchanged)"
    );
}
