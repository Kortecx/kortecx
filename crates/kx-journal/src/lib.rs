#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// TODO(workspace.lints cleanup): the encoder/decoder in `entry.rs` uses
// `.try_into().expect("N bytes")` on slices guarded by explicit-length
// checks earlier in the function (the entry-format spec pins exact byte
// budgets per kind). The lock-acquisition paths in `in_memory.rs` and
// `sqlite.rs` use the canonical `.read().expect("poisoned lock")` /
// `.write().expect("poisoned lock")` Rust idiom — poisoned locks
// indicate a panic occurred while another thread held the lock, the
// correct response is to propagate. A follow-up cleanup PR may migrate
// to typed errors; until then, the documented `expect(...)` messages
// are the audit trail.
#![allow(clippy::expect_used)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::too_many_lines,
    clippy::match_same_arms,
    clippy::range_plus_one
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! # kx-journal — the spine
//!
//! The append-only log of committed facts. **The synchronization substrate.** Scheduler,
//! executor, and recovery all coordinate through facts here; nothing inter-Mote travels
//! outside this log.
//!
//! ## Contracts (the strictest gate in P1)
//!
//! - **Append-only.** Entries are never modified in place. Repudiation is recorded as a
//!   new entry referencing the original.
//! - **Atomic per-entry txn.** One [`JournalEntry`] = one atomic write. All-or-nothing
//!   under panic, OS crash, or backend failure. Verified by the atomicity test sweep.
//! - **Dedupe-by-key for `Committed` and `Repudiated`.** Two appends with the same
//!   `idempotency_key` + kind yield exactly one durable fact; the second call returns
//!   the existing entry. This is what makes recovery sound — two workers racing on a
//!   re-scheduled Mote both produce identical keys; the first commit wins, the second
//!   is a no-op.
//! - **No dedupe for `Proposed` or `Failed`.** Many `Proposed` per identity is expected
//!   (re-scheduling, speculation); many `Failed` per identity is the retry path.
//! - **Per-run monotonic `seq`.** A u64 counter assigned at commit time, strictly
//!   increasing across all kinds within one workflow run. Survives restart of the same
//!   run; a fresh run starts a new sequence.
//! - **Single-writer-per-run.** Per `journal-txn.md` §7 (D13): the coordinator is the
//!   sole journal writer. For P1 single-process this is structural (one [`Journal`]
//!   handle per run); P2.2's `kx-coordinator` adds the loud-rejection enforcement when
//!   the worker/coordinator split lands.
//! - **No payloads inline.** The journal carries only `ContentRef`s (32-byte hashes);
//!   payload bytes live in [`kx_content::ContentStore`]. Anti-pattern #1 of
//!   `journal-entry.md` §11.
//! - **Backend-agnostic trait.** [`Journal`] does not name SQLite or any in-process
//!   type in its signature. The OSS impl is [`SqliteJournal`]; the cloud impl
//!   (replicated journal, P5.5) lands behind the same trait without redesign.
//!
//! ## What lives here
//!
//! - [`JournalEntry`] — the kind union (Proposed / Committed / Repudiated / Failed /
//!   EffectStaged / RunRegistered) with hand-rolled canonical byte encoding matching
//!   `journal-entry.md` §3-7 byte-for-byte (no bincode varint surprises; the `parents`
//!   length is u16 per spec).
//! - [`Journal`] — the backend-agnostic trait.
//! - [`SqliteJournal`] — the OSS local backend using `rusqlite` with `BEGIN IMMEDIATE`
//!   txns, a single `entries` table with a partial unique index for dedupe-by-key on
//!   `(idempotency_key, kind IN {Committed, Repudiated})`, and a `metadata` table
//!   pinning `schema_version`.
//! - [`InMemoryJournal`] — an in-memory backend for trait-seam proof + downstream
//!   test fixtures.
//! - [`encode_entry`], [`decode_entry`], [`repudiation_idempotency_key`] — encoding
//!   primitives + the Repudiated dedupe-key derivation per D15.
//!
//! ## What does NOT live here
//!
//! - Content payloads — [`kx_content`]. The journal carries `ContentRef`s only.
//! - Projection logic (fold log → graph) — `kx-projection` (P1.5).
//! - The scheduler / executor — P1.9 / P1.10.
//! - gRPC and the coordinator/worker split — P2.

