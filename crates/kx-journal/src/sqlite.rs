//! `SqliteJournal` — the OSS [`Journal`] backend backed by SQLite (via `rusqlite`).
//!
//! ## Schema
//!
//! ```sql
//! CREATE TABLE metadata (
//!     key   TEXT  PRIMARY KEY,
//!     value BLOB  NOT NULL
//! );
//! -- One row: ('schema_version', LE u16 bytes of JOURNAL_SCHEMA_VERSION)
//!
//! CREATE TABLE entries (
//!     seq             INTEGER  PRIMARY KEY,   -- per-run monotonic u64
//!     kind            INTEGER  NOT NULL,      -- u8 (Proposed=0, Committed=1, Repudiated=2, Failed=3, EffectStaged=4, RunRegistered=5, RunVersionsResolved=6)
//!     mote_id         BLOB     NOT NULL,      -- 32 bytes
//!     idempotency_key BLOB     NOT NULL,      -- 32 bytes
//!     nondeterminism  INTEGER  NOT NULL DEFAULT 0,
//!     mote_def_hash   BLOB,                   -- 32 bytes (Committed only; non-canonical metadata)
//!     entry_bytes     BLOB     NOT NULL       -- the full canonical entry bytes (header + body)
//! );
//!
//! CREATE INDEX idx_entries_mote_id ON entries (mote_id);
//! CREATE UNIQUE INDEX idx_entries_dedupe ON entries (idempotency_key, kind) WHERE kind IN (1, 2, 4);
//! -- RunRegistered (kind 5) + RunVersionsResolved (kind 6) are NOT in the dedupe set: run-metadata facts, append-only (one run registers once; resolved-versions are append-many).
//! CREATE INDEX idx_entries_def_hash ON entries (mote_def_hash) WHERE kind = 1;
//! ```
//!
//! The denormalized columns (`kind`, `mote_id`, `idempotency_key`, `nondeterminism`,
//! `mote_def_hash`) exist for query speed; `entry_bytes` is the canonical authoritative
//! form per `journal-entry.md`. A reader could reconstruct the columns from
//! `entry_bytes` if needed.
//!
//! ## Atomicity
//!
//! Every append runs in a `BEGIN IMMEDIATE` transaction: the dedupe-check SELECT, the
//! `seq` assignment, and the INSERT all see one consistent state and either all land
//! or none do. A panic inside the closure passed to `with_txn` drops the rusqlite
//! `Transaction`, which rolls back — verified by the atomicity test sweep.
//!
//! ## Seq monotonicity
//!
//! `seq INTEGER PRIMARY KEY` (without `AUTOINCREMENT`) means SQLite reuses freed
//! rowids on rollback, so a failed/aborted txn does not burn a `seq` value — matching
//! `journal-txn.md` §6 ("Assignment is final on successful commit only").

use std::path::{Path, PathBuf};

use kx_content::ContentRef;
use kx_mote::{MoteDefHash, MoteId};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::entry::{
    decode_entry_with_def_hash, encode_entry, repudiation_idempotency_key, JournalEntry,
    JOURNAL_SCHEMA_VERSION, KIND_COMMITTED, KIND_EFFECT_STAGED, KIND_REPUDIATED, MAX_ENTRY_LEN,
};
use crate::migration::{migrate_entry, MIN_SUPPORTED_SCHEMA_VERSION};
use crate::{Journal, JournalError};

const METADATA_SCHEMA_VERSION_KEY: &str = "schema_version";

