//! `InMemoryJournal` — an in-memory [`Journal`] backend.
//!
//! Exists for two reasons (same pattern as `kx-content`'s `InMemoryContentStore`):
//!
//! 1. **Trait-seam proof.** A second backend with a different storage substrate proves
//!    the [`Journal`] trait carries no SQLite-specific assumption in its signature.
//! 2. **Downstream test fixtures.** The projection (P1.5) and executor (P1.9) test
//!    suites can use this backend without touching the filesystem.
//!
//! **Not for production.** No durability across process restarts. Use [`SqliteJournal`]
//! or a future replicated backend in any deployment.

use std::ops::Range;
use std::sync::RwLock;

use kx_content::ContentRef;
use kx_mote::{MoteDefHash, MoteId};

use crate::entry::{
    repudiation_idempotency_key, JournalEntry, KIND_COMMITTED, KIND_EFFECT_STAGED, KIND_REPUDIATED,
};
use crate::{Journal, JournalError};

#[derive(Default)]
struct State {
    entries: Vec<JournalEntry>,
    next_seq: u64,
}

/// An in-memory [`Journal`] backed by a `Vec<JournalEntry>` under an `RwLock`.
///
/// Not durable across process restarts. Used as a trait-seam proof (the
/// `Journal` trait carries no in-process or filesystem assumption) AND as a
/// cheap deterministic fixture for downstream test suites.
///
/// # Examples
///
/// ```
/// use kx_journal::{InMemoryJournal, Journal};
///
/// let j = InMemoryJournal::new();
/// assert_eq!(j.count_entries().unwrap(), 0);
/// assert_eq!(j.current_seq().unwrap(), 0);
/// ```
#[derive(Default)]
pub struct InMemoryJournal {
    state: RwLock<State>,
}

impl std::fmt::Debug for InMemoryJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryJournal")
            .field("len", &self.count_entries().unwrap_or(0))
            .finish()
    }
}

impl InMemoryJournal {
    /// Construct an empty journal.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Journal for InMemoryJournal {
    fn append(&self, mut entry: JournalEntry) -> Result<JournalEntry, JournalError> {
        // Derive Repudiated idempotency_key from target (D15).
        if let JournalEntry::Repudiated {
            target_mote_id,
            target_committed_seq,
            ref mut idempotency_key,
            ..
        } = entry
        {
            *idempotency_key = repudiation_idempotency_key(&target_mote_id, target_committed_seq);
        }

        let mut state = self.state.write().expect("poisoned lock");
        let kind = entry.kind();

        // v2 (D38 §2b): dedupe-by-key index expands from {1, 2} to {1, 2, 4}.
        // EffectStaged participates — second-write of the same staged-intent
        // is a no-op success. Failed is INTENTIONALLY OUT (each retry is its
        // own attempt-fact; collapsing would lose attempt history).
        if kind == KIND_COMMITTED || kind == KIND_REPUDIATED || kind == KIND_EFFECT_STAGED {
            let key = *entry.idempotency_key();
            if let Some(existing) = state
                .entries
                .iter()
                .find(|e| e.kind() == kind && *e.idempotency_key() == key)
            {
                return Ok(existing.clone());
            }
        }

        // Assign next monotonic seq.
        state.next_seq += 1;
        let next_seq = state.next_seq;
        set_seq(&mut entry, next_seq);

        state.entries.push(entry.clone());
        Ok(entry)
    }

    fn append_batch(&self, entries: Vec<JournalEntry>) -> Result<Vec<JournalEntry>, JournalError> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        // Hold the write lock for the whole batch and stage all results before
        // committing them to `state`, so the batch is atomic (all-or-nothing) and
        // isolated from concurrent readers/writers. Within-batch duplicates dedupe
        // against both the committed log and the staged entries.
        let mut state = self.state.write().expect("poisoned lock");
        let mut next_seq = state.next_seq;
        let mut staged: Vec<JournalEntry> = Vec::new();
        let mut durable = Vec::with_capacity(entries.len());

        for mut entry in entries {
            if let JournalEntry::Repudiated {
                target_mote_id,
                target_committed_seq,
                ref mut idempotency_key,
                ..
            } = entry
            {
                *idempotency_key =
                    repudiation_idempotency_key(&target_mote_id, target_committed_seq);
            }

            let kind = entry.kind();
            if kind == KIND_COMMITTED || kind == KIND_REPUDIATED || kind == KIND_EFFECT_STAGED {
                let key = *entry.idempotency_key();
                if let Some(existing) = state
                    .entries
                    .iter()
                    .chain(staged.iter())
                    .find(|e| e.kind() == kind && *e.idempotency_key() == key)
                {
                    durable.push(existing.clone());
                    continue;
                }
            }

            next_seq += 1;
            set_seq(&mut entry, next_seq);
            staged.push(entry.clone());
            durable.push(entry);
        }

        state.entries.extend(staged);
        state.next_seq = next_seq;
        Ok(durable)
    }

    fn read_committed(&self, mote_id: &MoteId) -> Result<Option<JournalEntry>, JournalError> {
        let state = self.state.read().expect("poisoned lock");
        Ok(state
            .entries
            .iter()
            .find(|e| matches!(e, JournalEntry::Committed { mote_id: m, .. } if m == mote_id))
            .cloned())
    }

    fn read_entries_by_seq(
        &self,
        range: Range<u64>,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        let state = self.state.read().expect("poisoned lock");
        let mut filtered: Vec<JournalEntry> = state
            .entries
            .iter()
            .filter(|e| {
                let s = e.seq();
                s >= range.start && s < range.end
            })
            .cloned()
            .collect();
        filtered.sort_by_key(JournalEntry::seq);
        Ok(Box::new(filtered.into_iter()))
    }

    fn list_committed_refs(
        &self,
    ) -> Result<Box<dyn Iterator<Item = ContentRef> + '_>, JournalError> {
        let state = self.state.read().expect("poisoned lock");
        let refs: Vec<ContentRef> = state
            .entries
            .iter()
            .filter_map(|e| match e {
                JournalEntry::Committed { result_ref, .. } => Some(*result_ref),
                _ => None,
            })
            .collect();
        Ok(Box::new(refs.into_iter()))
    }

    fn list_committed_by_mote_def_hash(
        &self,
        h: &MoteDefHash,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        let state = self.state.read().expect("poisoned lock");
        let matches: Vec<JournalEntry> = state
            .entries
            .iter()
            .filter(|e| {
                matches!(e, JournalEntry::Committed { mote_def_hash, .. } if mote_def_hash == h)
            })
            .cloned()
            .collect();
        Ok(Box::new(matches.into_iter()))
    }

    fn current_seq(&self) -> Result<u64, JournalError> {
        let state = self.state.read().expect("poisoned lock");
        Ok(state.next_seq)
    }

    fn count_entries(&self) -> Result<u64, JournalError> {
        let state = self.state.read().expect("poisoned lock");
        Ok(state.entries.len() as u64)
    }
}

fn set_seq(entry: &mut JournalEntry, new_seq: u64) {
    match entry {
        JournalEntry::Proposed { seq, .. }
        | JournalEntry::Committed { seq, .. }
        | JournalEntry::Repudiated { seq, .. }
        | JournalEntry::Failed { seq, .. }
        | JournalEntry::EffectStaged { seq, .. }
        | JournalEntry::RunRegistered { seq, .. } => *seq = new_seq,
    }
}
