//! The three exit-gate scenarios and the [`run_seed`] entry point.
//!
//! Each scenario builds a tiny workflow, drives it through the [`Cluster`] with the
//! plan's fault injected, then asserts the guarantee. A violated invariant (or an
//! infrastructure fault) becomes a [`ChaosFailure`] carrying the reproducing seed.

use std::collections::BTreeSet;

use kx_coordinator::MoteState;
use kx_mote::{Mote, MoteId, TopologyDecision};
use kx_runtime::topology::DEMO_WORKER_COUNT;
use kx_warrant::WarrantSpec;

use crate::assertions::{ChaosFailure, ChaosOutcome};
use crate::cluster::Cluster;
use crate::plan::{ChaosPlan, FaultPoint, ScenarioKind, WmPattern};
use crate::workflow;

/// A failure reason paired with the outcome observed (when one was reached).
type Failure = (String, Option<ChaosOutcome>);
/// A scenario either proves its guarantee (returning the outcome) or fails.
type ScenarioResult = Result<ChaosOutcome, Failure>;

/// Wrap an infrastructure (RPC/coordinator) fault — no outcome was reached.
fn infra(mut e: String) -> Failure {
    e.insert_str(0, "infra: ");
    (e, None)
}

/// A gate-invariant violation carrying the observed outcome.
fn bad(reason: impl Into<String>, outcome: ChaosOutcome) -> Failure {
    (reason.into(), Some(outcome))
}

/// Run the chaos scenario the seed selects and check its exit-gate invariant.
///
/// Pure and deterministic: equal seeds replay the identical run. On success the
/// observed [`ChaosOutcome`] is returned; on any violation an [`ChaosFailure`] carries
/// the seed + plan + reason so the failing seed reproduces exactly via `run_seed(seed)`.
///
/// # Errors
/// Returns [`ChaosFailure`] when an exit-gate invariant is violated (double world
/// effect, orphaned/duplicated children, an incorrect cascade) or the coordinator
/// returns an unexpected error.
pub async fn run_seed(seed: u64) -> Result<ChaosOutcome, ChaosFailure> {
    let plan = ChaosPlan::from_seed(seed);
    let result = match plan.scenario {
        ScenarioKind::ExactlyOnce => exactly_once(&plan).await,
        ScenarioKind::TopologyShaper => topology_shaper(&plan).await,
        ScenarioKind::RepudiationCascade => repudiation_cascade(&plan).await,
    };
    result.map_err(|(reason, outcome)| ChaosFailure {
        seed,
        plan,
        reason,
        outcome,
    })
}

// ---------------------------------------------------------------------------
// Scenario A — exactly-once world effect under worker death
// ---------------------------------------------------------------------------

/// Whether the post-death re-lease re-offered the WM Mote (`None` off the death path)
/// and whether it was leasable on first dispatch.
struct WmDrive {
    re_offered: Option<bool>,
    leased_first: bool,
}