/// SQLite-backed [`Journal`] for the OSS local-node runtime.
///
/// One journal database == one workflow run (per `journal-txn.md` §6). Open the same
/// path on restart to resume; open a fresh path to start a new run.
///
/// # Examples
///
/// In-memory mode (used by tests + downstream fixtures):
///
/// ```
/// use kx_journal::{Journal, SqliteJournal};
///
/// let j = SqliteJournal::open_in_memory().unwrap();
/// assert_eq!(j.count_entries().unwrap(), 0);
/// assert_eq!(j.current_seq().unwrap(), 0);
/// ```
///
/// On-disk mode (the production shape):
///
/// ```
/// use kx_journal::{Journal, SqliteJournal};
/// use tempfile::TempDir;
///
/// let tmp = TempDir::new().unwrap();
/// let path = tmp.path().join("run-001.kxjournal");
/// let j = SqliteJournal::open(&path).unwrap();
/// assert_eq!(j.current_seq().unwrap(), 0);
///
/// // Re-opening the same path resumes the same run (the schema_version
/// // check happens here; mismatch refuses loudly).
/// drop(j);
/// let resumed = SqliteJournal::open(&path).unwrap();
/// assert_eq!(resumed.current_seq().unwrap(), 0);
/// ```
pub struct SqliteJournal {
    conn: std::sync::Mutex<Connection>,
}

impl std::fmt::Debug for SqliteJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteJournal").finish_non_exhaustive()
    }
}

impl SqliteJournal {
    /// Open or create a journal at the given filesystem path. Creates the schema on
    /// first use; on subsequent opens, verifies `schema_version` matches this binary
    /// and refuses loudly otherwise (`journal-entry.md` §10).
    ///
    /// The journal opens with `synchronous = FULL` and `journal_mode = WAL` for
    /// durability + concurrent reads with one writer.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::configure(&conn)?;
        Self::initialize(&conn)?;
        Self::verify_schema_version(&conn)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    /// Open an in-memory journal. Useful for tests that want a real SQLite backend
    /// without touching the filesystem; the journal vanishes when dropped.
    pub fn open_in_memory() -> Result<Self, JournalError> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        Self::initialize(&conn)?;
        Self::verify_schema_version(&conn)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    /// PRAGMA configuration applied at open time.
    fn configure(conn: &Connection) -> Result<(), JournalError> {
        // synchronous=FULL: fsync on every commit. Strictest durability.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;",
        )?;
        Ok(())
    }

    /// Create the schema on first open; idempotent (CREATE IF NOT EXISTS).
    fn initialize(conn: &Connection) -> Result<(), JournalError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                 key   TEXT PRIMARY KEY,
                 value BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS entries (
                 seq             INTEGER PRIMARY KEY,
                 kind            INTEGER NOT NULL,
                 mote_id         BLOB    NOT NULL,
                 idempotency_key BLOB    NOT NULL,
                 nondeterminism  INTEGER NOT NULL DEFAULT 0,
                 mote_def_hash   BLOB,
                 entry_bytes     BLOB    NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_entries_mote_id ON entries (mote_id);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_dedupe
                 ON entries (idempotency_key, kind) WHERE kind IN (1, 2, 4);
             CREATE INDEX IF NOT EXISTS idx_entries_def_hash
                 ON entries (mote_def_hash) WHERE kind = 1;",
        )?;

        // Insert the schema_version row if it isn't there yet.
        let version_bytes: [u8; 2] = JOURNAL_SCHEMA_VERSION.to_le_bytes();
        conn.execute(
            "INSERT OR IGNORE INTO metadata (key, value) VALUES (?1, ?2)",
            params![METADATA_SCHEMA_VERSION_KEY, &version_bytes[..]],
        )?;
        Ok(())
    }

    /// Read the journal-file `schema_version` and refuse loudly on mismatch.
    fn verify_schema_version(conn: &Connection) -> Result<(), JournalError> {
        let stored: Vec<u8> = conn.query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![METADATA_SCHEMA_VERSION_KEY],
            |r| r.get(0),
        )?;
        if stored.len() != 2 {
            return Err(JournalError::Invariant(
                "metadata.schema_version is not 2 bytes",
            ));
        }
        let found = u16::from_le_bytes([stored[0], stored[1]]);
        if found != JOURNAL_SCHEMA_VERSION {
            return Err(JournalError::SchemaVersionMismatch {
                expected: JOURNAL_SCHEMA_VERSION,
                found,
            });
        }
        Ok(())
    }
}

