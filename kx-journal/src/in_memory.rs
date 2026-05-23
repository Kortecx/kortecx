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

use crate::entry::{repudiation_idempotency_key, JournalEntry, KIND_COMMITTED, KIND_REPUDIATED};
use crate::{Journal, JournalError};

#[derive(Default)]
struct State {
    entries: Vec<JournalEntry>,
    next_seq: u64,
}

/// An in-memory [`Journal`] backed by a `Vec<JournalEntry>` under an `RwLock`.
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

        // Dedupe-by-key for Committed + Repudiated only.
        if kind == KIND_COMMITTED || kind == KIND_REPUDIATED {
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
        | JournalEntry::Failed { seq, .. } => *seq = new_seq,
    }
}