pub use crate::entry::{
    decode_entry, decode_entry_with_def_hash, encode_entry, is_pre_commit_crash,
    repudiation_idempotency_key, run_root_id, DecodeError, EncodeError, FailureReason,
    JournalEntry, ParentEntry, RepudiationReason, HEADER_LEN, INSTANCE_ID_LEN,
    JOURNAL_SCHEMA_VERSION, KIND_COMMITTED, KIND_EFFECT_STAGED, KIND_FAILED, KIND_PROPOSED,
    KIND_REPUDIATED, KIND_RUN_REGISTERED, MAX_ENTRY_LEN, MAX_PARENTS,
};
pub use crate::in_memory::InMemoryJournal;
pub use crate::sqlite::SqliteJournal;

mod entry;
mod in_memory;
mod sqlite;

use std::ops::Range;

use kx_content::ContentRef;
use kx_mote::{MoteDefHash, MoteId};

// ---------------------------------------------------------------------------
// JournalError
// ---------------------------------------------------------------------------

/// Errors raised by [`Journal`] operations.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// The journal file's `schema_version` does not match this binary's. Refused
    /// loudly per `journal-entry.md` §10 — the reader does not attempt to decode
    /// entries with a mismatched schema.
    #[error("schema version mismatch: file has {found}, this binary expects {expected}")]
    SchemaVersionMismatch {
        /// Version the binary supports.
        expected: u16,
        /// Version found on disk.
        found: u16,
    },

    /// A `Committed` entry's encoded form exceeds the absolute per-entry size cap
    /// (`journal-entry.md` §8).
    #[error("entry exceeds size cap: {got} bytes > {} max", MAX_ENTRY_LEN)]
    EntryTooLarge {
        /// Encoded entry length.
        got: usize,
    },

    /// The encoder rejected the entry (e.g., Data-edge with non_cascade set, parent
    /// count over the max).
    #[error(transparent)]
    Encode(#[from] EncodeError),

    /// The decoder rejected a stored entry — indicates on-disk corruption or a
    /// pre-existing schema mismatch.
    #[error(transparent)]
    Decode(#[from] DecodeError),

    /// Underlying SQLite error from the local backend.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// Underlying I/O error (filesystem, etc.).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// An invariant of the journal was violated by an internal call (e.g., the
    /// dedupe path found two existing entries for one key, or `seq` regressed).
    /// Always indicates a bug; never an expected runtime error.
    #[error("journal invariant violated: {0}")]
    Invariant(&'static str),
}

// ---------------------------------------------------------------------------
// The Journal trait — backend-agnostic
// ---------------------------------------------------------------------------

/// The journal — the append-only log of committed facts and the synchronization
/// substrate for the runtime.
///
/// Implementors choose their persistence substrate (SQLite for OSS; replicated
/// log for cloud). The trait surface is intentionally narrow; the contract
/// is in `journal-txn.md` (P0.3) + `journal-entry.md` (P0.11).
///
/// # Examples
///
/// Append a Failed entry to an in-memory journal and read it back:
///
/// ```
/// use kx_journal::{FailureReason, InMemoryJournal, Journal, JournalEntry};
/// use kx_mote::MoteId;
///
/// let journal = InMemoryJournal::new();
/// let entry = JournalEntry::Failed {
///     mote_id: MoteId::from_bytes([1u8; 32]),
///     idempotency_key: [0xaa; 32],
///     seq: 0, // assigned by the journal at append time
///     reason_class: FailureReason::TimedOut,
///     reporter_id: 42,
/// };
/// let stored = journal.append(entry).unwrap();
/// assert_eq!(stored.seq(), 1); // first entry → seq 1
/// assert_eq!(journal.current_seq().unwrap(), 1);
/// ```
pub trait Journal {
    /// Append an entry. **The atomic write boundary.**
    ///
    /// On input, the entry's `seq` is ignored — the journal assigns the next per-run
    /// monotonic value at commit time. For `Repudiated` entries, the input's
    /// `idempotency_key` is also ignored — the journal derives it per
    /// [`repudiation_idempotency_key`].
    ///
    /// On return, the resulting [`JournalEntry`] is the durable fact:
    /// - **New write**: the entry as written, with the journal-assigned `seq` (and
    ///   derived `idempotency_key` for `Repudiated`).
    /// - **Dedupe hit** (Committed / Repudiated only): the pre-existing entry with
    ///   the original `seq`. The second call is a no-op.
    fn append(&self, entry: JournalEntry) -> Result<JournalEntry, JournalError>;

    /// Append many entries as **one atomic unit** (the group-commit primitive).
    ///
    /// Either every entry is durably appended — each assigned the next monotonic
    /// `seq`, in input order — or none is. Per-entry dedup-by-key applies exactly
    /// as in [`append`](Journal::append): a duplicate (by `idempotency_key`, for the
    /// deduped kinds `Committed`/`Repudiated`/`EffectStaged`) yields its pre-existing
    /// durable form and consumes no new `seq`; duplicates *within the same batch*
    /// dedupe against earlier entries in that batch. Returns the durable form of
    /// each input entry, in input order (so the result length always equals the
    /// input length). An empty batch is a no-op that returns an empty vector.
    ///
    /// Backends that can amortize the commit (e.g. one transaction over N entries)
    /// override this. **The default loops [`append`](Journal::append) and is
    /// therefore NOT atomic across entries** — a backend that needs batch atomicity
    /// MUST override it (both shipped backends do).
    fn append_batch(&self, entries: Vec<JournalEntry>) -> Result<Vec<JournalEntry>, JournalError> {
        entries.into_iter().map(|e| self.append(e)).collect()
    }

    /// Look up the (at most one) `Committed` entry for a Mote identity.
    ///
    /// Returns `None` if no Committed entry exists for the Mote yet. Used by the
    /// projection (P1.5) to determine if a Mote's result is durably available.
    fn read_committed(&self, mote_id: &MoteId) -> Result<Option<JournalEntry>, JournalError>;

    /// Read entries by sequence range (half-open: `[start, end)`).
    ///
    /// Used by the projection (P1.5) to fold the log into the graph view. Iteration
    /// order is by ascending `seq`. Entries from all kinds are returned.
    fn read_entries_by_seq(
        &self,
        range: Range<u64>,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError>;

    /// Enumerate every `result_ref` referenced by a `Committed` entry.
    ///
    /// Used by the orphan-GC walker (`content-store.md` §5) to determine which
    /// content-store objects are still alive. `Repudiated` entries' targets are
    /// **included** in the live set (the repudiated entry's `result_ref` may still
    /// be needed as cascade evidence).
    fn list_committed_refs(
        &self,
    ) -> Result<Box<dyn Iterator<Item = ContentRef> + '_>, JournalError>;

    /// Enumerate every `Committed` entry whose Mote's `mote_def_hash` matches `h`.
    ///
    /// Used by the operator-driven definition-level repudiation flow
    /// (`repudiation.md` §6, D22 — "every Mote sharing this `mote_def_hash` is now
    /// suspect"). The backend stores `mote_def_hash` as a denormalized column for
    /// query speed; it is NOT part of the canonical entry body bytes.
    fn list_committed_by_mote_def_hash(
        &self,
        h: &MoteDefHash,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError>;

    /// The largest `seq` written so far. Returns `0` for an empty journal.
    fn current_seq(&self) -> Result<u64, JournalError>;

    /// Total number of entries (all kinds). Diagnostic / fixture helper.
    fn count_entries(&self) -> Result<u64, JournalError>;
}