impl Journal for SqliteJournal {
    fn append(&self, entry: JournalEntry) -> Result<JournalEntry, JournalError> {
        let mut conn = self.conn.lock().expect("poisoned mutex");
        let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let durable = append_one(&txn, entry)?;
        txn.commit()?;
        Ok(durable)
    }

    fn append_batch(&self, entries: Vec<JournalEntry>) -> Result<Vec<JournalEntry>, JournalError> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn.lock().expect("poisoned mutex");
        // One BEGIN IMMEDIATE transaction for the whole batch (group commit):
        // every entry's dedupe SELECT + `seq` assignment + INSERT see the prior
        // in-batch inserts (so within-batch duplicates dedupe and seqs stay
        // contiguous), and the first error short-circuits before `commit`, so the
        // `Transaction`'s Drop rolls the entire batch back — all-or-nothing.
        let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut durable = Vec::with_capacity(entries.len());
        for entry in entries {
            durable.push(append_one(&txn, entry)?);
        }
        txn.commit()?;
        Ok(durable)
    }

    fn read_committed(&self, mote_id: &MoteId) -> Result<Option<JournalEntry>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let row = conn
            .query_row(
                "SELECT entry_bytes, mote_def_hash
                   FROM entries
                  WHERE mote_id = ?1 AND kind = ?2
                  LIMIT 1",
                params![mote_id.as_bytes().as_slice(), KIND_COMMITTED as i64],
                |r| {
                    let bytes: Vec<u8> = r.get(0)?;
                    let dh: Option<Vec<u8>> = r.get(1)?;
                    Ok((bytes, dh))
                },
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((bytes, dh)) => {
                let def_hash = vec_to_mote_def_hash(dh);
                Ok(Some(decode_entry_with_def_hash(&bytes, def_hash)?))
            }
        }
    }

    fn read_entries_by_seq(
        &self,
        range: std::ops::Range<u64>,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let mut stmt = conn.prepare(
            "SELECT entry_bytes, mote_def_hash
               FROM entries
              WHERE seq >= ?1 AND seq < ?2
              ORDER BY seq ASC",
        )?;
        let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = stmt
            .query_map(params![range.start as i64, range.end as i64], |r| {
                let bytes: Vec<u8> = r.get(0)?;
                let dh: Option<Vec<u8>> = r.get(1)?;
                Ok((bytes, dh))
            })?
            .collect::<Result<_, _>>()?;
        // Eagerly collect to release the SQLite statement lock before returning.
        let decoded: Vec<JournalEntry> = rows
            .into_iter()
            .map(|(b, dh)| {
                decode_entry_with_def_hash(&b, vec_to_mote_def_hash(dh))
                    .expect("on-disk entry decodes")
            })
            .collect();
        Ok(Box::new(decoded.into_iter()))
    }

    fn list_committed_refs(
        &self,
    ) -> Result<Box<dyn Iterator<Item = ContentRef> + '_>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let mut stmt =
            conn.prepare("SELECT entry_bytes FROM entries WHERE kind = ?1 ORDER BY seq ASC")?;
        let entries: Vec<JournalEntry> = stmt
            .query_map(params![KIND_COMMITTED as i64], |r| {
                let bytes: Vec<u8> = r.get(0)?;
                Ok(bytes)
            })?
            .filter_map(Result::ok)
            .map(|bytes| {
                // mote_def_hash is irrelevant for ref enumeration; pass zeros.
                decode_entry_with_def_hash(&bytes, MoteDefHash::from_bytes([0u8; 32]))
                    .expect("on-disk entry decodes")
            })
            .collect();
        let refs: Vec<ContentRef> = entries
            .into_iter()
            .filter_map(|e| match e {
                JournalEntry::Committed { result_ref, .. } => Some(result_ref),
                _ => None,
            })
            .collect();
        Ok(Box::new(refs.into_iter()))
    }

    fn list_committed_by_mote_def_hash(
        &self,
        h: &MoteDefHash,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let mut stmt = conn.prepare(
            "SELECT entry_bytes, mote_def_hash
               FROM entries
              WHERE kind = ?1 AND mote_def_hash = ?2
              ORDER BY seq ASC",
        )?;
        let dh_bytes = h.as_bytes().to_vec();
        let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = stmt
            .query_map(params![KIND_COMMITTED as i64, dh_bytes], |r| {
                let bytes: Vec<u8> = r.get(0)?;
                let dh: Option<Vec<u8>> = r.get(1)?;
                Ok((bytes, dh))
            })?
            .collect::<Result<_, _>>()?;
        let decoded: Vec<JournalEntry> = rows
            .into_iter()
            .map(|(b, dh)| {
                decode_entry_with_def_hash(&b, vec_to_mote_def_hash(dh))
                    .expect("on-disk entry decodes")
            })
            .collect();
        Ok(Box::new(decoded.into_iter()))
    }

    fn current_seq(&self) -> Result<u64, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let v: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM entries", [], |r| {
            r.get(0)
        })?;
        #[allow(clippy::cast_sign_loss)]
        Ok(v as u64)
    }

    fn count_entries(&self) -> Result<u64, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let v: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        #[allow(clippy::cast_sign_loss)]
        Ok(v as u64)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Append one entry **within an already-open transaction** — the shared core of
