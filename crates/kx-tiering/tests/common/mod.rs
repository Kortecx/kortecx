//! Shared fixtures for the kx-tiering integration tests.
//!
//! A [`Fixture`] owns an in-memory content store + journal and commits Motes
//! whose `result_ref` is the *real* content-addressed ref of the payload bytes
//! (so eviction actually targets a stored object). This mirrors the runtime's
//! put-then-commit discipline closely enough to exercise the tiering contract.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code, unreachable_pub)]

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry, RepudiationReason};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_projection::{Projection, Snapshot};
use smallvec::SmallVec;

pub fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}
pub fn dh(b: u8) -> MoteDefHash {
    MoteDefHash::from_bytes([b; 32])
}

pub struct Fixture {
    pub store: InMemoryContentStore,
    pub journal: InMemoryJournal,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            store: InMemoryContentStore::new(),
            journal: InMemoryJournal::new(),
        }
    }

    /// Put `payload` and commit Mote `mote` (tag `nd`) referencing it by its
    /// real content-addressed ref. Returns the ref and the journal-assigned seq.
    pub fn commit_payload(&self, mote: u8, payload: &[u8], nd: NdClass) -> (ContentRef, u64) {
        let result_ref = self.store.put(payload).unwrap();
        let seq = self.commit_ref(mote, result_ref, nd);
        (result_ref, seq)
    }

    /// Commit Mote `mote` (tag `nd`) referencing an already-known ref (no put).
    /// Useful for shared-ref / already-absent scenarios. Returns the seq.
    pub fn commit_ref(&self, mote: u8, result_ref: ContentRef, nd: NdClass) -> u64 {
        self.journal
            .append(JournalEntry::Committed {
                mote_id: mid(mote),
                idempotency_key: [mote; 32],
                seq: 0, // ignored; journal assigns
                nondeterminism: nd,
                result_ref,
                parents: SmallVec::new(),
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                mote_def_hash: dh(mote),
            })
            .unwrap()
            .seq()
    }

    pub fn repudiate(&self, mote: u8, target_seq: u64) {
        self.journal
            .append(JournalEntry::Repudiated {
                target_mote_id: mid(mote),
                idempotency_key: [0xee ^ mote; 32],
                seq: 0,
                target_committed_seq: target_seq,
                reason_class: RepudiationReason::OperatorAction,
                repudiator_id: 1,
            })
            .unwrap();
    }

    pub fn snapshot(&self) -> Snapshot {
        Projection::from_journal(&self.journal).unwrap().snapshot()
    }
}
