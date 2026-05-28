//! [`ReadModel`] — a worker's local, incrementally-folded view of committed
//! results, built from `ReadEntries` deltas.
//!
//! This is the scalable read path (D55): a worker resolves a committed Mote's
//! `result_ref` from its own folded state instead of round-tripping the
//! coordinator per lookup. P2.4 keeps it minimal — a `MoteId -> ContentRef` map
//! over `Committed` entries; the forward seam (P3) is to fold the full journal
//! log (Proposed / Repudiated / ...) into a faithful `kx-projection` once those
//! entry kinds ride the wire (the `JournalEntry` oneof is reserved for them).

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_mote::MoteId;
use kx_proto::proto;

/// A worker-local index of committed results, advanced by a journal cursor.
#[derive(Debug, Default)]
pub(crate) struct ReadModel {
    committed: BTreeMap<MoteId, ContentRef>,
    cursor: u64,
}

impl ReadModel {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The cursor to resume a `ReadEntries` poll from.
    pub(crate) fn cursor(&self) -> u64 {
        self.cursor
    }

    /// The committed result_ref of `mote_id`, if this model has folded its commit.
    pub(crate) fn result_ref_of(&self, mote_id: &MoteId) -> Option<ContentRef> {
        self.committed.get(mote_id).copied()
    }

    /// Fold a `ReadEntries` page: record each committed (mote_id -> result_ref) and
    /// advance the cursor. Malformed entries (wrong-length hashes) are skipped — the
    /// coordinator only emits well-formed committed entries, so this is a guard.
    pub(crate) fn fold(&mut self, entries: Vec<proto::JournalEntry>, next_seq: u64) {
        for entry in entries {
            let Some(proto::journal_entry::Kind::Committed(c)) = entry.kind else {
                continue;
            };
            if let (Ok(mote_id), Ok(result_ref)) = (to_array(&c.mote_id), to_array(&c.result_ref)) {
                self.committed.insert(
                    MoteId::from_bytes(mote_id),
                    ContentRef::from_bytes(result_ref),
                );
            }
        }
        self.cursor = next_seq;
    }
}

fn to_array(bytes: &[u8]) -> Result<[u8; 32], ()> {
    bytes.try_into().map_err(|_| ())
}
