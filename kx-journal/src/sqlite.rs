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
//!     kind            INTEGER  NOT NULL,      -- u8 (Proposed=0, Committed=1, Repudiated=2, Failed=3)
//!     mote_id         BLOB     NOT NULL,      -- 32 bytes
//!     idempotency_key BLOB     NOT NULL,      -- 32 bytes
//!     nondeterminism  INTEGER  NOT NULL DEFAULT 0,
//!     mote_def_hash   BLOB,                   -- 32 bytes (Committed only; non-canonical metadata)
//!     entry_bytes     BLOB     NOT NULL       -- the full canonical entry bytes (header + body)
//! );
//!
//! CREATE INDEX idx_entries_mote_id ON entries (mote_id);
//! CREATE UNIQUE INDEX idx_entries_dedupe ON entries (idempotency_key, kind) WHERE kind IN (1, 2);
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

use std::path::Path;

use kx_content::ContentRef;
use kx_mote::{MoteDefHash, MoteId};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::entry::{
    decode_entry_with_def_hash, encode_entry, repudiation_idempotency_key, JournalEntry,
    JOURNAL_SCHEMA_VERSION, KIND_COMMITTED, KIND_REPUDIATED, MAX_ENTRY_LEN,
};
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
                 ON entries (idempotency_key, kind) WHERE kind IN (1, 2);
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
    fn append(&self, mut entry: JournalEntry) -> Result<JournalEntry, JournalError> {
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

        let mut conn = self.conn.lock().expect("poisoned mutex");
        let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Dedupe-by-key for Committed + Repudiated only.
        let kind = entry.kind();
        if kind == KIND_COMMITTED || kind == KIND_REPUDIATED {
            let key = *entry.idempotency_key();
            let existing = txn
                .query_row(
                    "SELECT entry_bytes, mote_def_hash
                       FROM entries
                      WHERE idempotency_key = ?1 AND kind = ?2",
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
                txn.commit()?;
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
        let nd_byte = match &entry {
            JournalEntry::Proposed { nondeterminism, .. }
            | JournalEntry::Committed { nondeterminism, .. } => nondeterminism.as_u8(),
            _ => 0,
        } as i64;
        let def_hash_bytes: Option<Vec<u8>> = match &entry {
            JournalEntry::Committed { mote_def_hash, .. } => {
                Some(mote_def_hash.as_bytes().to_vec())
            }
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

        txn.commit()?;
        Ok(entry)
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

/// Inject an assigned `seq` into an entry just before encoding.
fn set_seq(entry: &mut JournalEntry, new_seq: u64) {
    match entry {
        JournalEntry::Proposed { seq, .. }
        | JournalEntry::Committed { seq, .. }
        | JournalEntry::Repudiated { seq, .. }
        | JournalEntry::Failed { seq, .. } => *seq = new_seq,
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
