//! Poison-invalidation cascade (P3.5 / P0.7, D22). When a committed Mote is repudiated,
//! every committed Mote downstream of it (reachable via Data edges, and Control edges
//! that did not opt out of cascade) is **also** repudiated — `committed = truth`, so a
//! wrong commit's blast radius must be contained explicitly (the runtime cannot recompute
//! its way out: committed results are facts, not functions).
//!
//! This module is the **walker + entry builder** — a pure function of a projection
//! snapshot. The coordinator's sole-writer thread (D40) runs it and appends the resulting
//! `Repudiated` batch atomically (mirroring the P3.2 reap-write). The BFS itself + the
//! cascade edge rule live in `kx_projection::transitive_consumers`
//! (`control-edge-cascade-default.md`); this code applies the **fail-not-recompute**
//! policy (D22 §5: mark `Repudiated`, never auto-re-Propose) and the sanity ceiling.

use kx_journal::{repudiation_idempotency_key, JournalEntry, RepudiationReason};
use kx_mote::MoteId;
use kx_projection::Projection;

/// Sanity ceiling on a single cascade (target + downstream). A repudiation whose
/// invalidation set exceeds this is refused (D22 — mass-repudiation needs an explicit
/// operator override, deferred to the P4.5 ops UX); it bounds the worst-case write batch
/// and surfaces a likely mistaken mass-repudiation instead of silently rewriting the run.
pub(crate) const DEFAULT_CASCADE_CEILING: usize = 10_000;

/// Reporter id stamped on a coordinator-written cascade `Repudiated` entry (D22 §5):
/// the cascade is the coordinator's consequence of the operator's seed repudiation.
const COORDINATOR_REPUDIATOR_ID: u128 = 0;

/// Outcome of a repudiation: the seed target plus the number of downstream Motes the
/// cascade newly repudiated (a re-repudiation of an already-repudiated set reports `0`
/// newly-cascaded — the journal dedupes by key, D15).
#[derive(Debug, Clone, Copy)]
pub struct RepudiationOutcome {
    /// The seed Mote that was repudiated.
    pub target: MoteId,
    /// How many downstream committed Motes the cascade repudiated this call.
    pub cascade_size: usize,
}

/// Why a repudiation could not be performed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepudiationError {
    /// The target is not a committed Mote — only committed facts can be repudiated
    /// (`repudiation.md` §1: in-flight attempts are not repudiable).
    #[error("repudiation target {0:?} is not committed")]
    TargetNotCommitted(MoteId),
    /// The invalidation set (target + downstream) exceeds the sanity ceiling.
    #[error(
        "cascade of {size} exceeds the ceiling of {ceiling}; refusing (operator override is P4.5)"
    )]
    CascadeTooLarge {
        /// The would-be invalidation-set size (target + downstream).
        size: usize,
        /// The configured ceiling.
        ceiling: usize,
    },
    /// The journal append of the cascade batch failed (catastrophic — the batch is
    /// atomic, so nothing was written).
    #[error("repudiation journal write failed: {0}")]
    Append(String),
    /// The orchestration core is gone (channel closed on shutdown).
    #[error("coordinator core unavailable")]
    CoreUnavailable,
}

/// The minimal read surface the cascade walker needs — the downstream closure and each
/// Mote's committed seq. Behind a trait so the walker is unit-testable without a full
/// projection (and so the dependency is explicit).
pub(crate) trait CascadeGraph {
    /// Downstream consumers reachable via the cascade edge rule (Data always; Control
    /// unless opted out). Excludes `target` itself. Cycle-safe.
    fn transitive_consumers(&self, target: &MoteId) -> Vec<MoteId>;
    /// The seq of `mote`'s `Committed` entry, or `None` if not committed.
    fn committed_seq_of(&self, mote: &MoteId) -> Option<u64>;
}

impl CascadeGraph for Projection {
    fn transitive_consumers(&self, target: &MoteId) -> Vec<MoteId> {
        Projection::transitive_consumers(self, target)
    }
    fn committed_seq_of(&self, mote: &MoteId) -> Option<u64> {
        Projection::committed_seq_of(self, mote)
    }
}

/// Build the `Repudiated` batch for repudiating `target` with `reason` and cascading to
/// its committed downstream consumers (each `UpstreamCascade`). Pure: it only reads the
/// graph and constructs entries — the caller appends them through the sole writer. The
/// target is entry 0 (its own `reason`); the rest are the cascade. A downstream Mote that
/// is not committed is skipped (nothing to repudiate — `ready_set` already gates it on its
/// repudiated parent). Idempotency keys are derived per D15, so re-repudiating dedupes.
pub(crate) fn cascade_repudiation_entries<G: CascadeGraph>(
    graph: &G,
    target: MoteId,
    reason: RepudiationReason,
    repudiator_id: u128,
    ceiling: usize,
) -> Result<Vec<JournalEntry>, RepudiationError> {
    let Some(target_seq) = graph.committed_seq_of(&target) else {
        return Err(RepudiationError::TargetNotCommitted(target));
    };
    let downstream: Vec<(MoteId, u64)> = graph
        .transitive_consumers(&target)
        .into_iter()
        .filter_map(|c| graph.committed_seq_of(&c).map(|seq| (c, seq)))
        .collect();
    let size = 1 + downstream.len();
    if size > ceiling {
        return Err(RepudiationError::CascadeTooLarge { size, ceiling });
    }
    let mut entries = Vec::with_capacity(size);
    entries.push(repudiated_entry(target, target_seq, reason, repudiator_id));
    for (consumer, seq) in downstream {
        entries.push(repudiated_entry(
            consumer,
            seq,
            RepudiationReason::UpstreamCascade,
            COORDINATOR_REPUDIATOR_ID,
        ));
    }
    Ok(entries)
}

