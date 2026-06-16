//! [`MetricsState`] — RED counters folded from durable, committed journal facts.
//!
//! The fold is **incremental + idempotent**: it advances a high-water `last_seq`
//! and only reads entries above it, so a periodic refresh is O(new entries), and
//! re-running it over an empty tail is a no-op. Every counter is a monotone sum
//! over journal entry KINDS — never a recomputed identity, never a digest input
//! (off the truth path, like [`crate`]'s sibling `kx-audit`). A scrape renders a
//! snapshot of this state; it never scans the journal itself.

use kx_gateway_core::JournalReader;
use kx_journal::{FailureReason, JournalEntry};

use crate::error::OtelError;

/// The number of [`FailureReason`] discriminants (0..=8). Kept as a constant so a
/// future journal-schema variant addition is a single-line, compiler-checked bump
/// (the `from_u8` round-trip in `reason_index` guards the mapping).
pub const FAILURE_REASON_COUNT: usize = 9;

/// Stable lowercase snake labels for each [`FailureReason`], indexed by `as_u8`.
/// These ride the Prometheus `reason="…"` label, so they are part of the metric
/// contract — append-only, never reordered (mirrors the UI `failureReasonLabel`).
pub const FAILURE_REASON_LABELS: [&str; FAILURE_REASON_COUNT] = [
    "timed_out",                 // 0 TimedOut
    "executor_refused",          // 1 ExecutorRefused
    "validator_rejected",        // 2 ValidatorRejected
    "worker_crashed",            // 3 WorkerCrashed
    "upstream_repudiated",       // 4 UpstreamRepudiated
    "unsafe_world_mutating",     // 5 UnsafeWorldMutatingConstruction
    "compensated_at_least_once", // 6 CompensatedAtLeastOnce
    "quarantined_at_least_once", // 7 QuarantinedAtLeastOnce
    "dead_lettered",             // 8 DeadLettered
];

/// RED metrics accumulated from the journal. All counters are cumulative since
/// `seq 0`; Prometheus derives rates from the counter deltas across scrapes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricsState {
    /// `RunRegistered` facts — runs admitted (the rate numerator).
    pub runs_registered: u64,
    /// `Proposed` facts — scheduler placements.
    pub proposed: u64,
    /// `Committed` facts — durable Mote effects (success).
    pub committed: u64,
    /// `Failed` facts — terminal Mote failures (the error count).
    pub failed: u64,
    /// `Failed` facts bucketed by [`FailureReason`] (indexed by `as_u8`).
    pub failed_by_reason: [u64; FAILURE_REASON_COUNT],
    /// `Repudiated` facts — committed Motes later invalidated.
    pub repudiated: u64,
    /// `EffectStaged` facts — WORLD-MUTATING intents durably staged.
    pub effect_staged: u64,
    /// The highest journal `seq` folded so far (the incremental high-water mark).
    pub last_seq: u64,
}

impl MetricsState {
    /// A fresh, all-zero state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The success ratio `committed / (committed + failed)` scaled to basis points
    /// (0..=10000), or `None` when no terminal outcome has been observed. Integer
    /// math only (no float on any path) — a Prometheus consumer divides by 10000.
    #[must_use]
    pub fn success_ratio_bp(&self) -> Option<u64> {
        let terminal = self.committed + self.failed;
        (terminal > 0).then(|| (self.committed * 10_000) / terminal)
    }

    /// Fold all journal entries with `seq > last_seq` into the counters and advance
    /// `last_seq` to the journal head. Idempotent: a refresh with no new entries is
    /// a no-op. Reads through the [`JournalReader`] read-only seam — there is no
    /// `append` it could name (illegal-states-unrepresentable).
    ///
    /// # Errors
    /// Returns [`OtelError::Journal`] if reading the journal head or tail fails;
    /// the counters are left unchanged so the caller can serve the last snapshot.
    pub fn fold_from(&mut self, reader: &dyn JournalReader) -> Result<(), OtelError> {
        let head = reader.current_seq()?;
        if head <= self.last_seq {
            return Ok(());
        }
        // `seq` is 1-based (RunRegistered is seq 1); read the open tail (last, head].
        for entry in reader.read_entries_by_seq((self.last_seq + 1)..(head + 1))? {
            self.apply(&entry);
        }
        // Advance to the head we read through — robust even if a kind we don't
        // bucket (off-DAG metadata facts) is the highest seq in the tail.
        self.last_seq = head;
        Ok(())
    }

    /// Apply one entry to the counters (the per-kind bucketing).
    fn apply(&mut self, entry: &JournalEntry) {
        match entry {
            JournalEntry::RunRegistered { .. } => self.runs_registered += 1,
            JournalEntry::Proposed { .. } => self.proposed += 1,
            JournalEntry::Committed { .. } => self.committed += 1,
            JournalEntry::Failed { reason_class, .. } => {
                self.failed += 1;
                self.failed_by_reason[reason_index(*reason_class)] += 1;
            }
            JournalEntry::Repudiated { .. } => self.repudiated += 1,
            JournalEntry::EffectStaged { .. } => self.effect_staged += 1,
            // Off-DAG metadata facts (RunVersionsResolved, DigestSealed,
            // ReplanRound, ReactRound, …) are not RED signals — counted only by
            // the `last_seq` high-water advance in `fold_from`.
            _ => {}
        }
    }
}