/// [`SqliteJournal::append`] and [`SqliteJournal::append_batch`]. Does NOT begin
/// or commit the transaction (the caller owns those), so the same logic is atomic
/// for a single append and for a whole batch.
///
/// Mirrors the per-entry contract: derive the `Repudiated` key (D15), dedupe by
/// `(idempotency_key, kind)` for the deduped kinds (returning the pre-existing
/// durable entry without consuming a `seq`), else assign `MAX(seq)+1`, size-check,
/// and INSERT. Because the dedupe SELECT and the `MAX(seq)` query run inside the
/// caller's transaction, they see earlier inserts from the same batch.
fn append_one(
    txn: &rusqlite::Transaction<'_>,
    mut entry: JournalEntry,
) -> Result<JournalEntry, JournalError> {
    // For Repudiated entries, derive the idempotency_key from the target before
    // we touch the database — this is the dedupe-by-target rule from D15.
    if let JournalEntry::Repudiated {
        target_mote_id,
        target_committed_seq,
        ref mut idempotency_key,
        ..
    } = entry
    {
        *idempotency_key = repudiation_idempotency_key(&target_mote_id, target_committed_seq);
    }

    // v2 (D38 §2b): dedupe-by-key index expands from {1, 2} to {1, 2, 4}.
    // EffectStaged participates; Failed is intentionally out.
    let kind = entry.kind();
    if kind == KIND_COMMITTED || kind == KIND_REPUDIATED || kind == KIND_EFFECT_STAGED {
        let key = *entry.idempotency_key();
        // The dedupe lookup MUST use the partial `idx_entries_dedupe` index
        // (`... WHERE kind IN (1, 2, 4)`). SQLite only applies a partial index when
        // the query's WHERE clause provably implies the index's predicate — and a
        // *bound* `kind = ?2` parameter cannot be proven to imply `kind IN (1,2,4)`
        // at plan time, so without the explicit `AND kind IN (1, 2, 4)` below SQLite
        // falls back to a FULL TABLE SCAN, making every append O(n) and the journal
        // O(n²) over its life (IMP-4 / D116 — measured + EXPLAIN-confirmed). The added
        // predicate is always TRUE here (the enclosing guard restricts `kind` to
        // exactly {1, 2, 4}), so it changes nothing semantically — it only lets the
        // planner use the index, restoring O(log n) per append.
        let existing = txn
            .query_row(
                "SELECT entry_bytes, mote_def_hash
                   FROM entries
                  WHERE idempotency_key = ?1 AND kind = ?2
                    AND kind IN (1, 2, 4)",
                params![&key[..], kind as i64],
                |r| {
                    let bytes: Vec<u8> = r.get(0)?;
                    let dh: Option<Vec<u8>> = r.get(1)?;
                    Ok((bytes, dh))
                },
            )
            .optional()?;
        if let Some((bytes, dh)) = existing {
            let def_hash = vec_to_mote_def_hash(dh);
            return decode_entry_with_def_hash(&bytes, def_hash).map_err(Into::into);
        }
    }

    // Assign next monotonic seq.
    let next_seq: i64 =
        txn.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM entries", [], |r| {
            r.get(0)
        })?;
    #[allow(clippy::cast_sign_loss)]
    let next_seq_u64 = next_seq as u64;

    // Inject the assigned seq into the entry before encoding.
    set_seq(&mut entry, next_seq_u64);

    let bytes = encode_entry(&entry)?;
    if bytes.len() > MAX_ENTRY_LEN {
        return Err(JournalError::EntryTooLarge { got: bytes.len() });
    }

    let mote_id_owned = entry.mote_id();
    let mote_id_bytes: &[u8; 32] = mote_id_owned.as_bytes();
    let idem_key: [u8; 32] = *entry.idempotency_key();
    let nd_byte = i64::from(match &entry {
        JournalEntry::Proposed { nondeterminism, .. }
        | JournalEntry::Committed { nondeterminism, .. } => nondeterminism.as_u8(),
        _ => 0,
    });
    let def_hash_bytes: Option<Vec<u8>> = match &entry {
        JournalEntry::Committed { mote_def_hash, .. } => Some(mote_def_hash.as_bytes().to_vec()),
        _ => None,
    };

    txn.execute(
        "INSERT INTO entries
             (seq, kind, mote_id, idempotency_key, nondeterminism, mote_def_hash, entry_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            next_seq,
            kind as i64,
            &mote_id_bytes[..],
            &idem_key[..],
            nd_byte,
            def_hash_bytes,
            &bytes[..],
        ],
    )?;

    Ok(entry)
}