/// Drive the single WM Mote to its terminal state under the plan's fault.
async fn drive_wm(
    c: &mut Cluster,
    plan: &ChaosPlan,
    wm: &Mote,
    warrant: &WarrantSpec,
    staged: bool,
) -> Result<WmDrive, Failure> {
    let mut drive = WmDrive {
        re_offered: None,
        leased_first: true,
    };
    match plan.fault {
        FaultPoint::Clean => {
            let w = c.register().await.map_err(infra)?;
            c.lease(w).await.map_err(infra)?;
            if staged {
                c.stage(w, wm).await.map_err(infra)?;
            }
            let r = c.fire(wm, warrant).map_err(infra)?;
            c.commit(w, wm, warrant, r).await.map_err(infra)?;
        }
        FaultPoint::RacingDuplicate => {
            let a = c.register().await.map_err(infra)?;
            let b = c.register().await.map_err(infra)?;
            c.lease(a).await.map_err(infra)?;
            c.lease(b).await.map_err(infra)?;
            // Both live workers execute the same ready Mote (no lease lock); fire + commit
            // dedup to one net effect / one committed fact.
            for w in [a, b] {
                if staged {
                    c.stage(w, wm).await.map_err(infra)?;
                }
                let r = c.fire(wm, warrant).map_err(infra)?;
                c.commit(w, wm, warrant, r).await.map_err(infra)?;
            }
        }
        FaultPoint::DeathBeforeCommit => {
            let a = c.register().await.map_err(infra)?;
            let leased = c.lease(a).await.map_err(infra)?;
            drive.leased_first = leased.iter().any(|m| m.id == wm.id);
            if staged {
                c.stage(a, wm).await.map_err(infra)?;
            }
            // `a` FIRES the effect (net effect #1), then dies before committing — the exact
            // stage→fire→{die} window the EffectStaged hint exists to make recoverable.
            c.fire(wm, warrant).map_err(infra)?;
            c.advance_past_timeout();
            let b = c.register().await.map_err(infra)?;
            let leased_b = c.lease(b).await.map_err(infra)?; // reaps `a`
            let offered = leased_b.iter().any(|m| m.id == wm.id);
            drive.re_offered = Some(offered);
            if offered {
                // Staged ⇒ oracle admits re-dispatch: re-stage (dedup) + re-fire (idempotent,
                // net stays 1) + commit.
                if staged {
                    c.stage(b, wm).await.map_err(infra)?;
                }
                let r = c.fire(wm, warrant).map_err(infra)?;
                c.commit(b, wm, warrant, r).await.map_err(infra)?;
            }
            // else: the P3.6c oracle refused re-dispatch (no staged hint) ⇒ safely stuck.
        }
    }
    Ok(drive)
}

async fn exactly_once(plan: &ChaosPlan) -> ScenarioResult {
    let mut c = Cluster::new();
    let pattern = workflow::effect_pattern_of(plan.wm_pattern);
    let staged = matches!(plan.wm_pattern, WmPattern::StageThenCommit);
    let wm = workflow::wm_mote(plan.salt, 1, pattern);
    let warrant = workflow::wm_warrant();
    c.submit(&wm, &warrant).await.map_err(infra)?;

    let WmDrive {
        re_offered,
        leased_first,
    } = drive_wm(&mut c, plan, &wm, &warrant, staged).await?;

    let committed = c.committed_count().await.map_err(infra)?;
    let state = c.state_of(wm.id).await.map_err(infra)?;
    let net = c.broker().net_effects();
    let dispatches = c.broker().dispatch_calls();
    let is_committed = matches!(state, MoteState::Committed);
    let safely_stuck = matches!(plan.fault, FaultPoint::DeathBeforeCommit)
        && re_offered == Some(false)
        && !is_committed;
    let outcome = ChaosOutcome {
        committed_count: committed,
        net_effects: net,
        dispatch_calls: dispatches,
        safely_stuck,
        cascade_size: None,
        materialized_children: 0,
    };

    assert_exactly_once(plan, staged, leased_first, is_committed, outcome)
}

