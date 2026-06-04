//! Read-only seams over the journal + content store. The whole point: a
//! gateway backend can fold a projection and serve content **without ever being
//! able to write** — the traits below have no `append`/`put`, so a write cannot
//! type-check (illegal-states-unrepresentable, Rule 5.2). The dep-wall test adds
//! the second proof (no writer-crate link).

use std::ops::Range;

use kx_content::{ContentRef, ContentStore};
use kx_journal::{Journal, JournalEntry, JournalError};

/// The read-only subset of a journal a read-fold backend may touch. Deliberately
/// has no `append`/`append_batch` — gateway-core cannot name a write.
pub trait JournalReader: Send + Sync {
    /// Entries with `seq` in `range`, ascending. Mirrors
    /// [`Journal::read_entries_by_seq`].
    fn read_entries_by_seq(
        &self,
        range: Range<u64>,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError>;

    /// The largest `seq` written so far (`0` for an empty journal).
    fn current_seq(&self) -> Result<u64, JournalError>;
}

/// Wraps any [`Journal`] and exposes **only** its read methods. The inner
/// handle is private, so a holder of a `ReadOnly<J>` has no write surface.
pub struct ReadOnly<J>(J);

impl<J: Journal> ReadOnly<J> {
    /// Wrap a journal handle for read-only access.
    pub fn new(journal: J) -> Self {
        Self(journal)
    }
}

impl<J: Journal + Send + Sync> JournalReader for ReadOnly<J> {
    fn read_entries_by_seq(
        &self,
        range: Range<u64>,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        self.0.read_entries_by_seq(range)
    }

    fn current_seq(&self) -> Result<u64, JournalError> {
        self.0.current_seq()
    }
}

/// The read-only subset of a content store: fetch-by-ref / membership only — no
/// `put`. `get` returns owned bytes because the gateway forwards them over the
/// wire anyway. A blanket impl makes every [`ContentStore`] a `ContentReader`.
pub trait ContentReader: Send + Sync {
    /// The payload bytes at `content_ref`, or `None` if absent.
    fn get(&self, content_ref: &ContentRef) -> Option<Vec<u8>>;

    /// `true` if an object is present at `content_ref`.
    fn contains(&self, content_ref: &ContentRef) -> bool;
}

impl<S: ContentStore + Send + Sync> ContentReader for S {
    fn get(&self, content_ref: &ContentRef) -> Option<Vec<u8>> {
        ContentStore::get(self, content_ref)
            .ok()
            .map(|payload| payload.to_vec())
    }

    fn contains(&self, content_ref: &ContentRef) -> bool {
        ContentStore::contains(self, content_ref)
    }
}
