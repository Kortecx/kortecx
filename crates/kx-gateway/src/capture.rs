//! The Morphic Data Engine — durable serve-path capture (campaign Batch 2).
//!
//! `kx-capture` is on-by-default but, before this, was wired ONLY into the
//! single-node `kx run` engine and held in memory; `kx serve` captured nothing.
//! This is the serve-path capture: a background poll-fold of the gateway's
//! read-only journal handle into a durable [`CaptureLedger`] (`capture.db`
//! under `--catalog-dir`), exposing the action exhaust through the
//! `ListCaptureRecords` RPC.
//!
//! ## Off the truth path (D40 — the load-bearing invariant)
//! The ledger is a journal-DERIVED projection: a REBUILDABLE CACHE, never
//! journaled, never a `MoteId` input, never gating execution. It folds the
//! read-only journal handle (the coordinator stays the sole writer) — so it adds
//! ZERO latency to the commit path and cannot perturb the canonical digest.
//! On open it reconciles against the journal: a `schema_version` mismatch or a
//! corrupt/torn sidecar ⇒ drop-and-rebuild from seq 0 (the journal is truth);
//! otherwise it resumes incrementally from the durable watermark.
//!
//! ## Join-key-only (the default `ActionsOnly` scope, made structural)
//! Each record holds a committed action's identity keys ONLY — `mote_id`,
//! `instance_id`, `result_ref` (the truth join key == the Mote's `result_ref`),
//! `nd_class`, `seq` — plus the ReAct `turn`/`branch` joined from the chain's
//! off-DAG `ReactRound` metadata. There are NO payload/reasoning/thinking
//! columns: the privacy-safe default is a schema fact, not a runtime strip. Each
//! row passes through `kx_capture::StepRecord::action` under an
//! `actions_only()` consent before insert, so the crate's scope discipline is on
//! the path.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use kx_capture::{CaptureConsent, StepRecord};
use kx_gateway_core::{CaptureRecordEntry, CaptureView, JournalReader};
use kx_journal::{JournalEntry, ReactBranch};
use kx_mote::NdClass;
use rusqlite::{params, Connection};

// `open` returns the HOST error (it has `Catalog`, raised before the service is
// built); the `CaptureView::list` trait method returns gateway-core's error.
use crate::error::GatewayError;
use kx_gateway_core::GatewayError as CoreError;

/// The capture-projection schema version. A bump (or any decode failure) makes
/// the load path drop-and-rebuild from the journal — capture is a cache, so
/// there is NEVER a migration, only a rebuild.
// v2: the durable `run_meta` instance-id row (the multi-tick stamping fix —
// an old v1 sidecar drops-and-rebuilds with correct stamping; capture is a cache).
const SCHEMA_VERSION: i64 = 2;

/// The durable schema (idempotent). `capture_records` is the action projection;
/// `react_turns` is the incremental turn→branch join source; `meta` holds the
/// `last_seq` fold watermark + the `schema_version`. NO payload columns exist
/// (the structural ActionsOnly guarantee).
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS capture_records (
    mote_id      BLOB PRIMARY KEY,
    instance_id  BLOB NOT NULL,
    result_ref   BLOB NOT NULL,
    nd_class     TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    react_turn   INTEGER,
    react_branch TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS capture_by_instance ON capture_records(instance_id, seq);
CREATE TABLE IF NOT EXISTS react_turns (
    turn_mote_id BLOB PRIMARY KEY,
    turn         INTEGER NOT NULL,
    branch       TEXT NOT NULL,
    seq          INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
-- The run instance id (single-node: one RunRegistered per journal) is DURABLE
-- across fold ticks, so an action committed in a LATER tick is still stamped
-- even though RunRegistered (seq=1) folded in an EARLIER tick. A blob, so it
-- cannot live in `meta` (INTEGER values).
CREATE TABLE IF NOT EXISTS run_meta (id INTEGER PRIMARY KEY CHECK (id = 0), instance_id BLOB NOT NULL);";

/// The closed `nd_class` wire vocabulary (the `ReactTurnSummary.branch` style:
/// a string, so a future class is additive on the wire).
fn nd_class_wire(nd: NdClass) -> &'static str {
    match nd {
        NdClass::Pure => "pure",
        NdClass::ReadOnlyNondet => "read_only_nondet",
        NdClass::WorldMutating => "world_mutating",
    }
}

/// The settled-branch wire vocabulary (mirrors `kx_gateway_core::react`).
fn branch_wire(branch: &ReactBranch) -> &'static str {
    match branch {
        ReactBranch::Answer => "answer",
        ReactBranch::Tool { .. } => "tool",
        ReactBranch::DeadLettered => "dead_lettered",
        ReactBranch::Pending => "pending",
    }
}

