//! The tiering pass: budget-driven eviction of PURE payloads.

use kx_content::{ContentRef, ContentStore};
use kx_projection::Snapshot;

use crate::candidate::select_candidates;
use crate::error::TieringError;
use crate::policy::{ResidentUsage, TieringBudget};

/// The outcome of one tiering pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvictionReport {
    /// Refs deleted from the store during this pass, in eviction order.
    pub evicted: Vec<ContentRef>,
    /// Total bytes reclaimed (sum of the evicted payloads' sizes).
    pub bytes_reclaimed: u64,
    /// Number of PURE-only candidate refs the snapshot offered.
    pub candidates_considered: usize,
    /// Candidates already absent from the store (skipped — a prior pass or the
    /// orphan-GC walker already removed them; deletion is idempotent).
    pub skipped_absent: usize,
    /// Resident PURE footprint before eviction.
    pub usage_before: ResidentUsage,
    /// Resident PURE footprint after eviction.
    pub usage_after: ResidentUsage,
}

/// One resident PURE payload, sized for budgeting.
struct Resident {
    result_ref: ContentRef,
    size: u64,
}

/// Run one tiering pass: select PURE-only candidates (shared-ref-protected,
/// repudiated-excluded — see [`select_candidates`]), then delete them
/// oldest-commit-first until the resident PURE footprint is within `budget`.
///
/// WORLD-MUTATING and READ-ONLY-NONDET refs are never candidates, so they are
/// never deleted — regardless of how tight the budget is (a `MaxObjects(0)` /
/// `MaxBytes(0)` budget evicts every PURE payload and leaves the protected tags
/// untouched). The pass is idempotent: re-running after eviction finds the
/// already-deleted refs absent and reports them as `skipped_absent`.
#[tracing::instrument(skip(snapshot, store), fields(budget = ?budget))]
pub fn run_pass<S: ContentStore>(
    snapshot: &Snapshot,
    store: &S,
    budget: TieringBudget,
) -> Result<EvictionReport, TieringError> {
    let candidates = select_candidates(snapshot);
    let candidates_considered = candidates.len();

    // Size each candidate against the store (oldest-first order preserved).
    // A NotFound is a normal outcome — the payload is already gone — not an error.
    let mut resident: Vec<Resident> = Vec::with_capacity(candidates.len());
    let mut skipped_absent = 0usize;
    for c in &candidates {
        match store.get(&c.result_ref) {
            Ok(payload) => resident.push(Resident {
                result_ref: c.result_ref,
                size: u64::try_from(payload.len()).unwrap_or(u64::MAX),
            }),
            Err(_not_found) => skipped_absent += 1,
        }
    }

    let usage_before = ResidentUsage {
        objects: resident.len(),
        bytes: resident.iter().map(|r| r.size).sum(),
    };

    let mut current = usage_before;
    let mut evicted = Vec::new();
    let mut bytes_reclaimed = 0u64;

    // Evict oldest-first until within budget (or nothing left to evict).
    let mut iter = resident.into_iter();
    while !budget.is_satisfied(current) {
        let Some(r) = iter.next() else { break };
        store.delete(&r.result_ref)?;
        evicted.push(r.result_ref);
        bytes_reclaimed += r.size;
        current.objects -= 1;
        current.bytes -= r.size;
    }

    Ok(EvictionReport {
        evicted,
        bytes_reclaimed,
        candidates_considered,
        skipped_absent,
        usage_before,
        usage_after: current,
    })
}
