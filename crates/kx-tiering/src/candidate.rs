//! Tag-driven selection of evictable PURE payload refs.

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_mote::NdClass;
use kx_projection::{MoteState, Snapshot};

/// One evictable PURE result_ref together with its eviction-order key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionCandidate {
    /// The content ref backing one or more committed PURE Motes.
    pub result_ref: ContentRef,
    /// The smallest committing `seq` among the PURE Motes that resolve to this
    /// ref. Eviction is oldest-first, so candidates are returned ascending by
    /// this key (ties broken by `result_ref` for determinism).
    pub min_seq: u64,
}

/// Per-ref accumulator while folding the snapshot.
#[derive(Clone, Copy)]
struct RefTally {
    /// `true` while every committed contributor seen so far is PURE.
    all_pure: bool,
    /// Smallest committing `seq` seen for this ref.
    min_seq: u64,
}

/// Compute the eviction-ordered list of PURE-only candidate refs from a
/// projection snapshot.
///
/// A ref is a candidate **iff every committed, non-repudiated Mote that resolves
/// to it is [`NdClass::Pure`]**. Content-addressed dedup means a PURE and a
/// WORLD-MUTATING / READ-ONLY-NONDET Mote can share a ref; any non-PURE
/// contributor protects the ref from eviction. Repudiated Motes neither protect
/// nor offer a ref — their payloads are governed by orphan-GC/retention, not
/// tiering — so only `MoteState::Committed` Motes vote.
///
/// Pure tag logic: this does **not** touch the content store. Result is ordered
/// oldest-committing-`seq` first (the eviction order [`crate::run_pass`] consumes).
#[must_use]
pub fn select_candidates(snapshot: &Snapshot) -> Vec<EvictionCandidate> {
    let mut refs: BTreeMap<ContentRef, RefTally> = BTreeMap::new();

    for (mote_id, state) in snapshot.iter_motes() {
        // Only committed-and-not-repudiated Motes vote.
        if state != MoteState::Committed {
            continue;
        }
        // A Committed state without a folded result_ref/nd is not expected;
        // skip defensively rather than guess.
        let (Some(result_ref), Some(nd)) = (
            snapshot.result_ref_of(&mote_id),
            snapshot.nondeterminism_of(&mote_id),
        ) else {
            continue;
        };
        let seq = snapshot.committed_seq_of(&mote_id).unwrap_or(u64::MAX);
        let is_pure = nd == NdClass::Pure;

        refs.entry(result_ref)
            .and_modify(|t| {
                t.all_pure &= is_pure;
                t.min_seq = t.min_seq.min(seq);
            })
            .or_insert(RefTally {
                all_pure: is_pure,
                min_seq: seq,
            });
    }

    let mut candidates: Vec<EvictionCandidate> = refs
        .into_iter()
        .filter(|(_, t)| t.all_pure)
        .map(|(result_ref, t)| EvictionCandidate {
            result_ref,
            min_seq: t.min_seq,
        })
        .collect();
    // Oldest-commit-first; deterministic tie-break by ref.
    candidates.sort_by(|a, b| {
        a.min_seq
            .cmp(&b.min_seq)
            .then_with(|| a.result_ref.cmp(&b.result_ref))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_content::ContentRef;
    use kx_journal::{InMemoryJournal, Journal, JournalEntry, RepudiationReason};
    use kx_mote::{MoteDefHash, MoteId};
    use smallvec::SmallVec;

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }
    fn cref(b: u8) -> ContentRef {
        ContentRef::from_bytes([b; 32])
    }
    fn dh(b: u8) -> MoteDefHash {
        MoteDefHash::from_bytes([b; 32])
    }

    /// Append a Committed entry; returns the journal-assigned seq.
    fn commit(j: &InMemoryJournal, mote: u8, result: u8, nd: NdClass) -> u64 {
        let e = j
            .append(JournalEntry::Committed {
                mote_id: mid(mote),
                idempotency_key: [mote; 32],
                seq: 0, // ignored; the journal assigns
                nondeterminism: nd,
                result_ref: cref(result),
                parents: SmallVec::new(),
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                mote_def_hash: dh(mote),
            })
            .unwrap();
        e.seq()
    }

    fn repudiate(j: &InMemoryJournal, mote: u8, target_seq: u64) {
        j.append(JournalEntry::Repudiated {
            target_mote_id: mid(mote),
            idempotency_key: [0xee ^ mote; 32],
            seq: 0,
            target_committed_seq: target_seq,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 1,
        })
        .unwrap();
    }

    fn snapshot_of(j: &InMemoryJournal) -> Snapshot {
        kx_projection::Projection::from_journal(j)
            .unwrap()
            .snapshot()
    }

    #[test]
    fn pure_only_ref_is_evictable() {
        let j = InMemoryJournal::new();
        commit(&j, b'a', b'a', NdClass::Pure);
        let c = select_candidates(&snapshot_of(&j));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].result_ref, cref(b'a'));
    }

    #[test]
    fn wm_ref_never_selected() {
        let j = InMemoryJournal::new();
        commit(&j, b'a', b'a', NdClass::WorldMutating);
        assert!(select_candidates(&snapshot_of(&j)).is_empty());
    }

    #[test]
    fn rond_ref_never_selected() {
        let j = InMemoryJournal::new();
        commit(&j, b'a', b'a', NdClass::ReadOnlyNondet);
        assert!(select_candidates(&snapshot_of(&j)).is_empty());
    }

    #[test]
    fn shared_ref_pure_plus_wm_is_protected() {
        // Two distinct Motes, identical payload bytes => same ContentRef.
        let j = InMemoryJournal::new();
        commit(&j, b'a', b'x', NdClass::Pure);
        commit(&j, b'b', b'x', NdClass::WorldMutating);
        // The shared ref is protected by its WM contributor.
        assert!(select_candidates(&snapshot_of(&j)).is_empty());
    }

    #[test]
    fn shared_ref_pure_plus_pure_is_evictable() {
        let j = InMemoryJournal::new();
        commit(&j, b'a', b'x', NdClass::Pure);
        commit(&j, b'b', b'x', NdClass::Pure);
        let c = select_candidates(&snapshot_of(&j));
        assert_eq!(c.len(), 1, "deduped to a single ref");
        assert_eq!(c[0].result_ref, cref(b'x'));
    }

    #[test]
    fn repudiated_pure_mote_excluded_from_voting() {
        // A repudiated PURE Mote must NOT make a WM-shared ref evictable...
        let j = InMemoryJournal::new();
        let pure_seq = commit(&j, b'a', b'x', NdClass::Pure);
        commit(&j, b'b', b'x', NdClass::WorldMutating);
        repudiate(&j, b'a', pure_seq); // PURE side repudiated
                                       // ...the live WM contributor still protects the ref.
        assert!(select_candidates(&snapshot_of(&j)).is_empty());

        // ...and a repudiated-only PURE ref offers nothing (no live contributor).
        let j2 = InMemoryJournal::new();
        let s = commit(&j2, b'c', b'y', NdClass::Pure);
        repudiate(&j2, b'c', s);
        assert!(select_candidates(&snapshot_of(&j2)).is_empty());
    }

    #[test]
    fn candidates_ordered_oldest_seq_first() {
        let j = InMemoryJournal::new();
        commit(&j, b'a', b'a', NdClass::Pure); // seq 0
        commit(&j, b'b', b'b', NdClass::Pure); // seq 1
        commit(&j, b'c', b'c', NdClass::Pure); // seq 2
        let c = select_candidates(&snapshot_of(&j));
        let seqs: Vec<u64> = c.iter().map(|x| x.min_seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted, "ascending by min_seq");
        assert_eq!(c[0].result_ref, cref(b'a'));
    }
}