/// Inject an assigned `seq` into an entry just before encoding.
fn set_seq(entry: &mut JournalEntry, new_seq: u64) {
    match entry {
        JournalEntry::Proposed { seq, .. }
        | JournalEntry::Committed { seq, .. }
        | JournalEntry::Repudiated { seq, .. }
        | JournalEntry::Failed { seq, .. }
        | JournalEntry::EffectStaged { seq, .. }
        | JournalEntry::RunRegistered { seq, .. }
        | JournalEntry::RunVersionsResolved { seq, .. }
        | JournalEntry::DigestSealed { seq, .. }
        | JournalEntry::ReplanRound { seq, .. }
        | JournalEntry::ReactRound { seq, .. } => *seq = new_seq,
    }
}

/// Convert an optional column `BLOB` to a `MoteDefHash`, with all-zeros for absent
/// (used for non-Committed entries, where `mote_def_hash` is meaningless).
fn vec_to_mote_def_hash(v: Option<Vec<u8>>) -> MoteDefHash {
    match v {
        None => MoteDefHash::from_bytes([0u8; 32]),
        Some(bytes) if bytes.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            MoteDefHash::from_bytes(out)
        }
        Some(_) => MoteDefHash::from_bytes([0u8; 32]),
    }
}

/// Read the on-disk `schema_version` (LE u16) from the `metadata` table without
/// comparing it to this binary's — the migration entry points need the value, not
/// a refusal. (The strict `SqliteJournal::verify_schema_version` is unchanged.)
fn read_schema_version(conn: &Connection) -> Result<u16, JournalError> {
    let stored: Vec<u8> = conn.query_row(
        "SELECT value FROM metadata WHERE key = ?1",
        params![METADATA_SCHEMA_VERSION_KEY],
        |r| r.get(0),
    )?;
    if stored.len() != 2 {
        return Err(JournalError::Invariant(
            "metadata.schema_version is not 2 bytes",
        ));
    }
    Ok(u16::from_le_bytes([stored[0], stored[1]]))
}