/// A `Repudiated` entry (`seq` is journal-assigned on append; the idempotency key is
/// derived per D15 so duplicate repudiations of the same `(target, committed_seq)` dedupe).
fn repudiated_entry(
    target_mote_id: MoteId,
    target_committed_seq: u64,
    reason_class: RepudiationReason,
    repudiator_id: u128,
) -> JournalEntry {
    JournalEntry::Repudiated {
        target_mote_id,
        idempotency_key: repudiation_idempotency_key(&target_mote_id, target_committed_seq),
        seq: 0,
        target_committed_seq,
        reason_class,
        repudiator_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A hand-built graph: `committed` maps Mote → committed_seq; `consumers` maps a
    /// target → its (already cascade-filtered) downstream closure.
    struct MockGraph {
        committed: BTreeMap<MoteId, u64>,
        consumers: BTreeMap<MoteId, Vec<MoteId>>,
    }
    impl CascadeGraph for MockGraph {
        fn transitive_consumers(&self, target: &MoteId) -> Vec<MoteId> {
            self.consumers.get(target).cloned().unwrap_or_default()
        }
        fn committed_seq_of(&self, mote: &MoteId) -> Option<u64> {
            self.committed.get(mote).copied()
        }
    }

    fn m(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    fn reason_of(e: &JournalEntry) -> RepudiationReason {
        match e {
            JournalEntry::Repudiated { reason_class, .. } => *reason_class,
            _ => panic!("expected Repudiated"),
        }
    }
    fn target_of(e: &JournalEntry) -> MoteId {
        match e {
            JournalEntry::Repudiated { target_mote_id, .. } => *target_mote_id,
            _ => panic!("expected Repudiated"),
        }
    }

    #[test]
    fn cascades_to_committed_downstream_with_upstream_cascade_reason() {
        let graph = MockGraph {
            committed: [(m(1), 1), (m(2), 2), (m(3), 3)].into_iter().collect(),
            consumers: [(m(1), vec![m(2), m(3)])].into_iter().collect(),
        };
        let entries =
            cascade_repudiation_entries(&graph, m(1), RepudiationReason::OperatorAction, 7, 10_000)
                .unwrap();
        assert_eq!(entries.len(), 3, "target + 2 downstream");
        assert_eq!(target_of(&entries[0]), m(1));
        assert_eq!(reason_of(&entries[0]), RepudiationReason::OperatorAction);
        // Both downstream carry UpstreamCascade.
        for e in &entries[1..] {
            assert_eq!(reason_of(e), RepudiationReason::UpstreamCascade);
        }
    }

    #[test]
    fn skips_uncommitted_downstream() {
        // m(3) is downstream but NOT committed → not repudiated (nothing to invalidate).
        let graph = MockGraph {
            committed: [(m(1), 1), (m(2), 2)].into_iter().collect(),
            consumers: [(m(1), vec![m(2), m(3)])].into_iter().collect(),
        };
        let entries =
            cascade_repudiation_entries(&graph, m(1), RepudiationReason::OperatorAction, 7, 10_000)
                .unwrap();
        assert_eq!(
            entries.len(),
            2,
            "target + only the committed downstream m(2)"
        );
        assert_eq!(target_of(&entries[1]), m(2));
    }

    #[test]
    fn refuses_uncommitted_target() {
        let graph = MockGraph {
            committed: BTreeMap::new(),
            consumers: BTreeMap::new(),
        };
        let err =
            cascade_repudiation_entries(&graph, m(9), RepudiationReason::OperatorAction, 7, 10_000)
                .unwrap_err();
        assert_eq!(err, RepudiationError::TargetNotCommitted(m(9)));
    }

    #[test]
    fn refuses_cascade_over_ceiling() {
        let downstream: Vec<MoteId> = (10u8..20).map(m).collect();
        let mut committed: BTreeMap<MoteId, u64> = [(m(1), 1)].into_iter().collect();
        for (i, d) in downstream.iter().enumerate() {
            committed.insert(*d, 100 + i as u64);
        }
        let graph = MockGraph {
            committed,
            consumers: [(m(1), downstream)].into_iter().collect(),
        };
        // ceiling = 5, but the set is 1 + 10 = 11 → refused.
        let err =
            cascade_repudiation_entries(&graph, m(1), RepudiationReason::OperatorAction, 7, 5)
                .unwrap_err();
        assert_eq!(
            err,
            RepudiationError::CascadeTooLarge {
                size: 11,
                ceiling: 5
            }
        );
    }
}