/// Map a [`FailureReason`] to its `failed_by_reason` array index. Uses the
/// canonical `as_u8`, clamped defensively to the array bounds (a forward-compat
/// journal could in principle carry a discriminant this build does not know — it
/// is bucketed into the last slot rather than panicking, fail-open).
fn reason_index(reason: FailureReason) -> usize {
    let idx = reason.as_u8() as usize;
    idx.min(FAILURE_REASON_COUNT - 1)
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use kx_content::ContentRef;
    use kx_journal::{FailureReason, JournalEntry, JournalError};
    use kx_mote::{MoteDefHash, MoteId, NdClass};

    use super::*;

    /// A hand-built reader over a fixed entry list — tests the fold in isolation
    /// from journal append/dedup/ordering rules.
    struct MockReader {
        entries: Vec<(u64, JournalEntry)>,
    }

    impl JournalReader for MockReader {
        fn read_entries_by_seq(
            &self,
            range: Range<u64>,
        ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
            let v: Vec<JournalEntry> = self
                .entries
                .iter()
                .filter(|(seq, _)| range.contains(seq))
                .map(|(_, e)| e.clone())
                .collect();
            Ok(Box::new(v.into_iter()))
        }

        fn current_seq(&self) -> Result<u64, JournalError> {
            Ok(self.entries.iter().map(|(s, _)| *s).max().unwrap_or(0))
        }
    }

    fn run_registered(seq: u64) -> JournalEntry {
        JournalEntry::RunRegistered {
            instance_id: [seq as u8; 16],
            recipe_fingerprint: [0; 32],
            ts: 0,
            seq,
        }
    }

    fn committed(seq: u64) -> JournalEntry {
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes([seq as u8; 32]),
            idempotency_key: [seq as u8; 32],
            seq,
            nondeterminism: NdClass::Pure,
            result_ref: ContentRef::from_bytes([seq as u8; 32]),
            parents: Default::default(),
            warrant_ref: ContentRef::from_bytes([0; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0; 32]),
        }
    }

    fn failed(seq: u64, reason: FailureReason) -> JournalEntry {
        JournalEntry::Failed {
            mote_id: MoteId::from_bytes([seq as u8; 32]),
            idempotency_key: [seq as u8; 32],
            seq,
            reason_class: reason,
            reporter_id: 0,
        }
    }

    #[test]
    fn folds_red_counters_from_a_fixed_journal() {
        let reader = MockReader {
            entries: vec![
                (1, run_registered(1)),
                (2, committed(2)),
                (3, committed(3)),
                (4, failed(4, FailureReason::TimedOut)),
                (5, failed(5, FailureReason::DeadLettered)),
            ],
        };
        let mut state = MetricsState::new();
        state.fold_from(&reader).unwrap();

        assert_eq!(state.runs_registered, 1);
        assert_eq!(state.committed, 2);
        assert_eq!(state.failed, 2);
        assert_eq!(
            state.failed_by_reason[FailureReason::TimedOut.as_u8() as usize],
            1
        );
        assert_eq!(
            state.failed_by_reason[FailureReason::DeadLettered.as_u8() as usize],
            1
        );
        assert_eq!(state.last_seq, 5);
        // 2 committed / 4 terminal = 5000 bp.
        assert_eq!(state.success_ratio_bp(), Some(5_000));
    }

    #[test]
    fn fold_is_incremental_and_idempotent() {
        let mut entries = vec![(1, run_registered(1)), (2, committed(2))];
        let mut state = MetricsState::new();
        state
            .fold_from(&MockReader {
                entries: entries.clone(),
            })
            .unwrap();
        assert_eq!(state.committed, 1);
        assert_eq!(state.last_seq, 2);

        // Re-folding the SAME journal adds nothing (idempotent).
        state
            .fold_from(&MockReader {
                entries: entries.clone(),
            })
            .unwrap();
        assert_eq!(state.committed, 1, "re-fold must not double-count");

        // Append a new commit; only the new tail is folded.
        entries.push((3, committed(3)));
        state.fold_from(&MockReader { entries }).unwrap();
        assert_eq!(state.committed, 2);
        assert_eq!(state.last_seq, 3);
    }

    #[test]
    fn empty_journal_is_all_zero() {
        let state = {
            let mut s = MetricsState::new();
            s.fold_from(&MockReader { entries: vec![] }).unwrap();
            s
        };
        assert_eq!(state, MetricsState::default());
        assert_eq!(state.success_ratio_bp(), None);
    }

    /// GR10 spike (run `cargo test -p kx-otel --release -- --ignored fold_spike
    /// --nocapture`): the cost of a full cold fold over an N-entry journal + a
    /// render. The fold is incremental in production (a tick reads only the new
    /// tail), so this cold pass is the conservative ceiling. Off the hot path; the
    /// scrape never folds (it serves the cached snapshot). Numbers persist to the
    /// PRIVATE `docs/benchmarks/` (SN-2), never asserted (a ratio gate would belong
    /// in `scale-smoke`).
    #[test]
    #[ignore = "perf spike — run explicitly with --release --ignored --nocapture"]
    fn fold_spike() {
        use std::time::Instant;
        const N: u64 = 50_000;
        let mut entries = Vec::with_capacity(N as usize);
        for seq in 1..=N {
            // A representative mix: one run, mostly commits, ~10% failures.
            let e = match seq % 10 {
                0 => failed(seq, FailureReason::TimedOut),
                1 => run_registered(seq),
                _ => committed(seq),
            };
            entries.push((seq, e));
        }
        let reader = MockReader { entries };

        let t0 = Instant::now();
        let mut state = MetricsState::new();
        state.fold_from(&reader).unwrap();
        let fold = t0.elapsed();

        let t1 = Instant::now();
        let body =
            crate::render::render(&state, &crate::render::BuildInfo { version: "spike" }, None);
        let render = t1.elapsed();

        let per_entry_ns = fold.as_nanos() / u128::from(N);
        println!(
            "kx-otel fold_spike: N={N} fold={fold:?} ({per_entry_ns} ns/entry) \
             render={render:?} body_bytes={}",
            body.len()
        );
    }
}