// ===========================================================================
// Schema migration — read-only replay + offline rewrite (IMP-2, M2.x-E)
// ===========================================================================

/// A **read-only**, schema-version-aware view over a journal file, used to replay
/// or recover a run written by an *older* (still-supported) binary.
///
/// Unlike [`SqliteJournal::open`] — which refuses any file whose `schema_version`
/// is not exactly this binary's — [`ReplayJournal::open`] accepts any version in
/// `[MIN_SUPPORTED_SCHEMA_VERSION, JOURNAL_SCHEMA_VERSION]` and up-converts each
/// entry to the current in-memory shape on the fly (see [`crate::migrate_entry`]).
/// The on-disk **committed content is never modified** (the handle performs no
/// appends; writes are rejected at the Rust API by [`ReplayJournal::append`]).
///
/// This is the read half of the upgrade story: a current binary can fold/recover
/// an old journal. To obtain a *writable* current-version journal (to resume and
/// append new entries), use [`migrate_to`].
pub struct ReplayJournal {
    conn: std::sync::Mutex<Connection>,
    from_version: u16,
}

impl std::fmt::Debug for ReplayJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayJournal")
            .field("from_version", &self.from_version)
            .finish_non_exhaustive()
    }
}

impl ReplayJournal {
    /// Open a journal file for **read-only replay**, up-converting an older
    /// on-disk schema to the current in-memory shape.
    ///
    /// Refuses (`SchemaVersionMismatch`) a file newer than this binary's
    /// [`JOURNAL_SCHEMA_VERSION`] or older than [`MIN_SUPPORTED_SCHEMA_VERSION`] —
    /// the same loud-refusal contract as [`SqliteJournal::open`], just with a
    /// supported-range window instead of an exact match.
    ///
    /// Opens read-write at the SQLite layer (so a crashed journal's WAL can be
    /// recovered) but immediately sets `PRAGMA query_only` so the engine rejects
    /// any data write — the source's committed content is never altered.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Logical read-only: allow WAL recovery on open, reject data writes.
        conn.execute_batch("PRAGMA query_only = ON;")?;
        let from_version = read_schema_version(&conn)?;
        if !(MIN_SUPPORTED_SCHEMA_VERSION..=JOURNAL_SCHEMA_VERSION).contains(&from_version) {
            return Err(JournalError::SchemaVersionMismatch {
                expected: JOURNAL_SCHEMA_VERSION,
                found: from_version,
            });
        }
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            from_version,
        })
    }

    /// The on-disk schema version this handle is replaying from.
    #[must_use]
    pub fn from_version(&self) -> u16 {
        self.from_version
    }
}

impl Journal for ReplayJournal {
    fn append(&self, _entry: JournalEntry) -> Result<JournalEntry, JournalError> {
        Err(JournalError::Invariant(
            "ReplayJournal is read-only; use migrate_to to obtain a writable current-version journal",
        ))
    }

    fn append_batch(&self, _entries: Vec<JournalEntry>) -> Result<Vec<JournalEntry>, JournalError> {
        Err(JournalError::Invariant(
            "ReplayJournal is read-only; use migrate_to to obtain a writable current-version journal",
        ))
    }