/// Check the exactly-once invariants over an observed `outcome` (kept separate so the
/// driver stays readable and within the line budget).
fn assert_exactly_once(
    plan: &ChaosPlan,
    staged: bool,
    leased_first: bool,
    is_committed: bool,
    outcome: ChaosOutcome,
) -> ScenarioResult {
    if matches!(plan.fault, FaultPoint::DeathBeforeCommit) && !leased_first {
        return Err(bad(
            "exactly-once: the WM Mote was not leasable on first dispatch",
            outcome,
        ));
    }
    // INVARIANT 1 (load-bearing): never more than one NET world effect for one WM Mote.
    if outcome.net_effects != 1 {
        return Err(bad(
            format!(
                "exactly-once: expected exactly 1 net world effect, got {}",
                outcome.net_effects
            ),
            outcome,
        ));
    }
    // INVARIANT 2: committed iff recovery was admissible; otherwise the only legitimate
    // terminal is the P3.6c safe-stuck (death + unstaged + oracle refusal).
    match (is_committed, plan.fault, staged) {
        (true, _, _) => {
            if outcome.committed_count != 1 {
                return Err(bad(
                    format!(
                        "exactly-once: committed WM Mote but committed_count={}",
                        outcome.committed_count
                    ),
                    outcome,
                ));
            }
        }
        (false, FaultPoint::DeathBeforeCommit, false) => {
            if !outcome.safely_stuck {
                return Err(bad(
                    "exactly-once: unstaged WM Mote uncommitted but not classified safe-stuck",
                    outcome,
                ));
            }
            if outcome.committed_count != 0 {
                return Err(bad(
                    format!(
                        "exactly-once: safe-stuck but committed_count={}",
                        outcome.committed_count
                    ),
                    outcome,
                ));
            }
        }
        _ => {
            return Err(bad(
                format!(
                    "exactly-once: WM Mote uncommitted in a path that must commit ({:?}, staged={staged})",
                    plan.fault
                ),
                outcome,
            ));
        }
    }
    // INVARIANT 3: a staged death that recovered proves it via a re-dispatch (≥2 dispatches),
    // so dedup — not luck — is what bounded the net effect to one.
    if matches!(plan.fault, FaultPoint::DeathBeforeCommit)
        && staged
        && is_committed
        && outcome.dispatch_calls < 2
    {
        return Err(bad(
            format!(
                "exactly-once: staged re-dispatch expected ≥2 dispatches, got {}",
                outcome.dispatch_calls
            ),
            outcome,
        ));
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Scenario B — no orphaned / duplicated children after a shaper death
// ---------------------------------------------------------------------------

/// Drive the shaper to its terminal state under the plan's fault; return whether it ended
/// in the P3.6c safe-stuck terminal (death + no staged hint + oracle refusal).
async fn drive_shaper(
    c: &mut Cluster,
    plan: &ChaosPlan,
    shaper: &Mote,
    sw: &WarrantSpec,
    td: &TopologyDecision,
) -> Result<bool, Failure> {
    match plan.fault {
        FaultPoint::Clean => {
            let w = c.register().await.map_err(infra)?;
            c.lease(w).await.map_err(infra)?;
            c.commit(w, shaper, sw, workflow::shaper_result_ref(td))
                .await
                .map_err(infra)?;
            Ok(false)
        }
        FaultPoint::RacingDuplicate => {
            let a = c.register().await.map_err(infra)?;
            let b = c.register().await.map_err(infra)?;
            c.lease(a).await.map_err(infra)?;
            c.lease(b).await.map_err(infra)?;
            // Both commit the SAME decision bytes ⇒ identical result_ref ⇒ dedup to one fact.
            c.commit(a, shaper, sw, workflow::shaper_result_ref(td))
                .await
                .map_err(infra)?;
            c.commit(b, shaper, sw, workflow::shaper_result_ref(td))
                .await
                .map_err(infra)?;
            Ok(false)
        }
        FaultPoint::DeathBeforeCommit => {
            let a = c.register().await.map_err(infra)?;
            c.lease(a).await.map_err(infra)?; // a holds the shaper, never commits
            c.advance_past_timeout();
            let b = c.register().await.map_err(infra)?;
            let leased_b = c.lease(b).await.map_err(infra)?; // reaps a
            let offered = leased_b.iter().any(|m| m.id == shaper.id);
            let st = c.state_of(shaper.id).await.map_err(infra)?;
            // A READ-ONLY-NONDET shaper records no EffectStaged hint, so the oracle refuses to
            // re-dispatch a crash-failed one (P3.6c) — it is safely stuck, decisionless.
            Ok(!offered && !matches!(st, MoteState::Committed))
        }
    }
}

/// Derive the shaper's children (production `derive_child_motes`), assert the set is
/// deterministic + distinct, then submit + commit them. Returns the child count.
async fn materialize_children(
    c: &mut Cluster,
    shaper: &Mote,
    td: &TopologyDecision,
    safely_stuck: bool,
) -> Result<usize, Failure> {
    let children = workflow::derive_children(shaper, td);
    let again = workflow::derive_children(shaper, td);
    let ids: Vec<MoteId> = children.iter().map(|m| m.id).collect();
    let ids_again: Vec<MoteId> = again.iter().map(|m| m.id).collect();
    if ids != ids_again {
        let o = topo_outcome(c, 0, safely_stuck).await?;
        return Err(bad("topology: child derivation is non-deterministic", o));
    }
    let distinct: BTreeSet<MoteId> = ids.iter().copied().collect();
    if distinct.len() != ids.len() {
        let o = topo_outcome(c, 0, safely_stuck).await?;
        return Err(bad("topology: duplicate child ids derived", o));
    }
    let cw = workflow::pure_warrant();
    for child in &children {
        c.submit(child, &cw).await.map_err(infra)?;
    }
    let w = c.register().await.map_err(infra)?;
    c.lease(w).await.map_err(infra)?;
    for child in &children {
        c.commit(w, child, &cw, workflow::pure_result_ref(child))
            .await
            .map_err(infra)?;
    }
    Ok(children.len())
}

async fn topology_shaper(plan: &ChaosPlan) -> ScenarioResult {
    let mut c = Cluster::new();
    let shaper = workflow::shaper_mote(plan.salt);
    let sw = workflow::pure_warrant();
    let td = workflow::topology_decision();
    c.submit(&shaper, &sw).await.map_err(infra)?;

    let safely_stuck = drive_shaper(&mut c, plan, &shaper, &sw, &td).await?;
    let shaper_committed = matches!(
        c.state_of(shaper.id).await.map_err(infra)?,
        MoteState::Committed
    );
    let materialized = if shaper_committed {
        materialize_children(&mut c, &shaper, &td, safely_stuck).await?
    } else {
        0
    };

    let o = topo_outcome(&c, materialized, safely_stuck).await?;
    if o.net_effects != 0 {
        return Err(bad(
            "topology: a non-world-mutating workflow produced a world effect",
            o,
        ));
    }
    if shaper_committed {
        if materialized != DEMO_WORKER_COUNT {
            return Err(bad(
                format!("topology: expected {DEMO_WORKER_COUNT} children, got {materialized}"),
                o,
            ));
        }
        let expected = 1 + DEMO_WORKER_COUNT;
        if o.committed_count != expected {
            return Err(bad(
                format!(
                    "topology: expected committed_count {expected} (shaper + children), got {}",
                    o.committed_count
                ),
                o,
            ));
        }
    } else {
        if !safely_stuck {
            return Err(bad(
                "topology: shaper uncommitted but not classified safe-stuck",
                o,
            ));
        }
        if materialized != 0 {
            return Err(bad(
                "topology: children materialized without a committed shaper (orphans!)",
                o,
            ));
        }
        if o.committed_count != 0 {
            return Err(bad(
                format!(
                    "topology: nothing should be committed, got {}",
                    o.committed_count
                ),
                o,
            ));
        }
    }
    Ok(o)
}

async fn topo_outcome(
    c: &Cluster,
    materialized: usize,
    safely_stuck: bool,
) -> Result<ChaosOutcome, Failure> {
    Ok(ChaosOutcome {
        committed_count: c.committed_count().await.map_err(infra)?,
        net_effects: c.broker().net_effects(),
        dispatch_calls: c.broker().dispatch_calls(),
        safely_stuck,
        cascade_size: None,
        materialized_children: materialized,
    })
}

// ---------------------------------------------------------------------------
// Scenario C — repudiation cascade correctness under chaos
// ---------------------------------------------------------------------------

/// Commit the `root → mid → leaf` lineage under the plan's fault.
async fn drive_lineage(
    c: &mut Cluster,
    plan: &ChaosPlan,
    root: &Mote,
    mid: &Mote,
    leaf: &Mote,
    wt: &WarrantSpec,
) -> Result<(), Failure> {
    match plan.fault {
        FaultPoint::Clean => {
            let w = c.register().await.map_err(infra)?;
            for m in [root, mid, leaf] {
                c.commit(w, m, wt, workflow::pure_result_ref(m))
                    .await
                    .map_err(infra)?;
            }
        }
        FaultPoint::RacingDuplicate => {
            let a = c.register().await.map_err(infra)?;
            let b = c.register().await.map_err(infra)?;
            for m in [root, mid, leaf] {
                c.commit(a, m, wt, workflow::pure_result_ref(m))
                    .await
                    .map_err(infra)?;
                c.commit(b, m, wt, workflow::pure_result_ref(m))
                    .await
                    .map_err(infra)?; // dedup
            }
        }
        FaultPoint::DeathBeforeCommit => {
            // `a` commits the root, then leases `mid` and dies before committing it; a
            // replacement reaps and finishes the lineage (PURE reschedule, D57).
            let a = c.register().await.map_err(infra)?;
            c.commit(a, root, wt, workflow::pure_result_ref(root))
                .await
                .map_err(infra)?;
            c.lease(a).await.map_err(infra)?; // a holds `mid`
            c.advance_past_timeout();
            let b = c.register().await.map_err(infra)?;
            c.lease(b).await.map_err(infra)?; // reaps a → mid (PURE) re-offered
            c.commit(b, mid, wt, workflow::pure_result_ref(mid))
                .await
                .map_err(infra)?;
            c.commit(b, leaf, wt, workflow::pure_result_ref(leaf))
                .await
                .map_err(infra)?;
        }
    }
    Ok(())
}

async fn repudiation_cascade(plan: &ChaosPlan) -> ScenarioResult {
    let mut c = Cluster::new();
    let root = workflow::pure_mote(plan.salt, 1, &[]);
    let mid = workflow::pure_mote(plan.salt, 2, &[root.id]);
    let leaf = workflow::pure_mote(plan.salt, 3, &[mid.id]);
    let wt = workflow::pure_warrant();
    for m in [&root, &mid, &leaf] {
        c.submit(m, &wt).await.map_err(infra)?;
    }
    drive_lineage(&mut c, plan, &root, &mid, &leaf, &wt).await?;

    let committed_before = c.committed_count().await.map_err(infra)?;
    let cascade = c.repudiate(root.id).await.map_err(infra)?;
    let r_state = c.state_of(root.id).await.map_err(infra)?;
    let m_state = c.state_of(mid.id).await.map_err(infra)?;
    let l_state = c.state_of(leaf.id).await.map_err(infra)?;
    let cascade_again = c.repudiate(root.id).await.map_err(infra)?; // idempotent (D15)
    let committed_after = c.committed_count().await.map_err(infra)?;

    let outcome = ChaosOutcome {
        committed_count: committed_after,
        net_effects: c.broker().net_effects(),
        dispatch_calls: c.broker().dispatch_calls(),
        safely_stuck: false,
        cascade_size: Some(cascade),
        materialized_children: 0,
    };

    if committed_before != 3 {
        return Err(bad(
            format!("cascade: lineage not fully committed before repudiation (committed_count={committed_before})"),
            outcome,
        ));
    }
    if cascade != 2 {
        return Err(bad(
            format!("cascade: expected 2 downstream consumers, got {cascade}"),
            outcome,
        ));
    }
    for (name, st) in [("root", r_state), ("mid", m_state), ("leaf", l_state)] {
        if !matches!(st, MoteState::Repudiated) {
            return Err(bad(
                format!("cascade: {name} not Repudiated after cascade (state {st:?})"),
                outcome,
            ));
        }
    }
    // Idempotent (D15): the journal dedupes by key, so re-repudiating changes no facts.
    // `cascade_size` is the *structural* cascade (so it repeats); state must stay put.
    if cascade_again != cascade {
        return Err(bad(
            format!(
                "cascade: re-repudiation not idempotent (first={cascade}, again={cascade_again})"
            ),
            outcome,
        ));
    }
    if committed_after != 0 {
        return Err(bad(
            format!("cascade: committed_count should be 0 after full repudiation, got {committed_after}"),
            outcome,
        ));
    }
    if outcome.net_effects != 0 {
        return Err(bad(
            "cascade: a non-world-mutating workflow produced a world effect",
            outcome,
        ));
    }
    Ok(outcome)
}