/// The durable capture-projection ledger over `capture.db`. A single mutex'd
/// connection serves both the periodic fold (write) and the `ListCaptureRecords`
/// read — capture is low-traffic (one page read; a periodic incremental fold),
/// so the mutex is never contended.
pub(crate) struct CaptureLedger {
    conn: Mutex<Connection>,
    /// The capture scope this serve runs under — hard-coded `ActionsOnly` (the
    /// `Full` scope stays code-gated for embedders; no CLI flag exposes it).
    consent: CaptureConsent,
}

impl CaptureLedger {
    /// Open (or create) `capture.db` under `dir`, reconcile against the journal,
    /// and return the ledger. A `schema_version` mismatch or a corrupt sidecar
    /// drops every table (the watermark resets to 0 ⇒ the next fold backfills
    /// from the journal). Idempotent on restart.
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("capture dir: {e}")))?;
        let db_path = dir.join("capture.db");
        // A CORRUPT/foreign file (not a SQLite database) makes even the pragma
        // fail with "file is not a database" — SQLite cannot drop-and-rebuild
        // it. Capture is a rebuildable cache (the journal is truth), so we
        // simply delete the unreadable file(s) and recreate (the next fold
        // backfills from seq 0). A valid-but-stale-schema DB is handled below.
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("capture.db-wal"));
            let _ = std::fs::remove_file(dir.join("capture.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("capture reopen: {e}")))?
        };
        // A torn/foreign sidecar (never initialized, version drift, or
        // unreadable) ⇒ drop everything and rebuild from the journal.
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS capture_records;
                 DROP TABLE IF EXISTS react_turns;
                 DROP TABLE IF EXISTS run_meta;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("capture rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("capture schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1), ('last_seq', 0)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("capture meta init: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            consent: CaptureConsent::actions_only(),
        })
    }

    /// Open `capture.db` and apply the pragmas — fails on a non-SQLite file
    /// (the corruption signal `open` recovers from by deleting + recreating).
    fn open_with_pragma(db_path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        Ok(conn)
    }

    fn read_schema_version(conn: &Connection) -> rusqlite::Result<Option<i64>> {
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }

    /// The durable fold watermark (the highest seq folded into the projection).
    fn watermark(conn: &Connection) -> i64 {
        conn.query_row("SELECT value FROM meta WHERE key = 'last_seq'", [], |r| {
            r.get(0)
        })
        .unwrap_or(0)
    }

    /// Incrementally fold `(watermark, head]` of `reader` into the projection:
    /// insert each `Committed` action row, upsert each `ReactRound` turn/branch
    /// (highest seq wins), and back-fill the react join onto already-captured
    /// rows (the settle commits AFTER the turn, so the order is either way). One
    /// transaction per tick; advancing the watermark last. A read/store fault is
    /// logged and skipped (the next tick retries — fail-safe, never panics on the
    /// background task). Returns the number of action rows inserted this tick.
    // A flat fold-and-join over the journal range — the length is the entry-kind
    // count, not cognitive complexity (the coordinator `core_loop` precedent).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn fold(&self, reader: &dyn JournalReader) -> usize {
        let head = match reader.current_seq() {
            Ok(h) => h,
            Err(error) => {
                tracing::warn!(%error, "capture fold: journal head read failed");
                return 0;
            }
        };
        let Ok(mut conn) = self.conn.lock() else {
            return 0;
        };
        let from = u64::try_from(Self::watermark(&conn)).unwrap_or(0);
        if head <= from {
            return 0; // nothing new
        }
        let entries = match reader.read_entries_by_seq(from + 1..head.saturating_add(1)) {
            Ok(e) => e,
            Err(error) => {
                tracing::warn!(%error, "capture fold: range read failed");
                return 0;
            }
        };
        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(error) => {
                tracing::warn!(%error, "capture fold: begin txn failed");
                return 0;
            }
        };
        let mut inserted = 0usize;
        // The serve session's instance id (single-node: one RunRegistered per
        // journal). Seeded from the DURABLE run_meta row — so an action committed
        // in a later tick is stamped even though RunRegistered (seq=1) folded in
        // an earlier tick (the bug the installed-runtime sweep caught: relying on
        // a same-tick registration left every later action's instance empty).
        let mut instance: Option<Vec<u8>> = Self::run_instance(&tx).ok().flatten();
        // The react facts seen THIS tick (turn_mote_id → highest-seq turn/branch),
        // applied to both new and already-captured rows after the inserts.
        let mut react_updates: HashMap<[u8; 32], (i64, String)> = HashMap::new();
        for entry in entries {
            match entry {
                JournalEntry::RunRegistered { instance_id, .. } => {
                    instance = Some(instance_id.to_vec());
                    // Persist it DURABLY so later ticks stamp without re-seeing it.
                    let _ = tx.execute(
                        "INSERT OR REPLACE INTO run_meta(id, instance_id) VALUES (0, ?1)",
                        params![instance_id.to_vec()],
                    );
                }
                JournalEntry::Committed {
                    mote_id,
                    nondeterminism,
                    result_ref,
                    seq,
                    ..
                } => {
                    // The crate's ActionsOnly scope discipline on the path: the
                    // record carries ONLY the action join key (no opt-in field),
                    // re-stripped under the serve's actions_only consent.
                    debug_assert_eq!(self.consent.scope, kx_capture::CaptureScope::ActionsOnly);
                    let record = StepRecord::action(mote_id, result_ref).actions_only();
                    let action_ref = record.output_ref.unwrap_or(result_ref);
                    if tx
                        .execute(
                            "INSERT OR REPLACE INTO capture_records \
                             (mote_id, instance_id, result_ref, nd_class, seq) \
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![
                                record.mote_id.as_bytes().to_vec(),
                                instance.clone().unwrap_or_default(),
                                action_ref.as_bytes().to_vec(),
                                nd_class_wire(nondeterminism),
                                i64::try_from(seq).unwrap_or(i64::MAX),
                            ],
                        )
                        .is_ok()
                    {
                        inserted += 1;
                    }
                }
                JournalEntry::ReactRound {
                    turn,
                    turn_mote_id,
                    branch,
                    ..
                } => {
                    // The latest fact for a turn wins (anchor Pending → settled);
                    // entries fold in ascending seq, so a later overwrite is the
                    // settled branch.
                    react_updates.insert(
                        *turn_mote_id.as_bytes(),
                        (i64::from(turn), branch_wire(&branch).to_string()),
                    );
                }
                _ => {}
            }
        }
        // Stamp every still-unstamped row with the session instance id (the
        // registration is seq=1, usually folded first, but be order-robust).
        if let Some(inst) = &instance {
            let _ = tx.execute(
                "UPDATE capture_records SET instance_id = ?1 WHERE length(instance_id) <> 16",
                params![inst],
            );
        }
        // Apply the react join (this tick's facts) to matching action rows. The
        // turn commits as its own Committed Mote with the same id, so the join is
        // mote_id == turn_mote_id — robust to either commit/settle order.
        for (turn_mote_id, (turn, branch)) in &react_updates {
            let _ = tx.execute(
                "UPDATE capture_records SET react_turn = ?2, react_branch = ?3 WHERE mote_id = ?1",
                params![turn_mote_id.to_vec(), turn, branch],
            );
            let _ = tx.execute(
                "INSERT OR REPLACE INTO react_turns(turn_mote_id, turn, branch, seq) \
                 VALUES (?1, ?2, ?3, 0)",
                params![turn_mote_id.to_vec(), turn, branch],
            );
        }
        let _ = tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('last_seq', ?1)",
            params![i64::try_from(head).unwrap_or(i64::MAX)],
        );
        if let Err(error) = tx.commit() {
            tracing::warn!(%error, "capture fold: commit failed (retried next tick)");
            return 0;
        }
        inserted
    }

    /// The serve session's instance id from the DURABLE `run_meta` row (set when
    /// `RunRegistered` first folds), or `None` before any run registered.
    fn run_instance(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<Option<Vec<u8>>> {
        tx.query_row("SELECT instance_id FROM run_meta WHERE id = 0", [], |r| {
            r.get(0)
        })
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }
}