    fn read_committed(&self, mote_id: &MoteId) -> Result<Option<JournalEntry>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let row: Option<(Vec<u8>, Option<Vec<u8>>)> = conn
            .query_row(
                "SELECT entry_bytes, mote_def_hash
                   FROM entries
                  WHERE mote_id = ?1 AND kind = ?2",
                params![mote_id.as_bytes().to_vec(), KIND_COMMITTED as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((bytes, dh)) => Ok(Some(migrate_entry(
                &bytes,
                self.from_version,
                vec_to_mote_def_hash(dh),
            )?)),
        }
    }

    fn read_entries_by_seq(
        &self,
        range: std::ops::Range<u64>,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let mut stmt = conn.prepare(
            "SELECT entry_bytes, mote_def_hash
               FROM entries
              WHERE seq >= ?1 AND seq < ?2
              ORDER BY seq ASC",
        )?;
        let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = stmt
            .query_map(params![range.start as i64, range.end as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        // Version-aware decode; a migration failure surfaces (fail-closed) rather
        // than panicking the way the current-version `SqliteJournal` read path does.
        let decoded: Vec<JournalEntry> = rows
            .into_iter()
            .map(|(b, dh)| migrate_entry(&b, self.from_version, vec_to_mote_def_hash(dh)))
            .collect::<Result<_, _>>()?;
        Ok(Box::new(decoded.into_iter()))
    }

    fn list_committed_refs(
        &self,
    ) -> Result<Box<dyn Iterator<Item = ContentRef> + '_>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let mut stmt =
            conn.prepare("SELECT entry_bytes FROM entries WHERE kind = ?1 ORDER BY seq ASC")?;
        let rows: Vec<Vec<u8>> = stmt
            .query_map(params![KIND_COMMITTED as i64], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut refs: Vec<ContentRef> = Vec::with_capacity(rows.len());
        for bytes in rows {
            // mote_def_hash is irrelevant for ref enumeration; pass zeros.
            if let JournalEntry::Committed { result_ref, .. } = migrate_entry(
                &bytes,
                self.from_version,
                MoteDefHash::from_bytes([0u8; 32]),
            )? {
                refs.push(result_ref);
            }
        }
        Ok(Box::new(refs.into_iter()))
    }

    fn list_committed_by_mote_def_hash(
        &self,
        h: &MoteDefHash,
    ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let mut stmt = conn.prepare(
            "SELECT entry_bytes, mote_def_hash
               FROM entries
              WHERE kind = ?1 AND mote_def_hash = ?2
              ORDER BY seq ASC",
        )?;
        let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = stmt
            .query_map(params![KIND_COMMITTED as i64, h.as_bytes().to_vec()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        let decoded: Vec<JournalEntry> = rows
            .into_iter()
            .map(|(b, dh)| migrate_entry(&b, self.from_version, vec_to_mote_def_hash(dh)))
            .collect::<Result<_, _>>()?;
        Ok(Box::new(decoded.into_iter()))
    }

    fn current_seq(&self) -> Result<u64, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let v: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM entries", [], |r| {
            r.get(0)
        })?;
        Ok(v as u64)
    }

    fn count_entries(&self) -> Result<u64, JournalError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let v: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        Ok(v as u64)
    }
}

/// Outcome of an offline [`migrate_to`] rewrite — a small audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    /// The source journal's on-disk schema version.
    pub from_version: u16,
    /// The schema version the destination was written at (always
    /// [`JOURNAL_SCHEMA_VERSION`]).
    pub to_version: u16,
    /// Total entries copied into the destination.
    pub entries_migrated: u64,
    /// Of those, how many required a non-identity up-conversion (e.g. a v5
    /// capability record gaining its default `idempotency_class`).
    pub entries_upconverted: u64,
}

/// Offline-migrate the journal at `src` (any supported older version) into a
/// fresh **current-version** journal at `dst`, preserving every `seq`. After this
/// the destination opens with the strict [`SqliteJournal::open`] and can be
/// resumed and appended to.
///
/// Crash-safe: the destination is built at a temporary sibling of `dst`, fully
/// committed and WAL-checkpointed, then atomically `rename`d into place — a crash
/// mid-rewrite leaves `src` untouched and `dst` absent (retry is safe). Idempotent:
/// a `src` already at the current version is rebuilt into a logically-equivalent
/// current journal (`entries_upconverted == 0`).
///
/// The product identity (`kx-runtime`'s committed-facts digest) is invariant
/// across this rewrite. The migration refuses loudly
/// (`Invariant("source seq not preserved")`) rather than silently remap a `seq` if
/// the source is non-contiguous (corruption) — `DigestSealed::through_seq` and
/// committed `seq` references depend on exact preservation.
///
/// `src` must be quiesced (no concurrent writer).
pub fn migrate_to(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
) -> Result<MigrationReport, JournalError> {
    let replay = ReplayJournal::open(src.as_ref())?;
    let from_version = replay.from_version;
    let dst = dst.as_ref();
    let tmp = temp_sibling(dst);

    // Clear any stale temp from a previously-aborted migration.
    remove_sqlite_family(&tmp);

    let counts = migrate_into_temp(&replay, from_version, &tmp);
    match counts {
        Ok((entries_migrated, entries_upconverted)) => {
            std::fs::rename(&tmp, dst)?;
            Ok(MigrationReport {
                from_version,
                to_version: JOURNAL_SCHEMA_VERSION,
                entries_migrated,
                entries_upconverted,
            })
        }
        Err(e) => {
            remove_sqlite_family(&tmp);
            Err(e)
        }
    }
}

/// Build the current-version journal at `tmp` from `replay`'s up-converted
/// entries, returning `(entries_migrated, entries_upconverted)`. Single
/// transaction (group commit) + WAL checkpoint + clean close.
fn migrate_into_temp(
    replay: &ReplayJournal,
    from_version: u16,
    tmp: &Path,
) -> Result<(u64, u64), JournalError> {
    let out = SqliteJournal::open(tmp)?; // initialize() stamps the CURRENT version
    let mut conn = out.conn.lock().expect("poisoned mutex");
    let mut entries_migrated = 0u64;
    let mut entries_upconverted = 0u64;
    {
        let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head = replay.current_seq()?;
        for entry in replay.read_entries_by_seq(0..head + 1)? {
            let src_seq = entry.seq();
            // Only the v5 arm TRANSFORMS bytes (appends the idempotency_class
            // byte to a capability-present record); v6/v7 → current are pure
            // pass-throughs (additive kinds only), so nothing "up-converts".
            let upconverted = from_version < 6
                && matches!(
                    &entry,
                    JournalEntry::RunVersionsResolved {
                        capability: Some(_),
                        ..
                    }
                );
            let durable = append_one(&txn, entry)?;
            if durable.seq() != src_seq {
                return Err(JournalError::Invariant(
                    "migrate_to: source seq not preserved (non-contiguous journal)",
                ));
            }
            entries_migrated += 1;
            if upconverted {
                entries_upconverted += 1;
            }
        }
        txn.commit()?;
    }
    // Merge the WAL into the main db + truncate so the renamed file is a single,
    // self-contained, cleanly-closeable database.
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(conn);
    drop(out);
    Ok((entries_migrated, entries_upconverted))
}

/// A temp path in the SAME directory as `dst` (so the final `rename` is atomic on
/// one filesystem), derived from `dst`'s file name.
fn temp_sibling(dst: &Path) -> PathBuf {
    let mut file_name = dst
        .file_name()
        .map_or_else(|| std::ffi::OsString::from("journal"), ToOwned::to_owned);
    file_name.push(".kx-migrate.tmp");
    dst.with_file_name(file_name)
}

/// Best-effort removal of a SQLite database file plus its `-wal`/`-shm` siblings.
fn remove_sqlite_family(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm"] {
        let mut sibling = path.as_os_str().to_owned();
        sibling.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sibling));
    }
}