impl CaptureView for CaptureLedger {
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
    ) -> Result<(Vec<CaptureRecordEntry>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("capture lock poisoned".into()))?;
        // Over-fetch by one to compute `has_more` without a second COUNT query.
        let over = limit.saturating_add(1);
        let mut rows: Vec<CaptureRecordEntry> = match instance_id {
            Some(inst) => Self::select(
                &conn,
                "SELECT mote_id, instance_id, result_ref, nd_class, seq, react_turn, react_branch \
                 FROM capture_records WHERE instance_id = ?1 ORDER BY seq DESC LIMIT ?2",
                params![inst.to_vec(), i64::try_from(over).unwrap_or(i64::MAX)],
            ),
            None => Self::select(
                &conn,
                "SELECT mote_id, instance_id, result_ref, nd_class, seq, react_turn, react_branch \
                 FROM capture_records ORDER BY seq DESC LIMIT ?1",
                params![i64::try_from(over).unwrap_or(i64::MAX)],
            ),
        }?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok((rows, has_more))
    }
}

impl CaptureLedger {
    fn select(
        conn: &Connection,
        sql: &str,
        p: impl rusqlite::Params,
    ) -> Result<Vec<CaptureRecordEntry>, CoreError> {
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| CoreError::Internal(format!("capture query prep: {e}")))?;
        let rows = stmt
            .query_map(p, |r| {
                let mote: Vec<u8> = r.get(0)?;
                let inst: Vec<u8> = r.get(1)?;
                let result: Vec<u8> = r.get(2)?;
                let nd_class: String = r.get(3)?;
                let seq: i64 = r.get(4)?;
                let react_turn: Option<i64> = r.get(5)?;
                let react_branch: String = r.get(6)?;
                Ok((mote, inst, result, nd_class, seq, react_turn, react_branch))
            })
            .map_err(|e| CoreError::Internal(format!("capture query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let (mote, inst, result, nd_class, seq, react_turn, react_branch) =
                row.map_err(|e| CoreError::Internal(format!("capture row: {e}")))?;
            out.push(CaptureRecordEntry {
                mote_id: <[u8; 32]>::try_from(mote.as_slice()).unwrap_or([0; 32]),
                instance_id: <[u8; 16]>::try_from(inst.as_slice()).unwrap_or([0; 16]),
                result_ref: <[u8; 32]>::try_from(result.as_slice()).unwrap_or([0; 32]),
                nd_class,
                seq: u64::try_from(seq).unwrap_or(0),
                react_turn: react_turn.map(|t| u32::try_from(t).unwrap_or(0)),
                react_branch,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use kx_content::ContentRef;
    use kx_gateway_core::ReadOnly;
    use kx_journal::{InMemoryJournal, Journal, JournalEntry};
    use kx_mote::{MoteDefHash, MoteId};
    use smallvec::SmallVec;

    use super::*;

    fn committed(seq: u64, mote: u8, result: u8) -> JournalEntry {
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes([mote; 32]),
            idempotency_key: [mote; 32],
            seq,
            nondeterminism: NdClass::Pure,
            result_ref: ContentRef::from_bytes([result; 32]),
            parents: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
        }
    }

    fn react_round(seq: u64, turn: u32, turn_mote: u8, branch: ReactBranch) -> JournalEntry {
        JournalEntry::ReactRound {
            turn,
            turn_mote_id: MoteId::from_bytes([turn_mote; 32]),
            instance_id: [5; 16],
            base_prompt_ref: ContentRef::from_bytes([0xf1; 32]),
            warrant_ref: ContentRef::from_bytes([0xf2; 32]),
            model_id: "m".into(),
            branch,
            max_turns: 8,
            max_tool_calls: 6,
            seq,
        }
    }

    /// Build an in-memory journal from a registration + the given entries and
    /// return a read-only handle the ledger can fold.
    fn journal_with(entries: Vec<JournalEntry>) -> ReadOnly<InMemoryJournal> {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [5; 16],
            recipe_fingerprint: [6; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
        for e in entries {
            j.append(e).unwrap();
        }
        ReadOnly::new(j)
    }

    #[test]
    fn fold_is_idempotent_and_watermarked() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = CaptureLedger::open(dir.path()).unwrap();
        let reader = journal_with(vec![committed(2, 0x10, 0x20), committed(3, 0x11, 0x21)]);

        assert_eq!(ledger.fold(&reader), 2, "first fold captures both actions");
        assert_eq!(ledger.fold(&reader), 0, "re-fold is a no-op (watermark)");

        let (records, has_more) = ledger.list(10, None).unwrap();
        assert_eq!(records.len(), 2);
        assert!(!has_more);
        // Stamped with the run instance; newest-first by seq.
        assert!(records.iter().all(|r| r.instance_id == [5; 16]));
        assert!(records[0].seq > records[1].seq);
    }

    /// A test-only [`JournalReader`] whose contents GROW between reads — modeling
    /// a live journal that the background poller folds incrementally across ticks.
    /// (We cannot reuse `ReadOnly<InMemoryJournal>`: the seam deliberately exposes
    /// no write surface, and `InMemoryJournal` is not `Clone`, so there is no way
    /// to append to the folded handle between ticks. This reader stays a pure test
    /// fixture — no production seam is weakened for the test's sake.)
    struct GrowableReader {
        entries: std::sync::RwLock<Vec<JournalEntry>>,
    }

    impl GrowableReader {
        fn new() -> Self {
            Self {
                entries: std::sync::RwLock::new(Vec::new()),
            }
        }

        fn push(&self, entry: JournalEntry) {
            self.entries.write().unwrap().push(entry);
        }
    }

    impl JournalReader for GrowableReader {
        fn read_entries_by_seq(
            &self,
            range: std::ops::Range<u64>,
        ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, kx_journal::JournalError> {
            let hit: Vec<JournalEntry> = self
                .entries
                .read()
                .unwrap()
                .iter()
                .filter(|e| range.contains(&e.seq()))
                .cloned()
                .collect();
            Ok(Box::new(hit.into_iter()))
        }

        fn current_seq(&self) -> Result<u64, kx_journal::JournalError> {
            Ok(self
                .entries
                .read()
                .unwrap()
                .iter()
                .map(JournalEntry::seq)
                .max()
                .unwrap_or(0))
        }
    }

    #[test]
    fn instance_is_stamped_across_separate_fold_ticks() {
        // The installed-runtime-sweep regression (F9): in a real serve the
        // RunRegistered fact (seq=1) folds in an EARLY tick, before any action
        // commits. A later tick that folds a Committed action must STILL stamp
        // it with the run instance — via the durable run_meta row, not a
        // same-tick registration.
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = CaptureLedger::open(dir.path()).unwrap();
        let reader = GrowableReader::new();
        // Tick 1: only the registration is in the journal.
        reader.push(JournalEntry::RunRegistered {
            instance_id: [7; 16],
            recipe_fingerprint: [6; 32],
            ts: 0,
            seq: 1,
        });
        assert_eq!(
            ledger.fold(&reader),
            0,
            "registration tick captures no action"
        );
        // Tick 2: an action commits LATER (the registration is already past the
        // watermark, so the run instance comes only from the durable run_meta row).
        reader.push(committed(2, 0x10, 0x20));
        assert_eq!(ledger.fold(&reader), 1);

        let (records, _) = ledger.list(10, Some([7; 16])).unwrap();
        assert_eq!(
            records.len(),
            1,
            "the later action is stamped with the run instance from a PRIOR tick"
        );
        assert_eq!(records[0].instance_id, [7; 16]);
    }

    #[test]
    fn react_turn_branch_is_joined_onto_the_action() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = CaptureLedger::open(dir.path()).unwrap();
        // The turn Mote 0x28 commits (an action) AND has ReactRound facts
        // (anchor Pending then a settled Answer at a higher seq).
        let reader = journal_with(vec![
            react_round(2, 0, 0x28, ReactBranch::Pending),
            committed(3, 0x28, 0x30),
            react_round(4, 0, 0x28, ReactBranch::Answer),
        ]);
        assert_eq!(ledger.fold(&reader), 1);

        let (records, _) = ledger.list(10, None).unwrap();
        let turn = records
            .iter()
            .find(|r| r.mote_id == [0x28; 32])
            .expect("the react turn action was captured");
        assert_eq!(turn.react_turn, Some(0));
        assert_eq!(
            turn.react_branch, "answer",
            "the SETTLED branch (highest seq) joins, not the Pending anchor"
        );
    }

    #[test]
    fn a_schema_version_bump_drops_and_rebuilds() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let ledger = CaptureLedger::open(dir.path()).unwrap();
            let reader = journal_with(vec![committed(2, 0x10, 0x20)]);
            assert_eq!(ledger.fold(&reader), 1);
        }
        // Stamp a stale schema_version → reopen must drop-and-rebuild (empty,
        // watermark 0; a fresh fold backfills).
        {
            let conn = Connection::open(dir.path().join("capture.db")).unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', 999)",
                [],
            )
            .unwrap();
        }
        let ledger = CaptureLedger::open(dir.path()).unwrap();
        let (records, _) = ledger.list(10, None).unwrap();
        assert!(records.is_empty(), "stale schema dropped the projection");
        // Backfills cleanly on the next fold.
        let reader = journal_with(vec![committed(2, 0x10, 0x20)]);
        assert_eq!(ledger.fold(&reader), 1);
    }
}
